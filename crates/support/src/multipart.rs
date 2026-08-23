//! Incremental `multipart/form-data` framing engine (main spec §5.5, §17.1).
//!
//! §5.5 mandates incremental parsing: part payloads surface as
//! [`MultipartEvent::PartChunk`] frames as they arrive and are never
//! aggregated here. Cardinality enforcement (§17.1) runs while streaming:
//! [`MultipartLimits::max_parts`] is checked before a part's `PartBegin` is
//! yielded, [`MultipartLimits::max_header_bytes`] bounds each part's raw
//! header block (extension headers included) before any payload byte is
//! surfaced, and the name/file-name limits apply during header parsing.
//! Missing-required-field and duplicate-scalar detection live in generated
//! routers (§17.1); this layer only reports framing facts. Unknown fields are
//! simply additional parts downstream consumers may ignore (§17.1 default).
//!
//! # Framing decisions (RFC 2045 §5.1, RFC 2046 §5.1.1, RFC 7578)
//!
//! - Delimiters are strictly CRLF-framed: the scanner matches `\r\n--boundary`
//!   plus a `--boundary` at absolute body position 0 (RFC 2046 permits the
//!   first delimiter without the leading CRLF). LF-only delimiters are never
//!   tolerated: malformed input errors instead of being guessed at, matching
//!   the §28.1 philosophy. A bare `\n` where a delimiter's terminating CRLF
//!   belongs yields [`MultipartError::MalformedFraming`]; an LF-framed body
//!   that never matches any delimiter ends in [`MultipartError::Truncated`].
//! - Transport padding (LWSP) after `--boundary` and `--boundary--` is
//!   accepted per RFC 2046. Preamble before the first delimiter and epilogue
//!   after the closing delimiter are ignored and never buffered.
//! - Browser-style blank-line sharing: the CRLF that terminates a part's
//!   header block doubles as the next delimiter's leading CRLF whenever the
//!   payload is empty (`...\r\n\r\n--boundary--`). A non-empty payload never
//!   has that CRLF added to it.
//! - One deliberate leniency: after `--boundary--` plus optional padding, a
//!   clean EOF is a clean end because many producers omit the final CRLF.
//!   EOF anywhere else — including mid-CRLF after the closing dashes — is
//!   [`MultipartError::Truncated`].
//! - Boundary occurrences inside payload data that lack the preceding CRLF
//!   are payload bytes, not delimiters; only the run directly following the
//!   header blank line is treated as framing even without its own CRLF.
//! - [`MultipartEvent::PartEnd`] surfaces when the part's closing delimiter
//!   is confirmed — immediately before the next [`MultipartEvent::PartBegin`]
//!   or before the stream finishes.
//! - Source (transport) errors surface as [`MultipartError::Truncated`]:
//!   per main spec §40 an abnormally terminated stream must stay
//!   distinguishable from a clean end, and this enum has no transport
//!   variant; callers needing the underlying I/O error own the body stream.
//! - Nested `multipart/*` parts are NOT recursed into (§17.1 depth rule):
//!   their bytes stream through as opaque payload chunks.
//!   [`crate::limits::BodyLimits::max_multipart_depth`] remains reserved for
//!   a later codec plugin.
//!
//! # Memory bounds
//!
//! A single carry buffer serves every phase. Between source frames, outside a
//! header block, it retains at most `boundary.len() + 4` bytes (the longest
//! possible partial delimiter); while inside a header block it is bounded by
//! `max_header_bytes`, with an oversized single frame rejected before any
//! payload event. Each inbound frame is appended and scanned once (one copy
//! per byte), so peak buffering stays near the largest inbound frame plus the
//! carry bound; no allocation grows with total payload size (asserted by the
//! 4 MiB streaming test via an instrumented high-water mark). Backpressure is
//! purely poll-driven: nothing is read ahead of consumer demand and no
//! internal channel exists.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, BytesMut};
use futures_core::Stream;

use crate::mediatype::ParsedMediaType;

/// Multipart cardinality limits (main spec §17.1, mapped from §33 fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultipartLimits {
    /// Total parts per message; a further part is rejected before its
    /// `PartBegin` is surfaced (§17.1 `max_multipart_parts`).
    pub max_parts: usize,
    /// Raw bytes (CRLFs included) permitted per part header block, extension
    /// headers included; enforced before any payload byte is read
    /// (§17.1 `max_part_header_bytes`).
    pub max_header_bytes: usize,
    /// Maximum `name=` length in bytes, checked during header parsing
    /// (§17.1 `max_field_name_bytes`).
    pub max_field_name_bytes: usize,
    /// Maximum `filename=` length in bytes, checked during header parsing
    /// (§17.1 `max_file_name_bytes`).
    pub max_file_name_bytes: usize,
}

impl MultipartLimits {
    /// Maps the four §17.1/§33 cardinality fields from
    /// [`BodyLimits`](crate::limits::BodyLimits).
    #[must_use]
    pub fn from_body_limits(limits: &crate::limits::BodyLimits) -> Self {
        Self {
            max_parts: limits.max_multipart_parts,
            max_header_bytes: limits.max_part_header_bytes,
            max_field_name_bytes: limits.max_field_name_bytes,
            max_file_name_bytes: limits.max_file_name_bytes,
        }
    }
}

/// Parsed header facts of one part (RFC 7578 §4.2).
///
/// A part without a usable nonempty `name=` is a framing error, so
/// [`Self::name`] is always nonempty once parsing succeeds; an empty
/// `filename=""` value is preserved verbatim because browsers send it for
/// absent file selections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartPartHeaders {
    /// Decoded `name=` parameter (quoted-string escapes resolved).
    pub name: String,
    /// Optional decoded `filename=` parameter.
    pub filename: Option<String>,
    /// Optional per-part `Content-Type`.
    pub content_type: Option<mime::Mime>,
}

/// Incrementally produced framing events (main spec §5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipartEvent {
    /// A part begins; its headers are complete and validated.
    PartBegin(MultipartPartHeaders),
    /// Payload bytes of the current part, delivered as they arrive and
    /// never aggregated.
    PartChunk(bytes::Bytes),
    /// The current part ended cleanly.
    PartEnd,
}

/// Failure modes of the multipart framing engine (main spec §17.1, §40).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MultipartError {
    /// A part would begin after the configured maximum (§17.1).
    #[error("multipart body exceeds {limit} parts")]
    TooManyParts {
        /// The configured limit.
        limit: usize,
    },
    /// A part's header block exceeded its byte budget before the blank line
    /// arrived; no payload byte was surfaced (§17.1).
    #[error("part header block exceeds {limit} bytes")]
    PartHeaderTooLarge {
        /// The configured limit.
        limit: usize,
    },
    /// A `name=` value exceeded its byte limit during header parsing (§17.1).
    #[error("form field name exceeds {limit} bytes")]
    FieldNameTooLong {
        /// The configured limit.
        limit: usize,
    },
    /// A `filename=` value exceeded its byte limit during header parsing
    /// (§17.1).
    #[error("file name exceeds {limit} bytes")]
    FileNameTooLong {
        /// The configured limit.
        limit: usize,
    },
    /// The stream ended before the closing delimiter (§40 truncation
    /// philosophy applied to requests; transport failures map here too).
    #[error("multipart stream ended before the closing boundary")]
    Truncated,
    /// Structurally invalid framing or part headers.
    #[error("malformed multipart framing")]
    MalformedFraming,
}

/// Extracts the `boundary` parameter from a parsed `multipart/*` media type.
///
/// Both quoted (`boundary="abc"`) and token (`boundary=abc`) forms work;
/// parameter names were lowercased by
/// [`parse_content_type`](crate::mediatype::parse_content_type), values keep
/// their case because boundaries are case-sensitive. The first declaration
/// wins among duplicates. A missing or empty boundary — or one containing
/// CR/LF/NUL, which could never match a well-framed body — is
/// [`MultipartError::MalformedFraming`] (§28.1: never defaulted).
pub fn extract_boundary(ct: &ParsedMediaType) -> Result<String, MultipartError> {
    let value = ct
        .parameters
        .iter()
        .find(|(name, _)| name == "boundary")
        .map(|(_, value)| value)
        .ok_or(MultipartError::MalformedFraming)?;
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| matches!(byte, b'\r' | b'\n' | b'\0'))
    {
        return Err(MultipartError::MalformedFraming);
    }
    Ok(value.clone())
}

/// Frames an Axum request body into incremental [`MultipartEvent`]s
/// (main spec §5.5, §17 Output B streaming semantics).
///
/// The returned stream polls `body` directly (pure backpressure, no
/// buffering beyond the bounds documented in the module docs) and stops at
/// the first error item; dropping it cancels the producer.
pub fn stream_multipart(
    body: ::axum::body::BodyDataStream,
    boundary: String,
    limits: MultipartLimits,
) -> impl futures_core::Stream<Item = Result<MultipartEvent, MultipartError>> {
    build_stream(body, boundary, limits)
}

/// Builds the concrete stream so tests can observe internal buffering bounds.
fn build_stream(
    body: ::axum::body::BodyDataStream,
    boundary: String,
    limits: MultipartLimits,
) -> MultipartStream {
    // A boundary that can never form a valid delimiter is rejected on the
    // first poll rather than trusted to the caller (never panics on hostile
    // configuration).
    let rejected = boundary.is_empty()
        || boundary
            .bytes()
            .any(|byte| matches!(byte, b'\r' | b'\n' | b'\0'));
    let mut needle = Vec::with_capacity(boundary.len() + 4);
    needle.extend_from_slice(b"\r\n--");
    needle.extend_from_slice(boundary.as_bytes());
    let start_needle = needle[2..].to_vec();
    MultipartStream {
        body,
        needle,
        start_needle,
        buf: BytesMut::new(),
        pending_blank_crlf: false,
        dash: DashState::Undecided,
        state: State::Preamble,
        at_body_start: true,
        parts_opened: 0,
        limits,
        peak_buffered: 0,
        rejected: rejected.then_some(MultipartError::MalformedFraming),
    }
}

#[derive(Debug, Clone, Copy)]
enum State {
    /// Discarding the preamble while hunting the first delimiter.
    Preamble,
    /// Just past a `--boundary`; deciding open vs close via [`DashState`].
    DelimiterTail,
    /// Accumulating the current part's header block.
    Headers,
    /// Streaming the current part's payload toward the next delimiter.
    Payload,
    /// Closing delimiter confirmed; the final `PartEnd` is owed.
    EmitFinalEnd,
    /// Terminal: clean end or failure already reported.
    Finished,
}

/// Sub-state machine for the bytes between `--boundary` and the line break
/// (RFC 2046 §5.1.1: transport padding, then CRLF, or `--` for the close).
#[derive(Debug, Clone, Copy)]
enum DashState {
    /// Expecting `--`, LWSP, or CRLF.
    Undecided,
    /// Saw `-`: expecting the second dash of the closing delimiter.
    SecondDash,
    /// Closing dashes confirmed; skipping LWSP padding.
    ClosePad,
    /// Closing: saw the CR of its terminating CRLF.
    CloseCr,
    /// Opening delimiter confirmed; skipping LWSP padding.
    OpenPad,
    /// Opening: saw the CR of its terminating CRLF.
    OpenCr,
}

enum DashStep {
    Advance(DashState),
    Opened,
    ClosedClean,
    Malformed,
}

fn step_dash(state: DashState, byte: u8) -> DashStep {
    match state {
        DashState::Undecided => match byte {
            b'-' => DashStep::Advance(DashState::SecondDash),
            b' ' | b'\t' => DashStep::Advance(DashState::OpenPad),
            b'\r' => DashStep::Advance(DashState::OpenCr),
            _ => DashStep::Malformed,
        },
        DashState::SecondDash => match byte {
            b'-' => DashStep::Advance(DashState::ClosePad),
            _ => DashStep::Malformed,
        },
        DashState::ClosePad => match byte {
            b' ' | b'\t' => DashStep::Advance(DashState::ClosePad),
            b'\r' => DashStep::Advance(DashState::CloseCr),
            _ => DashStep::Malformed,
        },
        DashState::CloseCr => match byte {
            b'\n' => DashStep::ClosedClean,
            _ => DashStep::Malformed,
        },
        DashState::OpenPad => match byte {
            b' ' | b'\t' => DashStep::Advance(DashState::OpenPad),
            b'\r' => DashStep::Advance(DashState::OpenCr),
            _ => DashStep::Malformed,
        },
        DashState::OpenCr => match byte {
            b'\n' => DashStep::Opened,
            _ => DashStep::Malformed,
        },
    }
}

enum Flow {
    Continue,
    Pending,
    Event(MultipartEvent),
    Done,
    Fail(MultipartError),
}

struct MultipartStream {
    body: ::axum::body::BodyDataStream,
    needle: Vec<u8>,
    start_needle: Vec<u8>,
    /// Single carry buffer for every phase: partial delimiters, the active
    /// header block, and not-yet-emitted payload bytes.
    buf: BytesMut,
    /// The blank line's final CRLF is held back until the payload phase can
    /// tell an empty payload (delimiter shares it) from a non-empty one.
    pending_blank_crlf: bool,
    dash: DashState,
    state: State,
    at_body_start: bool,
    parts_opened: usize,
    limits: MultipartLimits,
    /// Test-visible high-water mark of `buf`, evidencing bounded buffering.
    peak_buffered: usize,
    rejected: Option<MultipartError>,
}

impl Stream for MultipartStream {
    type Item = Result<MultipartEvent, MultipartError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(error) = this.rejected.take() {
            this.state = State::Finished;
            return Poll::Ready(Some(Err(error)));
        }
        loop {
            let flow = match this.state {
                State::Preamble => this.poll_preamble(cx),
                State::DelimiterTail => this.poll_delimiter_tail(cx),
                State::Headers => this.poll_headers(cx),
                State::Payload => this.poll_payload(cx),
                State::EmitFinalEnd => Flow::Done,
                // A finished stream never polls the body again; per the
                // Stream contract callers stop after Ready(None), and after
                // an error item they observe None instead of more events.
                State::Finished => return Poll::Ready(None),
            };
            match flow {
                Flow::Continue => continue,
                Flow::Pending => return Poll::Pending,
                Flow::Event(event) => return Poll::Ready(Some(Ok(event))),
                Flow::Done => {
                    this.state = State::Finished;
                    return Poll::Ready(None);
                }
                Flow::Fail(error) => {
                    this.state = State::Finished;
                    return Poll::Ready(Some(Err(error)));
                }
            }
        }
    }
}

enum Fill {
    Pending,
    Eof,
    Filled,
}

/// Leftmost full occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

impl MultipartStream {
    /// Pulls one frame from the body into `buf` (module docs: the carry
    /// buffer). Transport failures map to [`Fill::Eof`] because the framing
    /// contract can no longer complete (§40 truncation).
    fn fill(
        cx: &mut Context<'_>,
        body: Pin<&mut ::axum::body::BodyDataStream>,
        buf: &mut BytesMut,
        peak: &mut usize,
    ) -> Fill {
        match body.poll_next(cx) {
            Poll::Pending => Fill::Pending,
            Poll::Ready(None) => Fill::Eof,
            Poll::Ready(Some(Err(_))) => Fill::Eof,
            Poll::Ready(Some(Ok(chunk))) => {
                buf.extend_from_slice(&chunk);
                if buf.len() > *peak {
                    *peak = buf.len();
                }
                Fill::Filled
            }
        }
    }

    fn fill_main(&mut self, cx: &mut Context<'_>) -> Fill {
        let body = Pin::new(&mut self.body);
        Self::fill(cx, body, &mut self.buf, &mut self.peak_buffered)
    }

    /// Length of the longest `buf` suffix that is a proper prefix of
    /// `needle`; retaining these bytes is sufficient for every partial
    /// delimiter spanning a chunk edge.
    fn partial_prefix_len(buf: &[u8], needle: &[u8]) -> usize {
        (1..needle.len())
            .rev()
            .find(|length| buf.ends_with(&needle[..*length]))
            .unwrap_or(0)
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        find(haystack, needle)
    }

    fn poll_preamble(&mut self, cx: &mut Context<'_>) -> Flow {
        loop {
            // RFC 2046: the very first delimiter may sit at position 0.
            if self.at_body_start && self.buf.starts_with(&self.start_needle) {
                self.buf.advance(self.start_needle.len());
                self.at_body_start = false;
                self.dash = DashState::Undecided;
                self.state = State::DelimiterTail;
                return Flow::Continue;
            }
            if let Some(index) = Self::find(&self.buf, &self.needle) {
                self.buf.advance(index + self.needle.len());
                self.at_body_start = false;
                self.dash = DashState::Undecided;
                self.state = State::DelimiterTail;
                return Flow::Continue;
            }
            // No delimiter yet: discard everything except bytes that could
            // still begin one. Two retention cases keep memory bounded:
            // (a) a tail that is a partial `\r\n--boundary` spanning the
            //     chunk edge;
            // (b) while NOTHING has been discarded yet, a buffer that is
            //     itself a prefix of `--boundary` — dropping it would lose
            //     the position-0 delimiter form RFC 2046 allows.
            let mut keep = Self::partial_prefix_len(&self.buf, &self.needle);
            if self.at_body_start && self.start_needle.starts_with(&self.buf) {
                keep = self.buf.len();
            }
            if keep < self.buf.len() {
                self.buf.advance(self.buf.len() - keep);
                self.at_body_start = false;
            }
            match self.fill_main(cx) {
                Fill::Pending => return Flow::Pending,
                Fill::Eof => return Flow::Fail(MultipartError::Truncated),
                Fill::Filled => {}
            }
        }
    }

    fn poll_delimiter_tail(&mut self, cx: &mut Context<'_>) -> Flow {
        loop {
            while !self.buf.is_empty() {
                let byte = self.buf.first().copied().expect("nonempty buffer");
                match step_dash(self.dash, byte) {
                    DashStep::Advance(next) => {
                        self.buf.advance(1);
                        self.dash = next;
                    }
                    DashStep::Opened => {
                        self.buf.advance(1);
                        return self.open_part();
                    }
                    // The epilogue after the closing CRLF is ignored (and
                    // unread; dropping cancels the producer).
                    DashStep::ClosedClean => return self.close_message(),
                    DashStep::Malformed => return Flow::Fail(MultipartError::MalformedFraming),
                }
            }
            match self.fill_main(cx) {
                Fill::Pending => return Flow::Pending,
                // Leniency: `--boundary--` (+padding) then EOF is a clean end.
                Fill::Eof if matches!(self.dash, DashState::ClosePad) => {
                    return self.close_message();
                }
                Fill::Eof => return Flow::Fail(MultipartError::Truncated),
                Fill::Filled => {}
            }
        }
    }

    /// Counts and opens a new part; §17.1 requires the part-count rejection
    /// to fire BEFORE the part's `PartBegin` is surfaced. Opening a
    /// delimiter also ends the previous part, so its `PartEnd` surfaces
    /// first.
    fn open_part(&mut self) -> Flow {
        self.dash = DashState::Undecided;
        self.parts_opened += 1;
        if self.parts_opened > self.limits.max_parts {
            return Flow::Fail(MultipartError::TooManyParts {
                limit: self.limits.max_parts,
            });
        }
        // Bytes already sitting in the carry buffer (same frame, after the
        // delimiter line) are header-block candidate bytes too; the header
        // phase measures them via `buf.len()` directly.
        self.state = State::Headers;
        if self.parts_opened == 1 {
            Flow::Continue
        } else {
            Flow::Event(MultipartEvent::PartEnd)
        }
    }

    /// Ends the message cleanly after the closing delimiter, surfacing the
    /// final part's `PartEnd` first (a zero-part body just finishes).
    fn close_message(&mut self) -> Flow {
        if self.parts_opened == 0 {
            Flow::Done
        } else {
            self.state = State::EmitFinalEnd;
            Flow::Event(MultipartEvent::PartEnd)
        }
    }

    fn poll_headers(&mut self, cx: &mut Context<'_>) -> Flow {
        loop {
            if let Some(end) = Self::find(&self.buf, b"\r\n\r\n") {
                // §17.1: the raw header block (both terminating CRLFs
                // included) is bounded even when the whole block already sits
                // in the carry buffer.
                if end + 4 > self.limits.max_header_bytes {
                    return Flow::Fail(MultipartError::PartHeaderTooLarge {
                        limit: self.limits.max_header_bytes,
                    });
                }
                let block = self.buf.split_to(end).freeze();
                // Consume the blank line's own CRLF but DEFER the final CRLF:
                // an empty payload shares it with the closing/opening
                // delimiter (`...\r\n\r\n--boundary--`), while a non-empty
                // payload lets the payload phase discard it.
                self.buf.advance(2);
                self.pending_blank_crlf = true;
                self.state = State::Payload;
                return match parse_part_headers(&block, &self.limits) {
                    Ok(headers) => Flow::Event(MultipartEvent::PartBegin(headers)),
                    Err(error) => Flow::Fail(error),
                };
            }
            // No terminator among the buffered candidate bytes; once they
            // reach the budget the block can only grow past it. Rejection
            // happens before any payload byte of this part surfaces (§17.1).
            if self.buf.len() >= self.limits.max_header_bytes {
                return Flow::Fail(MultipartError::PartHeaderTooLarge {
                    limit: self.limits.max_header_bytes,
                });
            }
            match self.fill_main(cx) {
                Fill::Pending => return Flow::Pending,
                Fill::Eof => return Flow::Fail(MultipartError::Truncated),
                Fill::Filled => {}
            }
        }
    }

    fn poll_payload(&mut self, cx: &mut Context<'_>) -> Flow {
        loop {
            // Resolve the blank line's deferred CRLF first: a delimiter at
            // offset 0 consumes it (empty payload); otherwise it was the
            // header block's terminating blank line and is discarded —
            // never emitted as payload.
            if self.pending_blank_crlf {
                if self.buf.starts_with(&self.needle) {
                    self.buf.advance(self.needle.len());
                    self.pending_blank_crlf = false;
                    self.dash = DashState::Undecided;
                    self.state = State::DelimiterTail;
                    return Flow::Continue;
                }
                let after_crlf = &self.buf[2..];
                let undecided = after_crlf.len() < self.start_needle.len()
                    && self.start_needle.starts_with(after_crlf);
                if undecided {
                    match self.fill_main(cx) {
                        Fill::Pending => return Flow::Pending,
                        Fill::Eof => return Flow::Fail(MultipartError::Truncated),
                        Fill::Filled => continue,
                    }
                }
                self.buf.advance(2);
                self.pending_blank_crlf = false;
            }
            if let Some(index) = Self::find(&self.buf, &self.needle) {
                let head = self.buf.split_to(index).freeze();
                self.buf.advance(self.needle.len());
                self.dash = DashState::Undecided;
                self.state = State::DelimiterTail;
                // Zero-length parts emit Begin/End with no chunks.
                if head.is_empty() {
                    return Flow::Continue;
                }
                return Flow::Event(MultipartEvent::PartChunk(head));
            }
            // Emit everything that cannot be part of a delimiter spanning the
            // edge; keep only the longest possible partial prefix.
            let keep = Self::partial_prefix_len(&self.buf, &self.needle);
            if keep < self.buf.len() {
                let head = self.buf.split_to(self.buf.len() - keep).freeze();
                return Flow::Event(MultipartEvent::PartChunk(head));
            }
            match self.fill_main(cx) {
                Fill::Pending => return Flow::Pending,
                Fill::Eof => return Flow::Fail(MultipartError::Truncated),
                Fill::Filled => {}
            }
        }
    }
}

/// Parses one part's header block (bytes before the blank line).
///
/// Strictness mirrors §28.1: duplicate recognized headers, header lines
/// without a colon, unparseable `Content-Type` values, or a missing/empty
/// `name=` are [`MultipartError::MalformedFraming`], never defaulted.
/// Extension headers are allowed and ignored (§17.1), still counted against
/// the caller's byte budget by the scanner.
fn parse_part_headers(
    block: &[u8],
    limits: &MultipartLimits,
) -> Result<MultipartPartHeaders, MultipartError> {
    let mut disposition: Option<&[u8]> = None;
    let mut content_type: Option<mime::Mime> = None;
    let mut rest = block;
    while !rest.is_empty() {
        let (line, remainder) = match find(rest, b"\r\n") {
            Some(index) => (&rest[..index], &rest[index + 2..]),
            None => (rest, &b""[..]),
        };
        rest = remainder;
        let colon = line
            .iter()
            .position(|byte| *byte == b':')
            .ok_or(MultipartError::MalformedFraming)?;
        let name = line[..colon].trim_ascii();
        let value = line[colon + 1..].trim_ascii();
        if name.eq_ignore_ascii_case(b"content-disposition") {
            if disposition.is_some() {
                return Err(MultipartError::MalformedFraming);
            }
            disposition = Some(value);
        } else if name.eq_ignore_ascii_case(b"content-type") {
            if content_type.is_some() {
                return Err(MultipartError::MalformedFraming);
            }
            let text = std::str::from_utf8(value).map_err(|_| MultipartError::MalformedFraming)?;
            let parsed: mime::Mime = text.parse().map_err(|_| MultipartError::MalformedFraming)?;
            content_type = Some(parsed);
        }
    }
    let value = disposition.ok_or(MultipartError::MalformedFraming)?;
    let (name, filename) = parse_disposition(value, limits)?;
    Ok(MultipartPartHeaders {
        name,
        filename,
        content_type,
    })
}

/// Parses the `Content-Disposition` parameter list, returning the decoded
/// `name=` and optional decoded `filename=`.
fn parse_disposition(
    value: &[u8],
    limits: &MultipartLimits,
) -> Result<(String, Option<String>), MultipartError> {
    let mut segments = split_quoted_params(value);
    let kind = segments
        .next()
        .map(<[u8]>::trim_ascii)
        .ok_or(MultipartError::MalformedFraming)?;
    if !kind.eq_ignore_ascii_case(b"form-data") {
        return Err(MultipartError::MalformedFraming);
    }
    let mut name: Option<Vec<u8>> = None;
    let mut filename: Option<Vec<u8>> = None;
    for segment in segments {
        let segment = segment.trim_ascii();
        let eq = segment
            .iter()
            .position(|byte| *byte == b'=')
            .ok_or(MultipartError::MalformedFraming)?;
        let pname = segment[..eq].trim_ascii();
        let pvalue = &segment[eq + 1..];
        if pname.eq_ignore_ascii_case(b"name") {
            if name.is_some() {
                return Err(MultipartError::MalformedFraming);
            }
            name = Some(decode_param_value(
                pvalue,
                limits.max_field_name_bytes,
                NameKind::Field,
            )?);
        } else if pname.eq_ignore_ascii_case(b"filename") {
            if filename.is_some() {
                return Err(MultipartError::MalformedFraming);
            }
            filename = Some(decode_param_value(
                pvalue,
                limits.max_file_name_bytes,
                NameKind::File,
            )?);
        }
    }
    let name = name.ok_or(MultipartError::MalformedFraming)?;
    if name.is_empty() {
        return Err(MultipartError::MalformedFraming);
    }
    let name = String::from_utf8(name).map_err(|_| MultipartError::MalformedFraming)?;
    let filename = filename
        .map(|bytes| String::from_utf8(bytes).map_err(|_| MultipartError::MalformedFraming))
        .transpose()?;
    Ok((name, filename))
}

/// Splits a parameter list on `;` bytes that sit outside quoted strings
/// (quoted-string escapes respected), so `name="a;b"` survives intact.
fn split_quoted_params(value: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut bounds = Vec::new();
    let mut start = 0usize;
    let mut in_quote = false;
    let mut escaped = false;
    for (index, &byte) in value.iter().enumerate() {
        if escaped {
            escaped = false;
        } else {
            match byte {
                b'\\' if in_quote => escaped = true,
                b'"' => in_quote = !in_quote,
                b';' if !in_quote => {
                    bounds.push(start..index);
                    start = index + 1;
                }
                _ => {}
            }
        }
    }
    bounds.push(start..value.len());
    bounds.into_iter().map(move |range| &value[range])
}

#[derive(Debug, Clone, Copy)]
enum NameKind {
    Field,
    File,
}

/// Decodes one `name=`/`filename=` value in quoted-string or token form,
/// checking the decoded byte length against `limit` while accumulating
/// (§17.1: checked during header parsing).
fn decode_param_value(raw: &[u8], limit: usize, kind: NameKind) -> Result<Vec<u8>, MultipartError> {
    let raw = raw.trim_ascii();
    let too_long = |limit| match kind {
        NameKind::Field => MultipartError::FieldNameTooLong { limit },
        NameKind::File => MultipartError::FileNameTooLong { limit },
    };
    let Some(quoted) = raw.strip_prefix(b"\"") else {
        // Token form: any trimmed byte run without the quotes.
        if raw.len() > limit {
            return Err(too_long(limit));
        }
        return Ok(raw.to_vec());
    };
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < quoted.len() {
        match quoted[index] {
            b'\\' => {
                index += 1;
                let Some(&byte) = quoted.get(index) else {
                    return Err(MultipartError::MalformedFraming);
                };
                push_capped(&mut out, byte, limit, &too_long)?;
                index += 1;
            }
            b'"' => {
                // After the closing quote only whitespace may remain.
                if !quoted[index + 1..].iter().all(u8::is_ascii_whitespace) {
                    return Err(MultipartError::MalformedFraming);
                }
                return Ok(out);
            }
            byte => {
                push_capped(&mut out, byte, limit, &too_long)?;
                index += 1;
            }
        }
    }
    Err(MultipartError::MalformedFraming)
}

fn push_capped(
    out: &mut Vec<u8>,
    byte: u8,
    limit: usize,
    too_long: &impl Fn(usize) -> MultipartError,
) -> Result<(), MultipartError> {
    if out.len() >= limit {
        return Err(too_long(limit));
    }
    out.push(byte);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mediatype::parse_content_type;
    use axum::body::{Body, BodyDataStream};
    use bytes::Bytes;
    use futures_util::StreamExt;

    const BOUNDARY: &str = "XyZzy123";

    type Frame = Result<Bytes, std::convert::Infallible>;

    fn body_from(parts: Vec<Vec<u8>>) -> BodyDataStream {
        let frames: Vec<Frame> = parts
            .into_iter()
            .map(|part| Ok(Bytes::from(part)))
            .collect();
        Body::from_stream(futures_util::stream::iter(frames)).into_data_stream()
    }

    fn limits() -> MultipartLimits {
        MultipartLimits {
            max_parts: 8,
            max_header_bytes: 512,
            max_field_name_bytes: 64,
            max_file_name_bytes: 128,
        }
    }

    async fn run(
        parts: Vec<Vec<u8>>,
        boundary: &str,
        limits: MultipartLimits,
    ) -> Result<Vec<MultipartEvent>, MultipartError> {
        let mut stream = Box::pin(stream_multipart(
            body_from(parts),
            boundary.to_owned(),
            limits,
        ));
        let mut events = Vec::new();
        loop {
            match stream.as_mut().next().await {
                None => return Ok(events),
                Some(Ok(event)) => events.push(event),
                Some(Err(error)) => return Err(error),
            }
        }
    }

    async fn run_whole(body: Vec<u8>) -> Result<Vec<MultipartEvent>, MultipartError> {
        run(vec![body], BOUNDARY, limits()).await
    }

    #[derive(Debug, PartialEq, Eq)]
    struct LogicalPart {
        name: String,
        filename: Option<String>,
        content_type: Option<String>,
        payload: Vec<u8>,
    }

    /// Folds raw events into logical parts so assertions hold regardless of
    /// how inbound frames were chunked.
    fn normalize(
        result: Result<Vec<MultipartEvent>, MultipartError>,
    ) -> Result<Vec<LogicalPart>, MultipartError> {
        result.map(|events| {
            let mut parts: Vec<LogicalPart> = Vec::new();
            for event in events {
                match event {
                    MultipartEvent::PartBegin(headers) => parts.push(LogicalPart {
                        name: headers.name,
                        filename: headers.filename,
                        content_type: headers.content_type.map(|mime| mime.to_string()),
                        payload: Vec::new(),
                    }),
                    MultipartEvent::PartChunk(chunk) => parts
                        .last_mut()
                        .expect("chunk inside an open part")
                        .payload
                        .extend_from_slice(&chunk),
                    MultipartEvent::PartEnd => {}
                }
            }
            parts
        })
    }

    fn scalar(name: &str, payload: &[u8]) -> LogicalPart {
        LogicalPart {
            name: name.to_owned(),
            filename: None,
            content_type: None,
            payload: payload.to_vec(),
        }
    }

    fn part_bytes(name: &str, extra_headers: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n{extra_headers}\r\n"
        )
        .into_bytes();
        out.extend_from_slice(payload);
        out.extend_from_slice(b"\r\n");
        out
    }

    fn close_bytes(padding: &str) -> Vec<u8> {
        format!("--{BOUNDARY}--{padding}\r\n").into_bytes()
    }

    /// Canonical two-part body whose file payload embeds boundary-shaped byte
    /// runs that are deliberately NOT delimiters (`\n--boundary`,
    /// `q--boundary`, `\r--boundary` — none preceded by CRLF).
    fn canonical_body() -> Vec<u8> {
        let mut body = part_bytes("field", "", b"value one");
        let file_payload: &[u8] = b"A\r\n-B\r\nq--XyZzy123z\n--XyZzy123\r--XyZzy123\xff\x00END";
        body.extend(part_bytes(
            "up",
            "Content-Type: application/octet-stream\r\nX-Extension: v\r\n",
            file_payload,
        ));
        body.extend(close_bytes(""));
        body
    }

    fn canonical_expected() -> Vec<LogicalPart> {
        vec![
            scalar("field", b"value one"),
            LogicalPart {
                name: "up".to_owned(),
                filename: None,
                content_type: Some("application/octet-stream".to_owned()),
                payload: b"A\r\n-B\r\nq--XyZzy123z\n--XyZzy123\r--XyZzy123\xff\x00END".to_vec(),
            },
        ]
    }

    #[tokio::test]
    async fn canonical_two_parts_round_trip_unsplit() {
        let parts = normalize(run_whole(canonical_body()).await).expect("clean body");
        assert_eq!(parts, canonical_expected());
    }

    #[tokio::test]
    async fn file_part_carries_filename_and_content_type() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"upload\"; filename=\"r\xc3\xa9 sum\xc3\xa9.bin\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(b"\x00\x01RIFX\r\n--not-a-delimiter\xff");
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

        let events = run_whole(body).await.expect("clean body");
        let [MultipartEvent::PartBegin(ref headers), ..] = events[..] else {
            panic!("expected a leading begin, got {events:?}");
        };
        assert_eq!(headers.name, "upload");
        assert_eq!(headers.filename.as_deref(), Some("ré sumé.bin"));
        assert_eq!(
            headers.content_type.as_ref().map(mime::Mime::to_string),
            Some("application/octet-stream".to_owned())
        );
        assert_eq!(events.last(), Some(&MultipartEvent::PartEnd));
        let payload = normalize(Ok(events)).expect("logical")[0].payload.clone();
        assert_eq!(payload, b"\x00\x01RIFX\r\n--not-a-delimiter\xff");
    }

    #[tokio::test]
    async fn simple_two_scalar_parts_round_trip() {
        let mut body = part_bytes("a", "", b"one");
        body.extend(part_bytes("b", "", b"two"));
        body.extend(close_bytes(""));
        let parts = normalize(run_whole(body).await).expect("clean body");
        assert_eq!(parts, vec![scalar("a", b"one"), scalar("b", b"two")]);
    }

    #[tokio::test]
    async fn preamble_and_epilogue_are_ignored_without_buffering() {
        let mut body = b"preamble prose\r\nmore preamble\r\n".to_vec();
        body.extend(part_bytes("a", "", b"one"));
        body.extend(close_bytes(""));
        body.extend_from_slice(b"epilogue junk --XyZzy123 trailing\r\nmore");
        let parts = normalize(run_whole(body).await).expect("clean body");
        assert_eq!(parts, vec![scalar("a", b"one")]);
    }

    #[tokio::test]
    async fn leading_crlf_before_first_delimiter_is_tolerated() {
        let mut body = b"\r\n".to_vec();
        body.extend(part_bytes("a", "", b"one"));
        body.extend(close_bytes(""));
        let parts = normalize(run_whole(body).await).expect("clean body");
        assert_eq!(parts, vec![scalar("a", b"one")]);
    }

    #[tokio::test]
    async fn transport_padding_tolerated_on_open_and_close_delimiters() {
        let mut body = format!("--{BOUNDARY}  \t\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n\r\n");
        body.extend_from_slice(b"one\r\n");
        body.extend(close_bytes(" \t"));
        let parts = normalize(run_whole(body).await).expect("clean body");
        assert_eq!(parts, vec![scalar("a", b"one")]);
    }

    #[tokio::test]
    async fn empty_part_emits_begin_end_without_chunks() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"empty\"\r\n\r\n");
        body.extend(close_bytes(""));
        let events = run_whole(body).await.expect("clean body");
        assert_eq!(
            events,
            vec![
                MultipartEvent::PartBegin(MultipartPartHeaders {
                    name: "empty".to_owned(),
                    filename: None,
                    content_type: None,
                }),
                MultipartEvent::PartEnd,
            ]
        );
    }

    #[tokio::test]
    async fn boundary_text_without_preceding_crlf_stays_in_payload() {
        // None of these runs is preceded by CRLF, so none is a delimiter.
        // (A run at payload offset 0 would share the blank line's CRLF and
        // legitimately read as the closing delimiter — covered by
        // `empty_part_emits_begin_end_without_chunks`.)
        let payload: &[u8] = b"x--XyZzy123lead\n--XyZzy123mid\r--XyZzy123tail";
        let mut body = part_bytes("a", "", payload);
        body.extend(close_bytes(""));
        let parts = normalize(run_whole(body).await).expect("clean body");
        assert_eq!(parts, vec![scalar("a", payload)]);
    }

    #[tokio::test]
    async fn crlf_prefixed_boundary_after_blank_line_is_framing_not_data() {
        // `\r\n--boundary` immediately after the blank line reads as the
        // part's closing delimiter even when the payload is empty; trailing
        // junk where padding/CRLF belongs is malformed framing.
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n\r\n--XyZzy123JUNK");
        let error = run_whole(body)
            .await
            .expect_err("junk after shared delimiter");
        assert_eq!(error, MultipartError::MalformedFraming);
    }

    #[tokio::test]
    async fn closing_delimiter_without_final_crlf_is_a_clean_end() {
        let mut body = part_bytes("a", "", b"one");
        body.extend_from_slice(format!("--{BOUNDARY}--").as_bytes());
        let parts = normalize(run_whole(body).await).expect("clean end without final CRLF");
        assert_eq!(parts, vec![scalar("a", b"one")]);
    }

    #[tokio::test]
    async fn lf_only_framing_is_never_tolerated() {
        // Strict CRLF: an LF-only body is never parsed. The position-0
        // delimiter matches (no leading CRLF required there), then the bare
        // LF where a real CRLF belongs is reported as malformed.
        let body: Vec<u8> = format!(
            "--{BOUNDARY}\nContent-Disposition: form-data; name=\"a\"\n\nv\n--{BOUNDARY}--\n"
        )
        .into_bytes();
        let error = run_whole(body).await.expect_err("LF-only must not parse");
        assert_eq!(error, MultipartError::MalformedFraming);

        // A body that never even matches a delimiter ends truncated instead
        // of guessed into a parse.
        let body: Vec<u8> =
            b"\nContent-Disposition: form-data; name=\"a\"\n\nv\n--XyZzy123--\n".to_vec();
        let error = run_whole(body).await.expect_err("LF-framed must not parse");
        assert_eq!(error, MultipartError::Truncated);
    }

    #[tokio::test]
    async fn bare_lf_where_crlf_belongs_is_malformed_framing() {
        let mut body = part_bytes("a", "", b"one");
        // `\r\n--boundary` then a bare LF instead of `--` or padding+CRLF.
        body.extend_from_slice(format!("--{BOUNDARY}\njunk").as_bytes());
        let error = run_whole(body).await.expect_err("bare LF after delimiter");
        assert_eq!(error, MultipartError::MalformedFraming);
    }

    #[tokio::test]
    async fn truncated_mid_payload_reports_truncated() {
        let mut body = part_bytes("field", "", b"value one");
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"up\"\r\n\r\npartial");
        let error = run_whole(body).await.expect_err("EOF mid-payload");
        assert_eq!(error, MultipartError::Truncated);
    }

    #[tokio::test]
    async fn missing_terminal_dashes_reports_truncated() {
        let mut body = part_bytes("a", "", b"one");
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let error = run_whole(body)
            .await
            .expect_err("body acts like more follows");
        assert_eq!(error, MultipartError::Truncated);

        // Ending exactly at `--boundary` (no dashes/CRLF yet) truncates too.
        let mut body = part_bytes("a", "", b"one");
        body.extend_from_slice(format!("--{BOUNDARY}").as_bytes());
        let error = run_whole(body).await.expect_err("EOF right after token");
        assert_eq!(error, MultipartError::Truncated);
    }

    #[tokio::test]
    async fn mid_crlf_after_close_dashes_reports_truncated() {
        let mut body = part_bytes("a", "", b"one");
        body.extend_from_slice(format!("--{BOUNDARY}--\r").as_bytes());
        let error = run_whole(body).await.expect_err("final CRLF cut in half");
        assert_eq!(error, MultipartError::Truncated);
    }

    #[tokio::test]
    async fn source_failure_surfaces_as_truncated() {
        // First frame completes one whole part (its closing delimiter
        // included); the transport then dies inside the NEXT part.
        let mut first_frame = part_bytes("a", "", b"one");
        first_frame.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        let frames: Vec<Result<Bytes, axum::Error>> = vec![
            Ok(Bytes::from(first_frame)),
            Err(axum::Error::new(std::io::Error::other("socket reset"))),
            Ok(Bytes::from(close_bytes(""))),
        ];
        let body = Body::from_stream(futures_util::stream::iter(frames)).into_data_stream();
        let mut stream = Box::pin(stream_multipart(body, BOUNDARY.to_owned(), limits()));
        let mut events = Vec::new();
        let error = loop {
            match stream.as_mut().next().await {
                None => panic!("expected an error item"),
                Some(Ok(event)) => events.push(event),
                Some(Err(error)) => break error,
            }
        };
        assert_eq!(error, MultipartError::Truncated);
        // The first part completed before the failure; nothing fabricated
        // afterwards.
        assert_eq!(events.last(), Some(&MultipartEvent::PartEnd));
    }

    #[tokio::test]
    async fn stream_polls_none_after_error_item() {
        let body: Vec<u8> = format!("--{BOUNDARY}\njunk").into_bytes();
        let mut stream = Box::pin(stream_multipart(
            body_from(vec![body]),
            BOUNDARY.to_owned(),
            limits(),
        ));
        let first = stream.as_mut().next().await;
        assert!(matches!(first, Some(Err(MultipartError::MalformedFraming))));
        assert!(
            stream.as_mut().next().await.is_none(),
            "terminal state yields None, never more events"
        );
    }

    #[test]
    fn returned_stream_is_send() {
        fn assert_send<T: Send>(value: T) -> T {
            value
        }
        let stream = stream_multipart(body_from(Vec::new()), BOUNDARY.to_owned(), limits());
        let _boxed: std::pin::Pin<
            Box<dyn futures_core::Stream<Item = Result<MultipartEvent, MultipartError>> + Send>,
        > = Box::pin(assert_send(stream));
    }

    #[tokio::test]
    async fn quoted_string_escapes_and_utf8_names_parse() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"a\\\"b;c\"; filename=\"caf\xc3\xa9.txt\"\r\n\r\n",
        );
        body.extend_from_slice(b"x\r\n");
        body.extend(close_bytes(""));
        let parts = normalize(run_whole(body).await).expect("clean body");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].name, "a\"b;c");
        assert_eq!(parts[0].filename.as_deref(), Some("café.txt"));
    }

    #[tokio::test]
    async fn missing_or_wrong_disposition_is_malformed() {
        // No Content-Disposition at all.
        let body: Vec<u8> =
            format!("--{BOUNDARY}\r\nX-Other: v\r\n\r\npayload\r\n--{BOUNDARY}--\r\n").into_bytes();
        assert_eq!(
            run_whole(body).await.unwrap_err(),
            MultipartError::MalformedFraming
        );

        // Wrong disposition type.
        let body: Vec<u8> = format!(
            "--{BOUNDARY}\r\nContent-Disposition: attachment; name=\"a\"\r\n\r\np\r\n--{BOUNDARY}--\r\n"
        )
        .into_bytes();
        assert_eq!(
            run_whole(body).await.unwrap_err(),
            MultipartError::MalformedFraming
        );
    }

    #[tokio::test]
    async fn malformed_part_content_type_is_never_defaulted() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n");
        body.extend_from_slice(b"Content-Type: /broken\r\n\r\nx\r\n");
        body.extend(close_bytes(""));
        assert_eq!(
            run_whole(body).await.unwrap_err(),
            MultipartError::MalformedFraming
        );
    }

    #[tokio::test]
    async fn duplicate_name_parameter_or_disposition_header_is_malformed() {
        // Duplicate `name=` parameters.
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"a\"; name=\"b\"\r\n\r\nx\r\n",
        );
        body.extend(close_bytes(""));
        assert_eq!(
            run_whole(body).await.unwrap_err(),
            MultipartError::MalformedFraming
        );

        // Two Content-Disposition headers.
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"b\"\r\n\r\nx\r\n");
        body.extend(close_bytes(""));
        assert_eq!(
            run_whole(body).await.unwrap_err(),
            MultipartError::MalformedFraming
        );
    }

    #[tokio::test]
    async fn header_line_without_colon_is_malformed() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n");
        body.extend_from_slice(b"garbage continuation line\r\n\r\nx\r\n");
        body.extend(close_bytes(""));
        assert_eq!(
            run_whole(body).await.unwrap_err(),
            MultipartError::MalformedFraming
        );
    }

    #[tokio::test]
    async fn empty_name_value_is_malformed_but_empty_filename_is_kept() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"\"\r\n\r\nx\r\n");
        body.extend(close_bytes(""));
        assert_eq!(
            run_whole(body).await.unwrap_err(),
            MultipartError::MalformedFraming
        );

        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"a\"; filename=\"\"\r\n\r\nx\r\n",
        );
        body.extend(close_bytes(""));
        let parts = normalize(run_whole(body).await).expect("empty filename is legal");
        assert_eq!(parts[0].filename.as_deref(), Some(""));
    }

    /// All compositions of `total` bytes into consecutive chunks of at most
    /// `max_size` bytes.
    fn compositions(total: usize, max_size: usize) -> Vec<Vec<usize>> {
        fn go(
            remaining: usize,
            max_size: usize,
            current: &mut Vec<usize>,
            out: &mut Vec<Vec<usize>>,
        ) {
            if remaining == 0 {
                out.push(current.clone());
                return;
            }
            for size in 1..=max_size.min(remaining) {
                current.push(size);
                go(remaining - size, max_size, current, out);
                current.pop();
            }
        }
        let mut out = Vec::new();
        go(total, max_size, &mut Vec::new(), &mut out);
        out
    }

    fn split_at_offsets(body: &[u8], offsets: &[usize]) -> Vec<Vec<u8>> {
        let mut parts = Vec::new();
        let mut start = 0usize;
        for &offset in offsets {
            parts.push(body[start..offset].to_vec());
            start = offset;
        }
        parts.push(body[start..].to_vec());
        parts.retain(|part| !part.is_empty());
        parts
    }

    /// §50 test 42 adapted to multipart: decode a body split at EVERY chunk
    /// boundary alignment and compare against the unsplit run. Full-body
    /// exhaustion into 1–3-BYTE chunks is exponential (tribonacci), so this
    /// covers every alignment exhaustively on a minimal real multipart body
    /// plus systematic/random splits of the canonical body below.
    #[tokio::test]
    async fn exhaustive_compositions_of_micro_body_match_unsplit_run() {
        // Minimal real multipart body: delimiter, empty header block, close.
        let micro: Vec<u8> = b"--Z\r\n\r\n\r\n--Z--\r\n".to_vec();
        let baseline = normalize(run(vec![micro.clone()], "Z", limits()).await);
        assert_eq!(
            baseline,
            Err(MultipartError::MalformedFraming),
            "micro body has no Content-Disposition"
        );

        let all = compositions(micro.len(), 3);
        assert_eq!(all.len(), 10_609, "tribonacci(16) compositions into ≤3");
        for sizes in all {
            let offsets: Vec<usize> = sizes
                .iter()
                .scan(0, |acc, size| {
                    *acc += size;
                    Some(*acc)
                })
                .take(sizes.len() - 1)
                .collect();
            let parts = split_at_offsets(&micro, &offsets);
            let outcome = normalize(run(parts, "Z", limits()).await);
            assert_eq!(outcome, baseline, "composition {sizes:?}");
        }
    }

    #[tokio::test]
    async fn every_single_split_point_matches_unsplit_run() {
        let body = canonical_body();
        let baseline =
            normalize(run(vec![body.clone()], BOUNDARY, limits()).await).expect("canonical parses");
        assert_eq!(baseline, canonical_expected());

        for step in 1..=3_usize {
            for offset in 1..body.len() {
                let mut parts = vec![body[..offset].to_vec()];
                let rest = &body[offset..];
                parts.extend(
                    rest.chunks(step)
                        .filter(|chunk| !chunk.is_empty())
                        .map(<[u8]>::to_vec),
                );
                let outcome = normalize(run(parts, BOUNDARY, limits()).await)
                    .expect("split must not change the verdict");
                assert_eq!(outcome, baseline, "step {step} offset {offset}");
            }
        }
    }

    #[tokio::test]
    async fn every_pair_of_split_points_matches_unsplit_run() {
        let body = canonical_body();
        let baseline =
            normalize(run(vec![body.clone()], BOUNDARY, limits()).await).expect("canonical parses");

        for first in 1..body.len() {
            for second in first + 1..body.len() {
                let parts = vec![
                    body[..first].to_vec(),
                    body[first..second].to_vec(),
                    body[second..].to_vec(),
                ];
                let outcome = normalize(run(parts, BOUNDARY, limits()).await)
                    .expect("splits must not change the verdict");
                assert_eq!(outcome, baseline, "pair {first},{second}");
            }
        }
    }

    #[tokio::test]
    async fn seeded_random_one_to_three_byte_chunkings_match_unsplit_run() {
        let body = canonical_body();
        let baseline =
            normalize(run(vec![body.clone()], BOUNDARY, limits()).await).expect("canonical parses");

        let mut state = 0x5EED_1234_ABCD_0042_u64;
        let mut next_size = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            1 + ((state >> 33) % 3) as usize
        };

        for trial in 0..2_000_u32 {
            let mut offsets = Vec::new();
            let mut cursor = 0usize;
            while cursor < body.len() {
                cursor = (cursor + next_size()).min(body.len());
                if cursor < body.len() {
                    offsets.push(cursor);
                }
            }
            let parts = split_at_offsets(&body, &offsets);
            let outcome = normalize(run(parts, BOUNDARY, limits()).await)
                .unwrap_or_else(|error| panic!("trial {trial}: unexpected {error:?}"));
            assert_eq!(outcome, baseline, "trial {trial} offsets {offsets:?}");
        }
    }

    #[tokio::test]
    async fn part_count_limit_rejects_before_excess_part_begins() {
        let mut body = part_bytes("a", "", b"one");
        body.extend(part_bytes("b", "", b"two"));
        body.extend(part_bytes("c", "", b"three"));
        body.extend(close_bytes(""));

        let mut limited = limits();
        limited.max_parts = 2;
        let mut stream = Box::pin(stream_multipart(
            body_from(vec![body]),
            BOUNDARY.to_owned(),
            limited,
        ));
        let mut names = Vec::new();
        let error = loop {
            match stream.as_mut().next().await {
                None => panic!("expected TooManyParts"),
                Some(Ok(MultipartEvent::PartBegin(headers))) => names.push(headers.name),
                Some(Ok(_)) => {}
                Some(Err(error)) => break error,
            }
        };
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(error, MultipartError::TooManyParts { limit: 2 });
    }

    #[tokio::test]
    async fn oversized_header_block_rejects_with_zero_payload_events() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"a\"\r\n");
        let padding = "0123456789abcdef".repeat(8);
        body.extend_from_slice(format!("X-Pad: {padding}\r\n").as_bytes());
        body.extend_from_slice(b"\r\npayload should never surface\r\n");
        body.extend(close_bytes(""));

        let mut limited = limits();
        limited.max_header_bytes = 64;
        let mut stream = Box::pin(stream_multipart(
            body_from(vec![body]),
            BOUNDARY.to_owned(),
            limited,
        ));
        let mut events = Vec::new();
        let error = loop {
            match stream.as_mut().next().await {
                None => panic!("expected PartHeaderTooLarge"),
                Some(Ok(event)) => events.push(event),
                Some(Err(error)) => break error,
            }
        };
        assert_eq!(error, MultipartError::PartHeaderTooLarge { limit: 64 });
        assert!(
            events.is_empty(),
            "no Begin/Chunk/End may precede the rejection, got {events:?}"
        );
    }

    #[tokio::test]
    async fn header_block_exactly_at_budget_is_accepted_and_over_is_not() {
        let header_line = "Content-Disposition: form-data; name=\"a\"";
        let extension = "X-Rust-Multipart: yes";
        let exact_budget = header_line.len() + extension.len() + 2 + 2 + 2;

        let build = || {
            let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
            body.extend_from_slice(header_line.as_bytes());
            body.extend_from_slice(b"\r\n");
            body.extend_from_slice(extension.as_bytes());
            body.extend_from_slice(b"\r\n\r\n");
            body.extend_from_slice(b"value\r\n");
            body.extend(close_bytes(""));
            body
        };

        let mut at_limit = limits();
        at_limit.max_header_bytes = exact_budget;
        let parts = normalize(run(vec![build()], BOUNDARY, at_limit).await).expect("exact fits");
        assert_eq!(parts, vec![scalar("a", b"value")]);

        let mut over_limit = limits();
        over_limit.max_header_bytes = exact_budget - 1;
        assert_eq!(
            run(vec![build()], BOUNDARY, over_limit).await.unwrap_err(),
            MultipartError::PartHeaderTooLarge {
                limit: exact_budget - 1
            }
        );
    }

    #[tokio::test]
    async fn oversized_field_name_is_rejected_during_header_parsing() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"abcdef\"\r\n\r\nx\r\n");
        body.extend(close_bytes(""));

        let mut limited = limits();
        limited.max_field_name_bytes = 4;
        assert_eq!(
            run(vec![body], BOUNDARY, limited).await.unwrap_err(),
            MultipartError::FieldNameTooLong { limit: 4 }
        );
    }

    #[tokio::test]
    async fn oversized_file_name_is_rejected_during_header_parsing() {
        let mut body = format!("--{BOUNDARY}\r\n").into_bytes();
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"f\"; filename=\"abcdefghij\"\r\n\r\nx\r\n",
        );
        body.extend(close_bytes(""));

        let mut limited = limits();
        limited.max_file_name_bytes = 8;
        assert_eq!(
            run(vec![body], BOUNDARY, limited).await.unwrap_err(),
            MultipartError::FileNameTooLong { limit: 8 }
        );
    }

    #[test]
    fn extract_boundary_accepts_quoted_token_and_charset_noise() {
        let quoted = parse_content_type("multipart/form-data; charset=utf-8; boundary=\"Xy 123\"")
            .expect("parses");
        assert_eq!(
            extract_boundary(&quoted).expect("quoted boundary"),
            "Xy 123"
        );

        let token =
            parse_content_type("multipart/form-data; charset=utf-8; boundary=Xy123").expect("");
        assert_eq!(extract_boundary(&token).expect("token boundary"), "Xy123");

        let case = parse_content_type("multipart/form-data; BouNDary=abc").expect("");
        assert_eq!(
            extract_boundary(&case).expect("case-insensitive name"),
            "abc"
        );

        let missing = parse_content_type("multipart/form-data; charset=utf-8").expect("");
        assert_eq!(
            extract_boundary(&missing).unwrap_err(),
            MultipartError::MalformedFraming
        );

        let empty = parse_content_type("multipart/form-data; boundary=\"\"").expect("");
        assert_eq!(
            extract_boundary(&empty).unwrap_err(),
            MultipartError::MalformedFraming
        );

        let injection = parse_content_type("multipart/form-data; boundary=\"a\rb\"").expect("");
        assert_eq!(
            extract_boundary(&injection).unwrap_err(),
            MultipartError::MalformedFraming
        );
    }

    #[tokio::test]
    async fn four_mib_binary_part_streams_incrementally_with_bounded_internal_buffer() {
        const MIB: usize = 1024 * 1024;
        const FRAME: usize = 64 * 1024;

        // Deterministic binary pattern that never contains CR, so it cannot
        // contain a delimiter by accident.
        let payload: Vec<u8> = (0..4 * MIB)
            .map(|index| {
                let byte = ((index * 7 + 3) & 0xFF) as u8;
                if byte == b'\r' {
                    b':'
                } else {
                    byte
                }
            })
            .collect();

        let mut body = part_bytes(
            "blob",
            "Content-Type: application/octet-stream\r\n",
            &payload,
        );
        body.extend(close_bytes(""));

        let frame_count = body.len().div_ceil(FRAME);
        let frames: Vec<Vec<u8>> = body.chunks(FRAME).map(<[u8]>::to_vec).collect();
        let mut stream = Box::pin(build_stream(
            body_from(frames),
            BOUNDARY.to_owned(),
            limits(),
        ));

        let mut chunk_events = 0usize;
        let mut reassembled = Vec::new();
        let mut began = false;
        loop {
            match stream.as_mut().next().await {
                None => break,
                Some(Ok(MultipartEvent::PartBegin(_))) => began = true,
                Some(Ok(MultipartEvent::PartChunk(chunk))) => {
                    chunk_events += 1;
                    reassembled.extend_from_slice(&chunk);
                }
                Some(Ok(MultipartEvent::PartEnd)) => {}
                Some(Err(error)) => panic!("unexpected {error:?}"),
            }
        }
        assert!(began);
        // Incremental delivery: many independent chunk events, not one blob.
        assert!(
            chunk_events * FRAME >= 2 * MIB && chunk_events > 16,
            "expected incremental chunks, got {chunk_events}"
        );
        assert_eq!(reassembled, payload);

        // Structural bound: the carry buffer never exceeded the largest
        // inbound frame plus the documented delimiter slack (boundary + 4,
        // rounded up generously). No allocation tracks total payload size.
        let largest_frame = FRAME.min(body.len());
        assert!(
            stream.peak_buffered <= largest_frame + BOUNDARY.len() + 64,
            "peak buffered {} exceeded {}",
            stream.peak_buffered,
            largest_frame + BOUNDARY.len() + 64
        );
        assert_eq!(frame_count, body.len().div_ceil(FRAME));
    }
}
