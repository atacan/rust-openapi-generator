//! Peek-and-preserve body presence detection (main spec §28.2).
//!
//! §28.2 implementation invariant: body emptiness cannot be inferred from
//! `Content-Length`, which may be absent on chunked HTTP/1.1 and HTTP/2
//! transfers. For optional-body operations presence is decided by awaiting the
//! first body data frame, subject to a small bounded peek cap
//! (`peek_buffer_bytes`). When bytes arrive they are preserved by re-prepending
//! them to the stream passed onward for decoding; the whole body is never
//! collected to determine emptiness and buffered frames are never discarded.

use std::future::poll_fn;

use bytes::{Bytes, BytesMut};
use futures_core::Stream;
use futures_util::StreamExt;

/// Outcome of [`detect_body_presence`].
#[derive(Debug)]
pub enum BodyPresence {
    /// EOF arrived before any nonzero byte (§28.2: empty body).
    Empty,
    /// At least one byte arrived; carries every peeked byte, at most
    /// `peek_cap` of them.
    NonEmpty(Bytes),
    /// The source stream failed during detection. This is NOT absence: the
    /// returned stream replays any bytes seen before the failure and then
    /// yields the original error, so callers observe a decode/transport
    /// failure rather than silently treating the request as empty-bodied.
    Failed,
}

/// Decides body presence per §28.2 without discarding anything.
///
/// Polls frames, skipping zero-length frames, accumulating up to `peek_cap`
/// bytes (a frame crossing the cap contributes only its prefix). EOF before
/// any byte yields [`BodyPresence::Empty`]; reaching the cap yields
/// [`BodyPresence::NonEmpty`] with what was seen. A source error yields
/// [`BodyPresence::Failed`].
///
/// The returned stream re-delivers every consumed byte exactly once — peeked
/// frames are re-prepended ahead of the remaining stream — and in the
/// [`BodyPresence::Failed`] case it terminates with the preserved source
/// error. `peek_cap` values below 1 are clamped to 1 because classifying
/// presence requires observing at least one byte.
pub async fn detect_body_presence(
    body: axum::body::BodyDataStream,
    peek_cap: usize,
) -> (BodyPresence, axum::body::BodyDataStream) {
    let cap = peek_cap.max(1);
    // Owned boxed stream: pollable during detection and chainable afterwards,
    // so no consumed byte is lost.
    let mut body: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, axum::Error>> + Send>> =
        Box::pin(body);
    let mut frames: Vec<Bytes> = Vec::new();
    let mut total = 0usize;
    let mut tail: Option<Bytes> = None;
    let mut failure: Option<axum::Error> = None;

    loop {
        match poll_fn(|cx| body.as_mut().poll_next(cx)).await {
            None => break,
            Some(Err(error)) => {
                failure = Some(error);
                break;
            }
            Some(Ok(frame)) => {
                let len = frame.len();
                // Zero-length frames never decide presence (§28.2).
                if len == 0 {
                    continue;
                }
                if total + len <= cap {
                    total += len;
                    frames.push(frame);
                    if total == cap {
                        break;
                    }
                    continue;
                }
                // Crossing the cap: buffer only the prefix that fits.
                let take = cap - total;
                frames.push(frame.slice(..take));
                tail = Some(frame.slice(take..));
                total = cap;
                break;
            }
        }
    }

    let presence = if failure.is_some() {
        BodyPresence::Failed
    } else if total == 0 {
        BodyPresence::Empty
    } else {
        let mut merged = BytesMut::with_capacity(total);
        for frame in &frames {
            merged.extend_from_slice(frame);
        }
        BodyPresence::NonEmpty(Bytes::from(merged))
    };

    // Re-prepend everything consumed so the decoder sees each byte once; a
    // preserved source error is yielded after the replay.
    let mut replay: Vec<Result<Bytes, axum::Error>> = frames.into_iter().map(Ok).collect();
    if let Some(tail) = tail {
        replay.push(Ok::<_, axum::Error>(tail));
    }
    if let Some(error) = failure {
        replay.push(Err::<_, axum::Error>(error));
    }
    let stream = futures_util::stream::iter(replay).chain(body);
    (
        presence,
        axum::body::Body::from_stream(stream).into_data_stream(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, BodyDataStream};
    use bytes::Buf;

    type Frame = Result<Bytes, axum::Error>;

    fn body_from(frames: Vec<Frame>) -> BodyDataStream {
        Body::from_stream(futures_util::stream::iter(frames)).into_data_stream()
    }

    fn text(value: &'static str) -> Frame {
        Ok(Bytes::from_static(value.as_bytes()))
    }

    fn error() -> Frame {
        Err(axum::Error::new(std::io::Error::other("source failed")))
    }

    async fn drain(mut stream: BodyDataStream) -> (String, Option<axum::Error>) {
        let mut out = String::new();
        loop {
            match StreamExt::next(&mut stream).await {
                None => return (out, None),
                Some(Ok(mut chunk)) => {
                    while chunk.has_remaining() {
                        let slice = chunk.chunk();
                        out.push_str(std::str::from_utf8(slice).expect("utf-8"));
                        let copied = slice.len();
                        chunk.advance(copied);
                    }
                }
                Some(Err(error)) => return (out, Some(error)),
            }
        }
    }
    #[tokio::test]
    async fn eof_before_any_byte_is_empty_with_untouched_stream() {
        let (presence, rest) = detect_body_presence(body_from(vec![]), 16).await;
        assert!(matches!(presence, BodyPresence::Empty));
        let (replayed, error) = drain(rest).await;
        assert_eq!(replayed, "");
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn leading_zero_length_frames_never_decide_presence() {
        let (presence, _rest) =
            detect_body_presence(body_from(vec![text(""), text(""), text("")]), 16).await;
        assert!(matches!(presence, BodyPresence::Empty));

        let (presence, rest) = detect_body_presence(body_from(vec![text(""), text("x")]), 16).await;
        assert!(matches!(presence, BodyPresence::NonEmpty(bytes) if bytes.as_ref() == b"x"));
        let (replayed, error) = drain(rest).await;
        assert_eq!(replayed, "x");
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn chunked_splits_are_replayed_exactly_once() {
        let (presence, rest) =
            detect_body_presence(body_from(vec![text("hel"), text("lo "), text("world")]), 4).await;
        // Cap reached after two frames: "hell" peeked.
        assert!(matches!(presence, BodyPresence::NonEmpty(ref bytes) if bytes.as_ref() == b"hell"));
        let (replayed, error) = drain(rest).await;
        assert_eq!(replayed, "hello world");
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn frame_crossing_the_cap_contributes_only_its_prefix() {
        let (presence, rest) = detect_body_presence(body_from(vec![text("abcdef")]), 3).await;
        assert!(matches!(presence, BodyPresence::NonEmpty(ref bytes) if bytes.as_ref() == b"abc"));
        let (replayed, _) = drain(rest).await;
        assert_eq!(replayed, "abcdef");
    }

    #[tokio::test]
    async fn zero_length_frames_do_not_trip_the_cap() {
        let (presence, rest) = detect_body_presence(
            body_from(vec![text(""), text("ab"), text(""), text("cd")]),
            4,
        )
        .await;
        assert!(matches!(presence, BodyPresence::NonEmpty(ref bytes) if bytes.as_ref() == b"abcd"));
        let (replayed, error) = drain(rest).await;
        assert_eq!(replayed, "abcd");
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn error_after_peek_yields_failed_then_replays_then_errors() {
        let (presence, rest) =
            detect_body_presence(body_from(vec![text("abc"), error(), text("never")]), 64).await;
        assert!(matches!(presence, BodyPresence::Failed), "got {presence:?}");
        let (replayed, error) = drain(rest).await;
        assert_eq!(replayed, "abc", "peeked bytes are never discarded");
        assert!(error.is_some(), "source error propagates after the replay");
    }

    #[tokio::test]
    async fn error_before_any_byte_is_failed_not_empty() {
        let (presence, rest) = detect_body_presence(body_from(vec![error()]), 16).await;
        assert!(matches!(presence, BodyPresence::Failed));
        let (_, error) = drain(rest).await;
        assert!(error.is_some());
    }
}
