//! Bounded collection of streamed bodies (main spec §30.2, §49).

use std::future::poll_fn;
use std::pin::pin;

use bytes::{Buf, Bytes};
use futures_core::Stream;

/// Failure modes of [`collect_limited`].
#[derive(Debug, thiserror::Error)]
pub enum CollectLimitedError<E> {
    /// The accumulated body would strictly exceed the configured byte limit.
    #[error("body exceeds limit of {limit} bytes")]
    TooLarge {
        /// The configured limit in bytes.
        limit: usize,
    },
    /// The source stream failed; its error is preserved.
    #[error(transparent)]
    Source(E),
}

/// Accumulates a chunked stream into `Bytes`, failing fast once `limit` bytes
/// would be exceeded.
///
/// A total of exactly `limit` bytes succeeds. When any chunk would push the
/// total strictly above `limit`, [`CollectLimitedError::TooLarge`] is returned
/// immediately without polling the stream again, so dropping it cancels the
/// producer. Source errors surface as [`CollectLimitedError::Source`].
pub async fn collect_limited<S, B, E>(
    chunks: S,
    limit: usize,
) -> Result<Bytes, CollectLimitedError<E>>
where
    S: Stream<Item = Result<B, E>>,
    B: Buf,
{
    let mut stream = pin!(chunks);
    let mut out = Vec::new();
    loop {
        let item = poll_fn(|cx| stream.as_mut().poll_next(cx)).await;
        match item {
            Some(Ok(mut chunk)) => {
                let remaining = chunk.remaining();
                if out.len() + remaining > limit {
                    return Err(CollectLimitedError::TooLarge { limit });
                }
                out.reserve(remaining);
                while chunk.has_remaining() {
                    let slice = chunk.chunk();
                    out.extend_from_slice(slice);
                    let copied = slice.len();
                    chunk.advance(copied);
                }
            }
            Some(Err(error)) => return Err(CollectLimitedError::Source(error)),
            None => return Ok(Bytes::from(out)),
        }
    }
}

/// Failure modes of [`collect_reqwest_limited`].
#[cfg(feature = "client")]
#[derive(Debug, thiserror::Error)]
pub enum ReqwestCollectError {
    /// The decoded response body would strictly exceed the configured limit.
    #[error("response body exceeds limit of {limit} bytes")]
    TooLarge {
        /// The configured limit in bytes.
        limit: usize,
    },
    /// The transport failed while streaming the body.
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
}

/// Collects a Reqwest response body under a byte limit.
///
/// Limits count DECODED bytes per section 30.2: when transparent decompression
/// is enabled on the client, content coding is removed below Reqwest, so a body
/// that decompresses beyond `limit` is rejected despite a smaller wire size.
///
/// Returns [`ReqwestCollectError::TooLarge`] immediately without polling the
/// stream again, cancelling the transfer.
#[cfg(feature = "client")]
pub async fn collect_reqwest_limited(
    response: reqwest::Response,
    limit: usize,
) -> Result<Bytes, ReqwestCollectError> {
    match collect_limited(response.bytes_stream(), limit).await {
        Ok(bytes) => Ok(bytes),
        Err(CollectLimitedError::TooLarge { limit }) => {
            Err(ReqwestCollectError::TooLarge { limit })
        }
        Err(CollectLimitedError::Source(error)) => Err(ReqwestCollectError::Transport(error)),
    }
}

/// Collects an Axum request body (from `axum::body::Body::into_data_stream()`)
/// under a byte limit with the same fail-fast semantics as [`collect_limited`].
#[cfg(feature = "server")]
pub async fn collect_body_limited(
    body: axum::body::BodyDataStream,
    limit: usize,
) -> Result<Bytes, CollectLimitedError<axum::Error>> {
    collect_limited(body, limit).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::task::Poll;

    fn chunks(
        items: Vec<&'static str>,
    ) -> impl Stream<Item = Result<Bytes, std::convert::Infallible>> {
        stream::iter(
            items
                .into_iter()
                .map(|item| Ok(Bytes::from_static(item.as_bytes()))),
        )
    }

    #[tokio::test]
    async fn collects_exact_concatenation_across_arbitrary_chunk_splits() {
        let collected = collect_limited(chunks(vec!["hel", "lo w", "o", "rld"]), 100).await;
        assert_eq!(
            collected.expect("under limit"),
            Bytes::from_static(b"hello world")
        );
        assert_eq!(
            collect_limited(chunks(Vec::new()), 10)
                .await
                .expect("empty stream"),
            Bytes::from_static(b"")
        );
    }

    #[tokio::test]
    async fn exactly_at_limit_succeeds() {
        let collected = collect_limited(chunks(vec!["abc", "def"]), 6).await;
        assert_eq!(collected.expect("at limit"), Bytes::from_static(b"abcdef"));
    }

    #[tokio::test]
    async fn one_byte_over_fails_without_polling_again() {
        struct PanickingStream {
            yielded: bool,
        }

        impl Stream for PanickingStream {
            type Item = Result<Bytes, std::convert::Infallible>;

            fn poll_next(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
            ) -> Poll<Option<Self::Item>> {
                if self.yielded {
                    panic!("stream polled after TooLarge");
                }
                self.get_mut().yielded = true;
                Poll::Ready(Some(Ok(Bytes::from_static(b"abcdefg"))))
            }
        }

        let error = collect_limited(PanickingStream { yielded: false }, 6).await;
        assert!(
            matches!(error, Err(CollectLimitedError::TooLarge { limit: 6 })),
            "expected TooLarge with limit 6, got {error:?}"
        );
    }

    #[tokio::test]
    async fn source_error_propagates_as_source() {
        #[derive(Debug, PartialEq, thiserror::Error)]
        #[error("transport down")]
        struct TransportDown;

        let items: Vec<Result<Bytes, TransportDown>> =
            vec![Ok(Bytes::from_static(b"ok")), Err(TransportDown)];
        let error = collect_limited(stream::iter(items), 100).await;
        assert!(
            matches!(error, Err(CollectLimitedError::Source(_))),
            "expected Source, got {error:?}"
        );
    }

    #[tokio::test]
    async fn zero_length_chunks_never_trip_the_limit() {
        let collected = collect_limited(chunks(vec!["ab", "", "cd", ""]), 4).await;
        assert_eq!(
            collected.expect("at limit with empty chunks"),
            Bytes::from_static(b"abcd")
        );
    }
}
