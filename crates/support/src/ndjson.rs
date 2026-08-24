//! Incremental NDJSON (newline-delimited JSON) record framing (main spec §19).
//!
//! Each `\n`-terminated line is one independently decoded item; the decoder
//! surfaces items as they complete and never aggregates the body (§19: buffer
//! only enough data to find and decode one record). The encoder writes one
//! bounded document per item followed by a newline.
//!
//! # Framing decisions (DECISIONS.md D-impl-ndjson-lines)
//!
//! - Records split on `\n` only. A single trailing terminator is part of the
//!   format: EOF right after a terminator is a clean end-of-stream.
//! - Interior empty lines are [`NdjsonDecodeError::MalformedJson`] (an empty
//!   line is a record position carrying no JSON — never skipped silently),
//!   and they terminate the stream fail-fast like any other decode error.
//! - EOF mid-record (bytes accumulated without their terminating newline) is
//!   [`NdjsonDecodeError::Truncated`], distinct from clean EOF per §40.
//! - Per-record bound `limit` (`max_stream_record_bytes`, §33) counts the
//!   record's own bytes, not its terminator: accumulation of an unterminated
//!   record past `limit` fails with [`NdjsonDecodeError::RecordTooLarge`]
//!   before anything from that record is yielded, and a completed line longer
//!   than `limit` is rejected the same way. Either path terminates the stream
//!   without polling the producer again (§50 test 19 semantics: rejection
//!   cancels the rest of the stream).
//! - Non-UTF-8 line bytes are [`NdjsonDecodeError::NotUtf8`]; invalid JSON is
//!   [`NdjsonDecodeError::MalformedJson`] (with the serde error as source)
//!   and both terminate the stream — never skip-and-continue.
//! - Transport failures surface as
//!   [`NdjsonDecodeError::Source`](crate::stream_errors::NdjsonDecodeError::Source),
//!   preserving the underlying error instead of masquerading as truncation.
//!   Dropping the stream after any terminal item cancels the producer.
//!
//! # Memory bounds
//!
//! One carry buffer holds at most the current partial record: complete
//! records are extracted before any oversize check, so between source frames
//! the buffer never exceeds `limit` bytes of one record, momentarily plus the
//! largest inbound chunk during append. Nothing tracks total body size, and
//! backpressure is purely poll-driven (pull-based, no internal channel).

use std::error::Error;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::encode::{serialize_json_framed, EncodeTooLarge};
use crate::stream_errors::NdjsonDecodeError;

/// Decodes an NDJSON body into one item per `\n`-terminated line (main spec
/// §19), bounding each record by `limit` bytes (`max_stream_record_bytes`).
///
/// Poll-driven over `chunks`: nothing is read ahead of consumer demand, and
/// the first framing/validation failure is yielded as the final item. See the
/// module docs for the exact framing contract and memory bounds.
pub fn decode_ndjson<T, S, E>(
    chunks: S,
    limit: usize,
) -> impl Stream<Item = Result<T, NdjsonDecodeError>>
where
    T: DeserializeOwned,
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    NdjsonStream {
        chunks: Box::pin(chunks),
        limit,
        buf: BytesMut::new(),
        finished: false,
        item: PhantomData,
    }
}

/// Encodes one item as an NDJSON record: `<json>` followed by `\n`, bounded
/// by `limit` bytes (D-impl-stream-item-bounds: per-item encode uses
/// `max_stream_record_bytes`, not `structured_encode_bytes`).
///
/// Fail-fast via [`CountingWriter`](crate::encode::CountingWriter): overflow
/// yields [`EncodeTooLarge`] and no partial output escapes.
pub fn encode_ndjson_item<T>(item: &T, limit: usize) -> Result<Bytes, EncodeTooLarge>
where
    T: Serialize + ?Sized,
{
    serialize_json_framed(item, limit, b"", b"\n")
}

enum Fill {
    Pending,
    Eof,
    Filled,
    Source(Box<dyn Error + Send + Sync>),
}

struct NdjsonStream<T, S> {
    chunks: Pin<Box<S>>,
    limit: usize,
    /// Carry buffer for the current partial record (module docs: bounds).
    buf: BytesMut,
    finished: bool,
    item: PhantomData<fn() -> T>,
}

impl<T, S, E> NdjsonStream<T, S>
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

    /// Parses one completed record; every failure here terminates the stream.
    fn parse_record(&self, line: &[u8]) -> Result<T, NdjsonDecodeError> {
        if line.len() > self.limit {
            return Err(NdjsonDecodeError::RecordTooLarge { limit: self.limit });
        }
        let text = std::str::from_utf8(line).map_err(|_| NdjsonDecodeError::NotUtf8)?;
        serde_json::from_str(text).map_err(NdjsonDecodeError::MalformedJson)
    }
}

impl<T, S, E> Stream for NdjsonStream<T, S>
where
    T: DeserializeOwned,
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<Box<dyn Error + Send + Sync>>,
{
    type Item = Result<T, NdjsonDecodeError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.finished {
                return Poll::Ready(None);
            }
            // Complete records first: extraction precedes any oversize check
            // so one large multi-record chunk never trips the per-record cap
            // for records it merely transports.
            if let Some(end) = this.buf.iter().position(|byte| *byte == b'\n') {
                let mut line = this.buf.split_to(end + 1);
                line.truncate(end); // drop the terminator
                let line = line.freeze();
                return match this.parse_record(&line) {
                    Ok(item) => Poll::Ready(Some(Ok(item))),
                    Err(error) => {
                        this.finished = true;
                        Poll::Ready(Some(Err(error)))
                    }
                };
            }
            // No terminator left in the buffer: what remains is one partial
            // record, so it can no longer shrink back under `limit`.
            if this.buf.len() > this.limit {
                let limit = this.limit;
                this.finished = true;
                return Poll::Ready(Some(Err(NdjsonDecodeError::RecordTooLarge { limit })));
            }
            match this.fill(cx) {
                Fill::Pending => return Poll::Pending,
                // EOF exactly on a record boundary is clean; otherwise the
                // tail is a record missing its terminator (§40 truncation).
                Fill::Eof => {
                    let truncated = !this.buf.is_empty();
                    this.finished = true;
                    return if truncated {
                        Poll::Ready(Some(Err(NdjsonDecodeError::Truncated)))
                    } else {
                        Poll::Ready(None)
                    };
                }
                Fill::Filled => continue,
                Fill::Source(error) => {
                    this.finished = true;
                    return Poll::Ready(Some(Err(NdjsonDecodeError::Source(error))));
                }
            }
        }
    }
}

// An interior empty line carries no JSON document; surface the same
// MalformedJson shape a serde parse would produce so callers see one variant
// regardless of which check catches it.
// An interior empty line reaches parse_record as empty input; serde rejects
// it, so callers see the standard MalformedJson variant regardless of which
// check catches a bad line.

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

    async fn collect_items<T, S>(stream: S) -> Result<Vec<T>, NdjsonDecodeError>
    where
        T: DeserializeOwned,
        S: Stream<Item = Result<T, NdjsonDecodeError>>,
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

    const CANONICAL: &str = "{\"name\":\"a\",\"count\":1}\n{\"name\":\"b\",\"count\":2}\n";

    fn widget(name: &str, count: u32) -> Widget {
        Widget {
            name: name.to_owned(),
            count,
        }
    }

    fn canonical_expected() -> Vec<Widget> {
        vec![widget("a", 1), widget("b", 2)]
    }

    #[tokio::test]
    async fn decodes_records_across_arbitrary_chunk_splits() {
        // §50 test 18: records split across arbitrary HTTP chunks.
        let items = collect_items(decode_ndjson::<Widget, _, _>(
            chunks(vec![
                "{\"name",
                "\":\"a\",\"c",
                "ount\":1}\n{",
                "\"name\":\"b\",\"count\":2}\n",
            ]),
            256,
        ))
        .await
        .expect("well-framed body");
        assert_eq!(items, canonical_expected());
    }

    #[tokio::test]
    async fn empty_body_is_a_clean_empty_stream() {
        let items = collect_items(decode_ndjson::<Widget, _, _>(chunks(Vec::new()), 64))
            .await
            .expect("empty body");
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn interior_empty_line_is_malformed_json_and_terminates() {
        let error = collect_items(decode_ndjson::<Widget, _, _>(
            chunks(vec!["{\"name\":\"a\",\"count\":1}\n\n{\"name\":\"b\"}"]),
            256,
        ))
        .await
        .expect_err("interior blank line");
        assert!(
            matches!(error, NdjsonDecodeError::MalformedJson(_)),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn lone_trailing_terminator_after_last_record_is_clean() {
        let items = collect_items(decode_ndjson::<Widget, _, _>(chunks(vec![CANONICAL]), 256))
            .await
            .expect("trailing terminator is part of the format");
        assert_eq!(items, canonical_expected());

        // A doubled trailing terminator still exposes an interior empty line.
        let doubled: &'static str = "{\"name\":\"a\",\"count\":1}\n\n";
        assert!(
            collect_items(decode_ndjson::<Widget, _, _>(chunks(vec![doubled]), 256))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn eof_mid_record_is_truncated_distinct_from_clean_eof() {
        let error = collect_items(decode_ndjson::<Widget, _, _>(
            chunks(vec!["{\"name\":\"a\",\"count\":1}\n{\"name\":\"b\","]),
            256,
        ))
        .await
        .expect_err("EOF mid-record");
        assert!(matches!(error, NdjsonDecodeError::Truncated), "{error:?}");

        // Clean boundary: identical prefix ending in its terminator.
        let items = collect_items(decode_ndjson::<Widget, _, _>(
            chunks(vec!["{\"name\":\"a\",\"count\":1}\n"]),
            256,
        ))
        .await
        .expect("clean EOF");
        assert_eq!(items, vec![widget("a", 1)]);
    }

    #[tokio::test]
    async fn non_utf8_line_reports_not_utf8() {
        let raw: Vec<u8> = b"{\"name\":\"caf\xE9\",\"count\":1}\n".to_vec();
        let error = collect_items::<Widget, _>(decode_ndjson(byte_chunks(vec![raw]), 256)).await;
        assert!(
            matches!(
                error.expect_err("invalid UTF-8"),
                NdjsonDecodeError::NotUtf8
            ),
            "expected NotUtf8"
        );
    }

    #[tokio::test]
    async fn malformed_json_terminates_stream_without_more_polls() {
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
                Poll::Ready(Some(Ok(Bytes::from_static(b"{not json}\n"))))
            }
        }

        let mut stream = Box::pin(decode_ndjson::<Widget, _, _>(
            PanickingStream { yielded: false },
            256,
        ));
        let first = stream.as_mut().next().await.expect("one error item");
        assert!(
            matches!(first, Err(NdjsonDecodeError::MalformedJson(_))),
            "{first:?}"
        );
        assert!(
            stream.as_mut().next().await.is_none(),
            "terminal error yields None afterwards"
        );
    }

    /// §50 test 19 semantics: an oversized record is rejected without
    /// collecting the rest — the producer stops being polled entirely.
    #[tokio::test]
    async fn oversize_rejects_before_yielding_the_record_and_stops_polling() {
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
                        b"{\"name\":\"a\",\"count\":1}\n",
                    )))),
                    // Overshoots the 32-byte budget mid-second-record.
                    2 => Poll::Ready(Some(Ok(Bytes::from_static(
                        b"{\"name\":\"b\",\"count\":22222222222222222222}",
                    )))),
                    _ => panic!("producer polled after RecordTooLarge rejection"),
                }
            }
        }

        let mut stream = Box::pin(decode_ndjson::<Widget, _, _>(
            PanickingStream { polled: 0 },
            32,
        ));
        let first = stream.as_mut().next().await.expect("first record fits");
        assert_eq!(first.expect("first record"), widget("a", 1));
        let second = stream.as_mut().next().await.expect("rejection item");
        assert!(
            matches!(
                second.expect_err("oversize"),
                NdjsonDecodeError::RecordTooLarge { limit: 32 }
            ),
            "expected RecordTooLarge {{ limit: 32 }}"
        );
        assert!(stream.as_mut().next().await.is_none());
    }

    #[tokio::test]
    async fn record_exactly_at_limit_succeeds_and_one_over_fails() {
        let line = serde_json::to_string(&widget("w", 7)).expect("serialize");

        let items = collect_items(decode_ndjson::<Widget, _, _>(
            byte_chunks(vec![format!("{line}\n").into_bytes()]),
            line.len(),
        ))
        .await
        .expect("exactly-at-limit record");
        assert_eq!(items, vec![widget("w", 7)]);

        let error = collect_items(decode_ndjson::<Widget, _, _>(
            byte_chunks(vec![format!("{line}\n").into_bytes()]),
            line.len() - 1,
        ))
        .await
        .expect_err("one byte over");
        assert!(
            matches!(
                error,
                NdjsonDecodeError::RecordTooLarge { limit } if limit == line.len() - 1
            ),
            "expected RecordTooLarge, got {error:?}"
        );
    }

    #[tokio::test]
    async fn transport_failure_surfaces_as_source_not_truncation() {
        #[derive(Debug, thiserror::Error)]
        #[error("socket reset")]
        struct TransportDown;

        let frames: Vec<Result<Bytes, TransportDown>> = vec![
            Ok(Bytes::from_static(b"{\"name\":\"a\",\"count\":1}\n")),
            Err(TransportDown),
        ];
        let error = collect_items(decode_ndjson::<Widget, _, _>(
            futures_util::stream::iter(frames),
            256,
        ))
        .await
        .expect_err("transport died mid-body");
        match error {
            NdjsonDecodeError::Source(source) => {
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

    /// §50 test 42 adapted to NDJSON: every single split point of the
    /// canonical body (plus 1–3-byte systematic re-chunkings) decodes
    /// identically to the unsplit run.
    #[tokio::test]
    async fn every_single_split_point_matches_unsplit_run() {
        let body = CANONICAL.as_bytes();
        let baseline = collect_items(decode_ndjson::<Widget, _, _>(
            byte_chunks(vec![body.to_vec()]),
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
                    collect_items(decode_ndjson::<Widget, _, _>(byte_chunks(parts), 4096))
                        .await
                        .unwrap_or_else(|error| {
                            panic!("step {step} offset {offset}: unexpected {error:?}")
                        });
                assert_eq!(outcome, baseline, "step {step} offset {offset}");
            }
        }
    }

    /// Exhaustive chunk compositions of a micro body (§50 test 42): every way
    /// to cut two tiny records into ≤3-byte chunks decodes like the unsplit
    /// run.
    #[tokio::test]
    async fn exhaustive_micro_compositions_match_unsplit_run() {
        let micro: &[u8] = b"[1]\n[2,3]\n";
        let baseline = collect_items::<serde_json::Value, _>(decode_ndjson(
            byte_chunks(vec![micro.to_vec()]),
            64,
        ))
        .await
        .expect("micro parses");
        assert_eq!(
            baseline,
            vec![serde_json::json!([1]), serde_json::json!([2, 3])]
        );

        for sizes in compositions(micro.len(), 3) {
            let offsets: Vec<usize> = sizes
                .iter()
                .scan(0, |acc, size| {
                    *acc += size;
                    Some(*acc)
                })
                .take(sizes.len() - 1)
                .collect();
            let outcome = collect_items::<serde_json::Value, _>(decode_ndjson(
                byte_chunks(split_at_offsets(micro, &offsets)),
                64,
            ))
            .await
            .unwrap_or_else(|error| panic!("composition {sizes:?}: unexpected {error:?}"));
            assert_eq!(outcome, baseline, "composition {sizes:?}");
        }
    }

    #[tokio::test]
    async fn encode_writes_json_plus_newline_and_round_trips() {
        let gear = widget("gear", 9);
        let bytes = encode_ndjson_item(&gear, 256).expect("under limit");
        assert_eq!(bytes.as_ref(), b"{\"name\":\"gear\",\"count\":9}\n");

        let decoded = collect_items(decode_ndjson::<Widget, _, _>(
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
        let error = encode_ndjson_item(&long, 16).expect_err("over limit");
        assert_eq!(error, EncodeTooLarge { limit: 16 });

        // Exactly-at-limit frame (terminator included in the budget).
        let exact = widget("abcd", 1);
        let encoded_len = serde_json::to_vec(&exact).expect("serialize").len() + 1;
        let ok = encode_ndjson_item(&exact, encoded_len).expect("exact fit");
        assert_eq!(ok.len(), encoded_len);
    }

    #[test]
    fn returned_stream_is_send() {
        fn assert_send<T: Send>(value: T) -> T {
            value
        }
        let stream = decode_ndjson::<Widget, _, _>(chunks(Vec::new()), 64);
        let _boxed: Pin<Box<dyn Stream<Item = Result<Widget, NdjsonDecodeError>> + Send>> =
            Box::pin(assert_send(stream));
    }
}
