//! Incremental JSON Text Sequence framing (RFC 7464; main spec §20).
//!
//! Each record is one independently decoded item introduced by the record
//! separator (RS, `0x1E`) and terminated by a line feed; the decoder surfaces
//! items as they complete and never aggregates the body (§19/§20 buffering
//! philosophy: only enough data to find and decode one record). The encoder
//! writes `RS` + `<json>` + `\n` per item.
//!
//! # Framing decisions (DECISIONS.md D-impl-jsonseq-eof)
//!
//! - RS introduces every record. Bytes before the first RS — or between one
//!   record's LF and the next RS — yield
//!   [`JsonSeqDecodeError::MissingRecordSeparator`]: a record whose first byte
//!   is not RS is never guessed into existence.
//! - The terminating LF is REQUIRED: EOF before it yields
//!   [`JsonSeqDecodeError::Truncated`]. RFC 7464 lets parsers recover a
//!   truncated final record ("MAY"); v1 deliberately does NOT, keeping
//!   truncation observable per §40 rather than guessed away.
//! - A record whose content is empty (RS immediately followed by LF) is
//!   [`JsonSeqDecodeError::MalformedJson`] and terminates the stream.
//! - Per-record bound `limit` (`max_stream_record_bytes`, §33) counts RS and
//!   LF together with the content: accumulation past `limit` fails with
//!   [`JsonSeqDecodeError::RecordTooLarge`] before anything from that record
//!   is yielded, without polling the producer again.
//! - Non-UTF-8 record bytes are [`JsonSeqDecodeError::NotUtf8`]; invalid JSON
//!   is [`JsonSeqDecodeError::MalformedJson`] (serde error as source); both
//!   terminate the stream fail-fast.
//! - Transport failures surface as
//!   [`JsonSeqDecodeError::Source`](crate::stream_errors::JsonSeqDecodeError::Source),
//!   preserving the underlying error. Dropping the stream after any terminal
//!   item cancels the producer.
//!
//! # Memory bounds
//!
//! One carry buffer holds at most the current partial record (content plus
//! its pending LF): complete records are extracted before any oversize check,
//! so between source frames the buffer stays within `limit` bytes of one
//! record, momentarily plus the largest inbound chunk during append. Nothing
//! tracks total body size, and backpressure is purely poll-driven.

use std::error::Error;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes, BytesMut};
use futures_core::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::encode::{serialize_json_framed, EncodeTooLarge};
use crate::stream_errors::JsonSeqDecodeError;

/// RFC 7464 record separator octet introducing each sequence record.
pub const RECORD_SEPARATOR: u8 = 0x1E;

/// Decodes an RFC 7464 JSON Text Sequence body into one item per
/// RS-introduced record (main spec §20), bounding each record — separators
/// included — by `limit` bytes (`max_stream_record_bytes`).
///
/// Poll-driven over `chunks`: nothing is read ahead of consumer demand, and
/// the first framing/validation failure is yielded as the final item. See the
/// module docs for the exact framing contract and memory bounds.
pub fn decode_jsonseq<T, S, E>(
    chunks: S,
    limit: usize,
) -> impl Stream<Item = Result<T, JsonSeqDecodeError>>
where
    T: DeserializeOwned,
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    JsonSeqStream {
        chunks: Box::pin(chunks),
        limit,
        buf: BytesMut::new(),
        state: SeqState::ExpectSeparator,
        finished: false,
        item: PhantomData,
    }
}

/// Encodes one item as an RFC 7464 record: `RS` + `<json>` + `\n`, bounded by
/// `limit` bytes (D-impl-stream-item-bounds: per-item encode uses
/// `max_stream_record_bytes`, not `structured_encode_bytes`).
///
/// Fail-fast via [`CountingWriter`](crate::encode::CountingWriter): overflow
/// yields [`EncodeTooLarge`] and no partial output escapes.
pub fn encode_jsonseq_item<T>(item: &T, limit: usize) -> Result<Bytes, EncodeTooLarge>
where
    T: Serialize + ?Sized,
{
    serialize_json_framed(item, limit, &[RECORD_SEPARATOR], b"\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeqState {
    /// Hunting the next RS (stream start, or just past a record's LF).
    ExpectSeparator,
    /// Inside a record, accumulating toward the mandatory LF.
    InRecord,
}

enum Fill {
    Pending,
    Eof,
    Filled,
    Source(Box<dyn Error + Send + Sync>),
}

struct JsonSeqStream<T, S> {
    chunks: Pin<Box<S>>,
    limit: usize,
    /// Carry buffer for the current partial record (module docs: bounds).
    buf: BytesMut,
    state: SeqState,
    finished: bool,
    item: PhantomData<fn() -> T>,
}

impl<T, S, E> JsonSeqStream<T, S>
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
                self.buf.extend_from_slice(&chunk);
                Fill::Filled
            }
            Poll::Ready(Some(Err(error))) => Fill::Source(error.into()),
        }
    }

    /// Parses one completed record's content (the bytes between RS and LF);
    /// every failure here terminates the stream.
    fn parse_record(&self, content: &[u8]) -> Result<T, JsonSeqDecodeError> {
        if content.is_empty() {
            // RS immediately followed by LF: no document to parse. Surface
            // the standard MalformedJson shape for uniformity.
            let empty = serde_json::from_str::<serde_json::Value>("")
                .expect_err("empty input is invalid JSON");
            return Err(JsonSeqDecodeError::MalformedJson(empty));
        }
        // Belt-and-braces with the buffering cap: a single chunk may carry a
        // whole record at once, bypassing incremental accumulation.
        if content.len() + 2 > self.limit {
            return Err(JsonSeqDecodeError::RecordTooLarge { limit: self.limit });
        }
        let text = std::str::from_utf8(content).map_err(|_| JsonSeqDecodeError::NotUtf8)?;
        serde_json::from_str(text).map_err(JsonSeqDecodeError::MalformedJson)
    }
}

impl<T, S, E> Stream for JsonSeqStream<T, S>
where
    T: DeserializeOwned,
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    type Item = Result<T, JsonSeqDecodeError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.finished {
                return Poll::Ready(None);
            }
            match this.state {
                SeqState::ExpectSeparator => match this.buf.first().copied() {
                    Some(RECORD_SEPARATOR) => {
                        this.buf.advance(1);
                        this.state = SeqState::InRecord;
                    }
                    Some(_) => {
                        return this.fail(JsonSeqDecodeError::MissingRecordSeparator);
                    }
                    None => match this.fill(cx) {
                        Fill::Pending => return Poll::Pending,
                        Fill::Eof => return this.finish(None),
                        Fill::Filled => {}
                        Fill::Source(error) => {
                            return this.finish(Some(JsonSeqDecodeError::Source(error)));
                        }
                    },
                },
                SeqState::InRecord => {
                    if let Some(end) = this.buf.iter().position(|byte| *byte == b'\n') {
                        let mut content = this.buf.split_to(end + 1);
                        content.truncate(end); // drop the terminating LF
                        let content = content.freeze();
                        this.state = SeqState::ExpectSeparator;
                        return match this.parse_record(&content) {
                            Ok(item) => Poll::Ready(Some(Ok(item))),
                            Err(error) => this.finish(Some(error)),
                        };
                    }
                    // Record budget counts both separators: RS consumed above
                    // plus the LF still owed. Fail before polling again so an
                    // oversized record never surfaces partially.
                    if this.buf.len() + 2 > this.limit {
                        let limit = this.limit;
                        return this.finish(Some(JsonSeqDecodeError::RecordTooLarge { limit }));
                    }
                    match this.fill(cx) {
                        Fill::Pending => return Poll::Pending,
                        // No recovery of a truncated final record (module docs).
                        Fill::Eof => return this.finish(Some(JsonSeqDecodeError::Truncated)),
                        Fill::Filled => {}
                        Fill::Source(error) => {
                            return this.finish(Some(JsonSeqDecodeError::Source(error)));
                        }
                    }
                }
            }
        }
    }
}

impl<T, S, E> JsonSeqStream<T, S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    /// Emits a terminal error item; afterwards the stream yields `None`.
    fn fail(&mut self, error: JsonSeqDecodeError) -> Poll<Option<Result<T, JsonSeqDecodeError>>> {
        self.finished = true;
        Poll::Ready(Some(Err(error)))
    }

    fn finish(
        &mut self,
        error: Option<JsonSeqDecodeError>,
    ) -> Poll<Option<Result<T, JsonSeqDecodeError>>> {
        self.finished = true;
        match error {
            Some(error) => Poll::Ready(Some(Err(error))),
            None => Poll::Ready(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Metric {
        key: String,
        value: u32,
    }

    type Chunk = Result<Bytes, std::convert::Infallible>;

    fn chunks(parts: Vec<Vec<u8>>) -> impl Stream<Item = Chunk> {
        futures_util::stream::iter(parts.into_iter().map(|part| Ok(Bytes::from(part))))
    }

    async fn collect_items<T, S>(stream: S) -> Result<Vec<T>, JsonSeqDecodeError>
    where
        T: DeserializeOwned,
        S: Stream<Item = Result<T, JsonSeqDecodeError>>,
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

    fn metric(key: &str, value: u32) -> Metric {
        Metric {
            key: key.to_owned(),
            value,
        }
    }

    fn frame(key: &str, value: u32) -> Vec<u8> {
        let mut out = vec![RECORD_SEPARATOR];
        out.extend_from_slice(
            serde_json::to_string(&metric(key, value))
                .expect("serialize")
                .as_bytes(),
        );
        out.push(b'\n');
        out
    }

    fn canonical() -> Vec<u8> {
        let mut body = frame("cpu", 1);
        body.extend(frame("mem", 2));
        body
    }

    #[tokio::test]
    async fn decodes_rs_lf_records_across_arbitrary_chunk_splits() {
        let body = canonical();
        let mut split_body = body[..5].to_vec();
        split_body.extend_from_slice(&body[5..12]);
        split_body.extend_from_slice(&body[12..]);
        let items = collect_items(decode_jsonseq::<Metric, _, _>(
            chunks(vec![split_body]),
            4096,
        ))
        .await;
        assert_eq!(
            items.expect("well-framed"),
            vec![metric("cpu", 1), metric("mem", 2)]
        );
    }

    #[tokio::test]
    async fn empty_body_is_a_clean_empty_stream() {
        let items = collect_items(decode_jsonseq::<Metric, _, _>(chunks(Vec::new()), 64)).await;
        assert!(items.expect("empty body").is_empty());
    }

    #[tokio::test]
    async fn bytes_before_first_separator_are_missing_record_separator() {
        let mut body = b"junk".to_vec();
        body.extend(frame("cpu", 1));
        let error = collect_items(decode_jsonseq::<Metric, _, _>(chunks(vec![body]), 256)).await;
        assert!(
            matches!(
                error.expect_err("no leading RS"),
                JsonSeqDecodeError::MissingRecordSeparator
            ),
            "expected MissingRecordSeparator"
        );

        // Junk between records trips the same rule (strict RFC framing).
        let mut body = frame("cpu", 1);
        body.extend_from_slice(b"junk");
        body.extend(frame("mem", 2));
        let error = collect_items(decode_jsonseq::<Metric, _, _>(chunks(vec![body]), 256)).await;
        assert!(
            matches!(
                error.expect_err("junk between records"),
                JsonSeqDecodeError::MissingRecordSeparator
            ),
            "expected MissingRecordSeparator"
        );
    }

    #[tokio::test]
    async fn rs_immediately_followed_by_lf_is_malformed_empty_record() {
        let mut body = frame("cpu", 1);
        body.extend_from_slice(&[RECORD_SEPARATOR, b'\n']);
        body.extend(frame("mem", 2));
        let error = collect_items(decode_jsonseq::<Metric, _, _>(chunks(vec![body]), 256)).await;
        assert!(matches!(
            error.expect_err("empty record"),
            JsonSeqDecodeError::MalformedJson(_)
        ));
    }

    #[tokio::test]
    async fn eof_before_terminating_lf_is_truncated() {
        // Mid-record tail.
        let mut body = frame("cpu", 1);
        body.extend(frame("mem", 2));
        body.truncate(body.len() - 3); // cut inside the last record
        let error = collect_items(decode_jsonseq::<Metric, _, _>(chunks(vec![body]), 4096)).await;
        assert!(
            matches!(
                error.expect_err("EOF mid-record"),
                JsonSeqDecodeError::Truncated
            ),
            "expected Truncated"
        );

        // A lone RS with nothing behind it truncates too (never recovered).
        let body = vec![RECORD_SEPARATOR];
        let error = collect_items(decode_jsonseq::<Metric, _, _>(chunks(vec![body]), 4096)).await;
        assert!(
            matches!(
                error.expect_err("RS then EOF"),
                JsonSeqDecodeError::Truncated
            ),
            "expected Truncated"
        );

        // Clean boundary for contrast: full canonical body ends cleanly.
        let items = collect_items(decode_jsonseq::<Metric, _, _>(
            chunks(vec![canonical()]),
            4096,
        ))
        .await
        .expect("clean EOF");
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn non_utf8_or_malformed_records_terminate_fail_fast() {
        let raw: Vec<u8> = vec![RECORD_SEPARATOR, b'{', b'"', b'k', 0xFF, b'}', b'\n'];
        let error = collect_items::<Metric, _>(decode_jsonseq(chunks(vec![raw]), 64)).await;
        assert!(
            matches!(
                error.expect_err("invalid UTF-8"),
                JsonSeqDecodeError::NotUtf8
            ),
            "expected NotUtf8"
        );

        let mut body = vec![RECORD_SEPARATOR];
        body.extend_from_slice(b"{oops}\n");
        let mut stream = Box::pin(decode_jsonseq::<Metric, _, _>(chunks(vec![body]), 64));
        let first = stream.as_mut().next().await.expect("error item");
        assert!(matches!(first, Err(JsonSeqDecodeError::MalformedJson(_))));
        assert!(stream.as_mut().next().await.is_none(), "terminal");
    }

    /// Oversized record rejected before yielding anything; producer stops
    /// being polled entirely (§50 test 19 semantics).
    #[tokio::test]
    async fn oversize_record_rejects_without_polling_producer_again() {
        struct PanickingStream {
            polled: usize,
        }

        impl Stream for PanickingStream {
            type Item = Chunk;

            fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Chunk>> {
                let polled = &mut self.get_mut().polled;
                *polled += 1;
                match *polled {
                    1 => Poll::Ready(Some(Ok(Bytes::from(frame("cpu", 1))))),
                    2 => Poll::Ready(Some(Ok(Bytes::from_static(
                        b"\x1E{\"key\":\"m\",\"value\":99999999999999999999999}",
                    )))),
                    _ => panic!("producer polled after RecordTooLarge rejection"),
                }
            }
        }

        let mut stream = Box::pin(decode_jsonseq::<Metric, _, _>(
            PanickingStream { polled: 0 },
            32,
        ));
        let first = stream.as_mut().next().await.expect("first record fits");
        assert_eq!(first.expect("first record"), metric("cpu", 1));
        let second = stream.as_mut().next().await.expect("rejection item");
        assert!(
            matches!(
                second.expect_err("oversize"),
                JsonSeqDecodeError::RecordTooLarge { limit: 32 }
            ),
            "expected RecordTooLarge {{ limit: 32 }}"
        );
        assert!(stream.as_mut().next().await.is_none());
    }

    #[tokio::test]
    async fn record_exactly_at_limit_including_separators_succeeds() {
        let framed = frame("cpu", 7); // RS + json + LF
        assert_eq!(framed.first(), Some(&RECORD_SEPARATOR));
        let framed_len = framed.len();

        let items = collect_items(decode_jsonseq::<Metric, _, _>(
            chunks(vec![framed.clone()]),
            framed_len,
        ))
        .await
        .expect("exactly-at-limit record");
        assert_eq!(items, vec![metric("cpu", 7)]);

        let error = collect_items(decode_jsonseq::<Metric, _, _>(
            chunks(vec![framed]),
            framed_len - 1,
        ))
        .await
        .expect_err("one byte over");
        assert!(
            matches!(
                error,
                JsonSeqDecodeError::RecordTooLarge { limit } if limit == framed_len - 1
            ),
            "expected RecordTooLarge, got {error:?}"
        );
    }

    #[tokio::test]
    async fn transport_failure_surfaces_as_source_not_truncation() {
        #[derive(Debug, thiserror::Error)]
        #[error("socket reset")]
        struct TransportDown;

        let frames: Vec<Result<Bytes, TransportDown>> =
            vec![Ok(Bytes::from(frame("cpu", 1))), Err(TransportDown)];
        let error = collect_items(decode_jsonseq::<Metric, _, _>(
            futures_util::stream::iter(frames),
            256,
        ))
        .await
        .expect_err("transport died mid-body");
        match error {
            JsonSeqDecodeError::Source(source) => {
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

    /// §50 test 42 adapted to JSON sequences: every single split point of the
    /// canonical body (plus 1–3-byte systematic re-chunkings) decodes
    /// identically to the unsplit run.
    #[tokio::test]
    async fn every_single_split_point_matches_unsplit_run() {
        let body = canonical();
        let baseline = collect_items(decode_jsonseq::<Metric, _, _>(
            chunks(vec![body.clone()]),
            4096,
        ))
        .await
        .expect("canonical parses");

        for step in 1..=3_usize {
            for offset in 1..body.len() {
                let mut parts = vec![body[..offset].to_vec()];
                parts.extend(
                    body[offset..]
                        .chunks(step)
                        .filter(|chunk| !chunk.is_empty())
                        .map(<[u8]>::to_vec),
                );
                let outcome = collect_items(decode_jsonseq::<Metric, _, _>(chunks(parts), 4096))
                    .await
                    .unwrap_or_else(|error| {
                        panic!("step {step} offset {offset}: unexpected {error:?}")
                    });
                assert_eq!(outcome, baseline, "step {step} offset {offset}");
            }
        }
    }

    /// Exhaustive chunk compositions of a micro body (§50 test 42): every way
    /// to cut one tiny record into ≤3-byte chunks decodes like the unsplit
    /// run.
    #[tokio::test]
    async fn exhaustive_micro_compositions_match_unsplit_run() {
        let micro = [RECORD_SEPARATOR, b'1', b'\n'];
        let baseline = collect_items(decode_jsonseq::<serde_json::Value, _, _>(
            chunks(vec![micro.to_vec()]),
            64,
        ))
        .await
        .expect("micro parses");
        assert_eq!(baseline, vec![serde_json::Value::from(1)]);

        for sizes in compositions(micro.len(), 3) {
            let offsets: Vec<usize> = sizes
                .iter()
                .scan(0, |acc, size| {
                    *acc += size;
                    Some(*acc)
                })
                .take(sizes.len() - 1)
                .collect();
            let outcome = collect_items(decode_jsonseq::<serde_json::Value, _, _>(
                chunks(split_at_offsets(&micro, &offsets)),
                64,
            ))
            .await
            .unwrap_or_else(|error| panic!("composition {sizes:?}: unexpected {error:?}"));
            assert_eq!(outcome, baseline, "composition {sizes:?}");
        }
    }

    #[tokio::test]
    async fn encode_writes_rs_json_lf_and_round_trips() {
        let cpu = metric("gear", 9);
        let bytes = encode_jsonseq_item(&cpu, 256).expect("under limit");
        let mut expected = vec![RECORD_SEPARATOR];
        expected.extend_from_slice(b"{\"key\":\"gear\",\"value\":9}");
        expected.push(b'\n');
        assert_eq!(bytes.as_ref(), expected.as_slice());

        let decoded = collect_items(decode_jsonseq::<Metric, _, _>(
            chunks(vec![bytes.to_vec()]),
            256,
        ))
        .await
        .expect("clean");
        assert_eq!(decoded, vec![cpu]);
    }

    #[test]
    fn encode_overflow_yields_encode_too_large_with_no_partial_output() {
        let long = metric(&"w".repeat(64), 0);
        let error = encode_jsonseq_item(&long, 16).expect_err("over limit");
        assert_eq!(error, EncodeTooLarge { limit: 16 });

        // Exactly-at-limit frame (RS + JSON + LF all counted).
        let exact = metric("abcd", 1);
        let encoded_len = serde_json::to_vec(&exact).expect("serialize").len() + 2;
        let ok = encode_jsonseq_item(&exact, encoded_len).expect("exact fit");
        assert_eq!(ok.len(), encoded_len);
    }

    #[test]
    fn returned_stream_is_send() {
        fn assert_send<T: Send>(value: T) -> T {
            value
        }
        let stream = decode_jsonseq::<Metric, _, _>(chunks(Vec::new()), 64);
        let _boxed: Pin<Box<dyn Stream<Item = Result<Metric, JsonSeqDecodeError>> + Send>> =
            Box::pin(assert_send(stream));
    }
}
