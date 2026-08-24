//! Incremental Server-Sent Events JSON framing (main spec §18.2, §5.6).
//!
//! Each event's `data:` payload is one independently decoded JSON item; the
//! decoder surfaces items as their dispatching blank line arrives and never
//! aggregates the body beyond a single event. The encoder writes one bounded
//! document per item as `data:<json>\n\n` (§18.2: no `event:`/`id:` fields in
//! server output).
//!
//! # Framing decisions (DECISIONS.md D-impl-sse-framing, §18.2)
//!
//! - Line terminators: CRLF, LF, and CR are all accepted between lines. A CR
//!   at a chunk edge stays ambiguous until the next byte arrives (or EOF
//!   confirms it as a bare-CR terminator).
//! - `field: value` parsing follows WHATWG: a single leading space after the
//!   colon is stripped, lines without a colon are whole-line field names with
//!   an empty value, and unrecognized fields are ignored.
//! - Comment lines (`:` prefix) are ignored.
//! - Multi-line `data:` fields are joined with `\n` before exactly ONE JSON
//!   parse per dispatched event (the bare item — default mode).
//! - `id:`/`event:` are captured nowhere; `retry:` lines are accepted and
//!   discarded (surfacing retry through generator configuration is upstream
//!   of this runtime; automatic reconnection is out of scope per §18.2).
//! - An event without `data:` lines is skipped per WHATWG.
//! - UTF-8 BOM is stripped once at stream start (WHATWG), including when the
//!   BOM itself spans chunk boundaries.
//! - Per-event bound `limit` (`max_stream_record_bytes`, §33) counts the
//!   joined `data:` document: exceeding it before the blank line fails with
//!   [`SseDecodeError::RecordTooLarge`] without polling the producer again.
//!   As a memory-bounding measure the same limit caps ANY single buffered
//!   line — even ignorable comment/metadata lines — so the carry buffer can
//!   never grow past `limit` regardless of event shape (real-world SSE
//!   metadata lines are tiny).
//! - Malformed JSON ([`SseDecodeError::MalformedJson`], serde error as
//!   source) TERMINATES the stream fail-fast, never skip-and-continue (§18.2
//!   "without collecting the rest"). Non-UTF-8 event data is
//!   [`SseDecodeError::NotUtf8`].
//! - EOF with a partially buffered line, or with `data:` accumulated but not
//!   yet dispatched, is [`SseDecodeError::Truncated`] — stricter than WHATWG
//!   (which would dispatch a trailing unterminated event), keeping §40
//!   truncation observable instead of guessed away. Clean EOF between events
//!   is a clean end.
//! - Transport failures surface as
//!   [`SseDecodeError::Source`](crate::stream_errors::SseDecodeError::Source),
//!   preserving the underlying error. Dropping the stream after any terminal
//!   item cancels the producer.
//!
//! # Memory bounds
//!
//! Two bounded accumulators: the carry buffer holds at most one partial line
//! (`limit` cap above, plus the largest inbound chunk momentarily during
//! append), and the joined-data accumulator never exceeds `limit` bytes by
//! construction. Nothing tracks total body size; backpressure is purely
//! poll-driven (pull-based, no internal channel).

use std::error::Error;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes, BytesMut};
use futures_core::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::encode::{serialize_json_framed, EncodeTooLarge};
use crate::stream_errors::SseDecodeError;

/// UTF-8 byte order mark stripped once at stream start (WHATWG).
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Decodes an SSE body into one JSON item per dispatched event (main spec
/// §18.2), bounding each event's joined data — and every single buffered
/// line — by `limit` bytes (`max_stream_record_bytes`).
///
/// Poll-driven over `chunks`: nothing is read ahead of consumer demand, and
/// the first framing/validation failure is yielded as the final item. See the
/// module docs for the exact framing contract and memory bounds.
pub fn decode_sse_json<T, S, E>(
    chunks: S,
    limit: usize,
) -> impl Stream<Item = Result<T, SseDecodeError>>
where
    T: DeserializeOwned,
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    SseStream {
        chunks: Box::pin(chunks),
        limit,
        line: BytesMut::new(),
        data: Vec::new(),
        has_data_lines: false,
        bom_pending: true,
        finished: false,
        item: PhantomData,
    }
}

/// Encodes one item as an SSE event: `data:<json>` followed by a blank line,
/// bounded by `limit` bytes (D-impl-stream-item-bounds: per-item encode uses
/// `max_stream_record_bytes`, not `structured_encode_bytes`).
///
/// Fail-fast via [`CountingWriter`](crate::encode::CountingWriter): overflow
/// yields [`EncodeTooLarge`] and no partial output escapes.
pub fn encode_sse_event<T>(item: &T, limit: usize) -> Result<Bytes, EncodeTooLarge>
where
    T: Serialize + ?Sized,
{
    serialize_json_framed(item, limit, b"data:", b"\n\n")
}

enum Fill {
    Pending,
    Eof,
    Filled,
    Source(Box<dyn Error + Send + Sync>),
}

enum BomStep {
    /// Decision made (or more bytes buffered): resume the poll loop.
    Continue,
    /// Source has no decision-relevant bytes yet.
    Pending,
    Fail(SseDecodeError),
}

enum Step<T> {
    /// A dispatched event decoded into an item.
    Yield(T),
    /// Line consumed without producing anything.
    Continue,
}

struct SseStream<T, S> {
    chunks: Pin<Box<S>>,
    limit: usize,
    /// Carry buffer for the current partial line (module docs: bounds).
    line: BytesMut,
    /// Joined `data:` document for the event being assembled (≤ `limit`).
    data: Vec<u8>,
    has_data_lines: bool,
    bom_pending: bool,
    finished: bool,
    item: PhantomData<fn() -> T>,
}

impl<T, S, E> SseStream<T, S>
where
    T: DeserializeOwned,
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    fn fill(&mut self, cx: &mut Context<'_>) -> Fill {
        match self.chunks.as_mut().poll_next(cx) {
            Poll::Pending => Fill::Pending,
            Poll::Ready(None) => Fill::Eof,
            Poll::Ready(Some(Ok(chunk))) => {
                self.line.extend_from_slice(&chunk);
                Fill::Filled
            }
            Poll::Ready(Some(Err(error))) => Fill::Source(error.into()),
        }
    }

    fn fail(&mut self, error: SseDecodeError) -> Poll<Option<Result<T, SseDecodeError>>> {
        self.finished = true;
        Poll::Ready(Some(Err(error)))
    }

    /// Resolves the once-per-stream BOM decision before any line parsing.
    fn resolve_bom(&mut self, cx: &mut Context<'_>) -> BomStep {
        if self.line.starts_with(&BOM) {
            self.line.advance(BOM.len());
            self.bom_pending = false;
        } else if BOM.starts_with(&self.line) && self.line.len() < BOM.len() {
            // Could still grow into a BOM: wait for more bytes. At EOF the
            // short run is ordinary (doomed) content instead.
            match self.fill(cx) {
                Fill::Pending => return BomStep::Pending,
                Fill::Filled => {}
                Fill::Eof => self.bom_pending = false,
                Fill::Source(error) => return BomStep::Fail(SseDecodeError::Source(error)),
            }
        } else {
            self.bom_pending = false;
        }
        BomStep::Continue
    }

    /// Consumes one completed line: comments and metadata fields are ignored,
    /// `data:` joins into the event accumulator, and an empty line dispatches
    /// the event.
    fn handle_line(&mut self, content: &[u8]) -> Result<Step<T>, SseDecodeError> {
        if content.is_empty() {
            return self.dispatch();
        }
        if content[0] == b':' {
            return Ok(Step::Continue); // comment line
        }
        let (field, value) = split_field(content);
        match field {
            b"data" => {
                // WHATWG join: newline before every value after the first
                // data line, even when a value itself is empty.
                let joined_len = self.data.len() + usize::from(self.has_data_lines) + value.len();
                if joined_len > self.limit {
                    return Err(SseDecodeError::RecordTooLarge { limit: self.limit });
                }
                if self.has_data_lines {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value);
                self.has_data_lines = true;
                Ok(Step::Continue)
            }
            // `id`, `event`, `retry`, and unknown names: captured nowhere.
            _ => Ok(Step::Continue),
        }
    }

    /// Blank line: parse the joined data as ONE JSON item, or skip events
    /// that never saw a `data:` line (WHATWG dispatch rules).
    fn dispatch(&mut self) -> Result<Step<T>, SseDecodeError> {
        if !self.has_data_lines {
            return Ok(Step::Continue);
        }
        let text = std::str::from_utf8(&self.data).map_err(|_| SseDecodeError::NotUtf8)?;
        let parsed: T = serde_json::from_str(text).map_err(SseDecodeError::MalformedJson)?;
        self.data.clear();
        self.has_data_lines = false;
        Ok(Step::Yield(parsed))
    }

    /// True when EOF cannot be a clean end: a line is half-buffered, or an
    /// event is open without its dispatching blank line (§40 observability).
    fn eof_is_truncation(&self) -> bool {
        !self.line.is_empty() || self.has_data_lines
    }

    fn finish_eof(&mut self) -> Poll<Option<Result<T, SseDecodeError>>> {
        let truncated = self.eof_is_truncation();
        self.finished = true;
        if truncated {
            Poll::Ready(Some(Err(SseDecodeError::Truncated)))
        } else {
            Poll::Ready(None)
        }
    }
}

/// Splits `field: value` per WHATWG: everything before the first colon is the
/// field name; a single leading space is stripped from the value. Lines
/// without a colon carry an empty value.
fn split_field(content: &[u8]) -> (&[u8], &[u8]) {
    match content.iter().position(|byte| *byte == b':') {
        None => (content, &[]),
        Some(colon) => {
            let mut value = &content[colon + 1..];
            if value.first() == Some(&b' ') {
                value = &value[1..];
            }
            (&content[..colon], value)
        }
    }
}

/// Finds the next line terminator, returning `(content_end, terminator_len)`.
/// CRLF counts as one terminator; a CR at the very end stays unresolved until
/// another byte (or EOF) decides between CR and CRLF.
fn find_line_end(haystack: &[u8]) -> Option<(usize, usize)> {
    for (index, &byte) in haystack.iter().enumerate() {
        match byte {
            b'\n' => return Some((index, 1)),
            b'\r' => {
                return match haystack.get(index + 1) {
                    Some(b'\n') => Some((index, 2)),
                    Some(_) => Some((index, 1)),
                    None => None, // wait: CR could be half of a CRLF
                };
            }
            _ => {}
        }
    }
    None
}

impl<T, S, E> Stream for SseStream<T, S>
where
    T: DeserializeOwned,
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    type Item = Result<T, SseDecodeError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.finished {
                return Poll::Ready(None);
            }
            if this.bom_pending {
                match this.resolve_bom(cx) {
                    BomStep::Continue => {}
                    BomStep::Pending => return Poll::Pending,
                    BomStep::Fail(error) => return this.fail(error),
                }
                continue;
            }
            match find_line_end(&this.line) {
                Some((content_end, term_len)) => {
                    // Whole-line cap (module docs): a fully buffered line —
                    // even an ignorable comment or metadata field — may never
                    // exceed the limit, keeping the carry buffer bounded.
                    if content_end > this.limit {
                        let limit = this.limit;
                        return this.fail(SseDecodeError::RecordTooLarge { limit });
                    }
                    let mut completed = this.line.split_to(content_end + term_len);
                    completed.truncate(content_end);
                    let content = completed.freeze();
                    match this.handle_line(&content) {
                        Ok(Step::Continue) => continue,
                        Ok(Step::Yield(item)) => return Poll::Ready(Some(Ok(item))),
                        Err(error) => return this.fail(error),
                    }
                }
                None => {
                    // No terminator: the buffer is one partial line, so it can
                    // no longer shrink back under the limit (module docs).
                    if this.line.len() > this.limit {
                        let limit = this.limit;
                        return this.fail(SseDecodeError::RecordTooLarge { limit });
                    }
                    match this.fill(cx) {
                        Fill::Pending => return Poll::Pending,
                        Fill::Eof => {
                            // A trailing CR acts as a terminator at EOF; the
                            // resulting line is processed on the next pass,
                            // where the truncation verdict is reached.
                            if this.line.ends_with(b"\r") {
                                let mut completed = this.line.split();
                                completed.truncate(completed.len() - 1);
                                let content = completed.freeze();
                                match this.handle_line(&content) {
                                    Ok(Step::Continue) => continue,
                                    Ok(Step::Yield(item)) => {
                                        return Poll::Ready(Some(Ok(item)));
                                    }
                                    Err(error) => return this.fail(error),
                                }
                            }
                            return this.finish_eof();
                        }
                        Fill::Filled => {}
                        Fill::Source(error) => {
                            return this.fail(SseDecodeError::Source(error));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Widget {
        name: String,
        count: u32,
    }

    type Chunk = Result<Bytes, std::convert::Infallible>;

    fn chunks(parts: Vec<&'static str>) -> impl Stream<Item = Chunk> {
        futures_util::stream::iter(
            parts
                .into_iter()
                .map(|part| Ok(Bytes::from_static(part.as_bytes()))),
        )
    }

    fn byte_chunks(parts: Vec<Vec<u8>>) -> impl Stream<Item = Chunk> {
        futures_util::stream::iter(parts.into_iter().map(|part| Ok(Bytes::from(part))))
    }

    async fn collect_items<T, S>(stream: S) -> Result<Vec<T>, SseDecodeError>
    where
        T: DeserializeOwned,
        S: Stream<Item = Result<T, SseDecodeError>>,
    {
        let mut items = Vec::new();
        let mut pinned = Box::pin(stream);
        loop {
            match pinned.as_mut().next().await {
                None => return Ok(items),
                Some(Ok(item)) => items.push(item),
                Some(Err(error)) => return Err(error),
            }
        }
    }

    fn widget(name: &str, count: u32) -> Widget {
        Widget {
            name: name.to_owned(),
            count,
        }
    }

    /// Exercises comments, metadata fields (id/event/retry/unknown), a plain
    /// event, multi-line data joined with `\n`, and a skipped no-data event.
    fn canonical() -> String {
        [
            ": keep-alive comment",
            "",
            "id: 42",
            "event: tick",
            "retry: 25000",
            "x-unknown: noise",
            "data: {\"name\":\"a\",\"count\":1}",
            "",
            "event: skipped-because-no-data",
            "",
            "data: {\"name\":\"b\",",
            "data: \"count\":2}",
            "",
            "",
        ]
        .join("\n")
    }

    fn canonical_expected() -> Vec<Widget> {
        vec![widget("a", 1), widget("b", 2)]
    }

    #[tokio::test]
    async fn canonical_body_decodes_comments_metadata_and_multiline_data() {
        let items = collect_items(decode_sse_json::<Widget, _, _>(
            byte_chunks(vec![canonical().into_bytes()]),
            256,
        ))
        .await
        .expect("well-framed body");
        assert_eq!(items, canonical_expected());
    }

    #[tokio::test]
    async fn empty_body_is_a_clean_empty_stream() {
        let items = collect_items(decode_sse_json::<Widget, _, _>(chunks(Vec::new()), 64)).await;
        assert!(items.expect("empty body").is_empty());
    }

    #[tokio::test]
    async fn bom_stripped_once_even_when_split_across_chunks() {
        let body = "\u{FEFF}data: {\"name\":\"a\",\"count\":1}\n\n";
        let items = collect_items(decode_sse_json::<Widget, _, _>(
            byte_chunks(
                body.as_bytes()
                    .to_vec()
                    .chunks(2)
                    .map(<[u8]>::to_vec)
                    .collect(),
            ),
            256,
        ))
        .await
        .expect("BOM stripped");
        assert_eq!(items, vec![widget("a", 1)]);
    }

    #[tokio::test]
    async fn crlf_cr_and_lf_terminators_are_all_accepted() {
        // CRLF line end + LF blank line; CR line end + bare-CR blank line at
        // EOF (the CR stays ambiguous until EOF confirms it).
        let body: &'static str =
            "data: {\"name\":\"a\",\"count\":1}\r\n\ndata: {\"name\":\"b\",\"count\":2}\r\r";
        let items = collect_items(decode_sse_json::<Widget, _, _>(chunks(vec![body]), 256))
            .await
            .expect("mixed terminators");
        assert_eq!(items, vec![widget("a", 1), widget("b", 2)]);
    }

    #[tokio::test]
    async fn field_value_space_stripping_follows_whatwg() {
        // Exactly ONE leading space is stripped; a colon-less line is an
        // unknown whole-line field and is ignored.
        let body: &'static str = "data:{\"name\":\"a\",\"count\":1}\n\ngarbage line\ndata:  {\"name\":\"b\",\"count\":2}\n\n";
        let items = collect_items(decode_sse_json::<Widget, _, _>(chunks(vec![body]), 256))
            .await
            .expect("parsed");
        assert_eq!(items, vec![widget("a", 1), widget("b", 2)]);
    }

    #[tokio::test]
    async fn malformed_json_terminates_stream_fail_fast() {
        struct PanickingStream {
            yielded: bool,
        }

        impl Stream for PanickingStream {
            type Item = Chunk;

            fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Chunk>> {
                if self.yielded {
                    panic!("stream polled after MalformedJson");
                }
                self.get_mut().yielded = true;
                Poll::Ready(Some(Ok(Bytes::from_static(b"data: {oops}\n\n"))))
            }
        }

        let mut stream = Box::pin(decode_sse_json::<Widget, _, _>(
            PanickingStream { yielded: false },
            256,
        ));
        let first = stream.as_mut().next().await.expect("error item");
        assert!(
            matches!(first, Err(SseDecodeError::MalformedJson(_))),
            "{first:?}"
        );
        assert!(
            stream.as_mut().next().await.is_none(),
            "terminal error yields None afterwards"
        );
    }

    #[tokio::test]
    async fn non_utf8_event_data_reports_not_utf8() {
        let raw: Vec<u8> = b"data: {\"name\":\"caf\xE9\",\"count\":1}\n\n".to_vec();
        let error = collect_items::<Widget, _>(decode_sse_json(byte_chunks(vec![raw]), 256)).await;
        assert!(
            matches!(error.expect_err("invalid UTF-8"), SseDecodeError::NotUtf8),
            "expected NotUtf8"
        );
    }

    #[tokio::test]
    async fn eof_mid_event_is_truncated_distinct_from_clean_eof() {
        // Data dispatched-ready but the blank line never arrives.
        let error = collect_items(decode_sse_json::<Widget, _, _>(
            chunks(vec!["data: {\"name\":\"a\",\"count\":1}\n"]),
            256,
        ))
        .await
        .expect_err("EOF mid-event");
        assert!(matches!(error, SseDecodeError::Truncated), "{error:?}");

        // Partially buffered line at EOF.
        let error = collect_items(decode_sse_json::<Widget, _, _>(
            chunks(vec!["data: {\"name\":\"a\""]),
            256,
        ))
        .await
        .expect_err("EOF mid-line");
        assert!(matches!(error, SseDecodeError::Truncated), "{error:?}");

        // Clean contrast: identical event WITH its dispatching blank line.
        let items = collect_items(decode_sse_json::<Widget, _, _>(
            chunks(vec!["data: {\"name\":\"a\",\"count\":1}\n\n"]),
            256,
        ))
        .await
        .expect("clean EOF");
        assert_eq!(items, vec![widget("a", 1)]);
    }

    /// Oversized joined data trips RecordTooLarge before the blank line and
    /// stops polling the producer entirely (test-19 family semantics).
    #[tokio::test]
    async fn oversize_event_rejects_before_blank_line_without_polling_again() {
        struct PanickingStream {
            polled: usize,
        }

        impl Stream for PanickingStream {
            type Item = Chunk;

            fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Chunk>> {
                let polled = &mut self.get_mut().polled;
                *polled += 1;
                match *polled {
                    1 => Poll::Ready(Some(Ok(Bytes::from_static(
                        b"data: {\"name\":\"tiny\"}\n\n",
                    )))),
                    2 => Poll::Ready(Some(Ok(Bytes::from_static(
                        b"data: {\"name\":\"aaaaaaaaaaaaaaaaaaaaaaaaaa\"",
                    )))),
                    _ => panic!("producer polled after RecordTooLarge rejection"),
                }
            }
        }

        let mut stream = Box::pin(decode_sse_json::<serde_json::Value, _, _>(
            PanickingStream { polled: 0 },
            32,
        ));
        let first = stream.as_mut().next().await.expect("first event fits");
        assert_eq!(
            first.expect("first event"),
            serde_json::json!({"name": "tiny"})
        );
        let second = stream.as_mut().next().await.expect("rejection item");
        assert!(
            matches!(
                second.expect_err("oversize"),
                SseDecodeError::RecordTooLarge { limit: 32 }
            ),
            "expected RecordTooLarge {{ limit: 32 }}"
        );
        assert!(stream.as_mut().next().await.is_none());
    }

    #[tokio::test]
    async fn single_line_over_limit_fails_even_for_ignorable_fields() {
        // Documented memory bound: ANY buffered line longer than the limit
        // fails, keeping the carry buffer bounded regardless of event shape.
        let padding = "z".repeat(80);
        let body = format!(": {padding}\n\ndata: {{}}\n\n").into_bytes();
        let error = collect_items(decode_sse_json::<serde_json::Value, _, _>(
            byte_chunks(vec![body]),
            64,
        ))
        .await
        .expect_err("comment line over limit");
        assert!(
            matches!(error, SseDecodeError::RecordTooLarge { limit: 64 }),
            "expected RecordTooLarge {{ limit: 64 }}, got {error:?}"
        );
    }

    #[tokio::test]
    async fn transport_failure_surfaces_as_source_not_truncation() {
        #[derive(Debug, thiserror::Error)]
        #[error("socket reset")]
        struct TransportDown;

        let frames: Vec<Result<Bytes, TransportDown>> =
            vec![Ok(Bytes::from_static(b"data: {}\n\n")), Err(TransportDown)];
        let error = collect_items(decode_sse_json::<serde_json::Value, _, _>(
            futures_util::stream::iter(frames),
            256,
        ))
        .await
        .expect_err("transport died mid-body");
        match error {
            SseDecodeError::Source(source) => {
                assert_eq!(source.to_string(), "socket reset");
            }
            other => panic!("expected Source, got {other:?}"),
        }
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

    /// §50 test 42 adapted to SSE: every single split point of the canonical
    /// body (plus 1–3-byte systematic re-chunkings) decodes identically to
    /// the unsplit run.
    #[tokio::test]
    async fn every_single_split_point_matches_unsplit_run() {
        let body = canonical().into_bytes();
        let baseline = collect_items(decode_sse_json::<Widget, _, _>(
            byte_chunks(vec![body.clone()]),
            4096,
        ))
        .await
        .expect("canonical parses");
        assert_eq!(baseline, canonical_expected());

        for step in 1..=3_usize {
            for offset in 1..body.len() {
                let mut parts = vec![body[..offset].to_vec()];
                parts.extend(
                    body[offset..]
                        .chunks(step)
                        .filter(|chunk| !chunk.is_empty())
                        .map(<[u8]>::to_vec),
                );
                let outcome =
                    collect_items(decode_sse_json::<Widget, _, _>(byte_chunks(parts), 4096))
                        .await
                        .unwrap_or_else(|error| {
                            panic!("step {step} offset {offset}: unexpected {error:?}")
                        });
                assert_eq!(outcome, baseline, "step {step} offset {offset}");
            }
        }
    }

    /// Exhaustive chunk compositions of a micro body (§50 test 42): every way
    /// to cut one tiny event into ≤3-byte chunks decodes like the unsplit
    /// run.
    #[tokio::test]
    async fn exhaustive_micro_compositions_match_unsplit_run() {
        let micro: &[u8] = b"data: {}\n\n";
        let baseline = collect_items(decode_sse_json::<serde_json::Value, _, _>(
            byte_chunks(vec![micro.to_vec()]),
            64,
        ))
        .await
        .expect("micro parses");
        assert_eq!(baseline, vec![serde_json::json!({})]);

        for sizes in compositions(micro.len(), 3) {
            let offsets: Vec<usize> = sizes
                .iter()
                .scan(0, |acc, size| {
                    *acc += size;
                    Some(*acc)
                })
                .take(sizes.len() - 1)
                .collect();
            let outcome = collect_items(decode_sse_json::<serde_json::Value, _, _>(
                byte_chunks(split_at_offsets(micro, &offsets)),
                64,
            ))
            .await
            .unwrap_or_else(|error| panic!("composition {sizes:?}: unexpected {error:?}"));
            assert_eq!(outcome, baseline, "composition {sizes:?}");
        }
    }

    #[tokio::test]
    async fn encode_writes_data_prefix_and_blank_line_and_round_trips() {
        let gear = widget("gear", 9);
        let bytes = encode_sse_event(&gear, 256).expect("under limit");
        assert_eq!(bytes.as_ref(), b"data:{\"name\":\"gear\",\"count\":9}\n\n");

        let decoded = collect_items(decode_sse_json::<Widget, _, _>(
            byte_chunks(vec![bytes.to_vec()]),
            256,
        ))
        .await
        .expect("clean");
        assert_eq!(decoded, vec![gear]);
    }

    #[test]
    fn encode_overflow_yields_encode_too_large_with_no_partial_output() {
        let long = widget(&"w".repeat(64), 0);
        let error = encode_sse_event(&long, 16).expect_err("over limit");
        assert_eq!(error, EncodeTooLarge { limit: 16 });

        // Exactly-at-limit frame: "data:" (5) + json + "\n\n" (2) counted
        // together.
        let exact = widget("abcd", 1);
        let encoded_len = serde_json::to_vec(&exact).expect("serialize").len() + 7;
        let ok = encode_sse_event(&exact, encoded_len).expect("exact fit");
        assert_eq!(ok.len(), encoded_len);
    }

    #[test]
    fn returned_stream_is_send() {
        fn assert_send<T: Send>(value: T) -> T {
            value
        }
        let stream = decode_sse_json::<Widget, _, _>(chunks(Vec::new()), 64);
        let _boxed: Pin<Box<dyn Stream<Item = Result<Widget, SseDecodeError>> + Send>> =
            Box::pin(assert_send(stream));
    }
}
