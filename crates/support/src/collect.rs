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

/// §30.2 / main spec §50 test 32: structured-body limits count DECODED
/// bytes. A one-shot loopback server answers with a gzipped JSON body whose
/// WIRE size is far below the configured limit while its DECOMPRESSED size
/// exceeds it; a gzip-enabled Reqwest client removes the content coding
/// beneath [`collect_reqwest_limited`], so the collector must reject the
/// response as `TooLarge` even though the wire transfer itself would fit.
#[cfg(all(test, feature = "client", feature = "client-gzip"))]
mod gzip_decoded_byte_tests {
    use super::*;

    /// `gzip --best` of `{"id":"w-1","name":"widget","blob":"aaa…a"}` (294
    /// decoded bytes; deterministic constant so no compression dependency is
    /// needed to build the fixture).
    const GZIP_JSON_WIRE: [u8; 56] = [
        31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 171, 86, 202, 76, 81, 178, 82, 42, 215, 53, 84, 210, 81,
        202, 75, 204, 77, 5, 113, 50, 83, 210, 83, 75, 128, 252, 164, 156, 252, 36, 32, 63, 113,
        132, 3, 165, 90, 0, 251, 232, 12, 232, 38, 1, 0, 0,
    ];
    /// Strictly between wire (56) and decoded (294) sizes.
    const LIMIT: usize = 128;

    /// Answers every connection with the same HTTP/1.1 response carrying the
    /// gzipped constant. Raw-socket on purpose: no axum/hyper server
    /// dependency is pulled into openapi-support for a test, and nothing
    /// recompresses or strips the hand-set `Content-Encoding` behind the
    /// collector's back. Multiple connections are fine — each test spins up
    /// its own ephemeral instance and issues as many requests as its proof
    /// needs.
    async fn serve_gzip() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let address = listener.local_addr().expect("local address");
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                // Drain request headers up to the empty line before answering.
                let mut seen = 0_usize;
                let mut scratch = [0_u8; 4096];
                loop {
                    let read = tokio::io::AsyncReadExt::read(&mut socket, &mut scratch)
                        .await
                        .expect("read request head");
                    seen += read;
                    if read == 0 || scratch[..seen].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     content-encoding: gzip\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n",
                    GZIP_JSON_WIRE.len()
                );
                use tokio::io::AsyncWriteExt;
                socket.write_all(head.as_bytes()).await.expect("write head");
                socket.write_all(&GZIP_JSON_WIRE).await.expect("write body");
                socket.shutdown().await.expect("shutdown");
            }
        });
        address
    }

    fn gzip_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .gzip(true)
            .build()
            .expect("gzip-capable client builds")
    }

    /// Control transport for the triangulation: Reqwest 0.12 removes codings
    /// BY DEFAULT whenever the matching cargo feature is active, so exposing
    /// the raw wire bytes requires explicitly opting OUT through
    /// `.gzip(false)`.
    fn plain_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .gzip(false)
            .build()
            .expect("plain client builds")
    }

    #[tokio::test]
    async fn limit_rejects_when_decoded_size_exceeds_it_despite_small_wire_size() {
        assert!(
            GZIP_JSON_WIRE.len() < LIMIT,
            "fixture invariant broken: wire must fit the limit"
        );
        let address = serve_gzip().await;
        let response = gzip_client()
            .get(format!("http://{address}/widgets/w-1"))
            .send()
            .await
            .expect("transport succeeds");
        assert_eq!(response.status(), 200);
        // Reqwest's transparent decoder CONSUMES `Content-Encoding` (and
        // `Content-Length`) off the response before returning it — visible
        // proof that collection happens ABOVE the decompression layer.
        assert_eq!(
            response.headers().get("content-encoding").map(|_| "gzip"),
            None,
            "decoder must strip the coding header it removed"
        );

        let error = collect_reqwest_limited(response, LIMIT)
            .await
            .expect_err("decoded size exceeds the limit despite the small wire size");
        assert!(
            matches!(error, ReqwestCollectError::TooLarge { limit: LIMIT }),
            "expected TooLarge at {LIMIT}, got {error:?}"
        );
    }

    /// The triangulating half of the §50-test-32 proof: with decoding OFF the
    /// SAME body fits `LIMIT` on the wire (collector observes exactly the
    /// gzipped constant), so the rejection above can only be explained by
    /// DECODED-byte accounting under a decompressing client.
    #[tokio::test]
    async fn wire_size_alone_fits_the_limit_so_rejection_proves_decoded_accounting() {
        assert!(
            GZIP_JSON_WIRE.len() < LIMIT,
            "fixture invariant broken: wire must fit the limit"
        );
        let address = serve_gzip().await;
        let response = plain_client()
            .get(format!("http://{address}/widgets/w-1"))
            .send()
            .await
            .expect("transport succeeds");
        assert_eq!(
            response
                .headers()
                .get("content-encoding")
                .and_then(|value| value.to_str().ok()),
            Some("gzip"),
            "without opt-in the coding must still be on the wire"
        );

        let collected = collect_reqwest_limited(response, LIMIT)
            .await
            .expect("wire transfer fits the limit when no coding is removed");
        assert_eq!(collected.len(), GZIP_JSON_WIRE.len());
        assert!(
            collected.starts_with(&[0x1f, 0x8b]),
            "raw gzip magic proves the collector saw WIRE bytes: {collected:?}"
        );
    }

    #[tokio::test]
    async fn collection_yields_the_decompressed_payload_under_a_fitting_limit() {
        let address = serve_gzip().await;
        let response = gzip_client()
            .get(format!("http://{address}/widgets/w-1"))
            .send()
            .await
            .expect("transport succeeds");

        let decoded = collect_reqwest_limited(response, 512)
            .await
            .expect("decoded payload fits 512");
        assert_eq!(decoded.len(), 294, "collector must observe DECODED bytes");
        assert!(
            decoded.starts_with(b"{\"id\":\"w-1\",\"name\":\"widget\""),
            "transparent decompression must yield the original JSON: {decoded:?}"
        );
    }
}
