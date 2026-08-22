//! Identity-only inbound content coding policy (main spec §30.4).
//!
//! The generated router calls this guard **before any body byte is decoded**
//! (ordering lives in generated routers); any non-identity request
//! `Content-Encoding` yields a `415` `ProtocolRejection`
//! (`UnsupportedContentCoding`), closing the request-direction decompression
//! path entirely.

use crate::rejection::ProtocolRejection;
use crate::rejection::RejectionKind;

/// Accepts a request only when its `Content-Encoding` is absent or exactly
/// `identity` (case-insensitive per HTTP token semantics, surrounding OWS
/// allowed).
///
/// Rejects when any header value contains another coding (`identity, gzip`),
/// when more than one `Content-Encoding` header line is present even if each
/// reads `identity`, or when a value is not valid UTF-8.
pub fn ensure_identity_content_coding(headers: &http::HeaderMap) -> Result<(), ProtocolRejection> {
    let mut lines = headers.get_all(http::header::CONTENT_ENCODING).iter();
    let Some(first) = lines.next() else {
        return Ok(());
    };
    if lines.next().is_some() {
        return Err(rejection("multiple Content-Encoding header lines"));
    }
    let text = first
        .to_str()
        .map_err(|_| rejection("Content-Encoding is not valid UTF-8"))?;
    for token in text.split(',') {
        let token = token.trim();
        if !token.eq_ignore_ascii_case("identity") {
            return Err(rejection("request Content-Encoding other than identity"));
        }
    }
    Ok(())
}

fn rejection(detail: &'static str) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::UnsupportedContentCoding).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_str(value).expect("header value"),
        );
        headers
    }

    #[test]
    fn absent_and_identity_codings_are_accepted() {
        assert_eq!(
            ensure_identity_content_coding(&http::HeaderMap::new()),
            Ok(())
        );
        assert_eq!(
            ensure_identity_content_coding(&headers_with("identity")),
            Ok(())
        );
        assert_eq!(
            ensure_identity_content_coding(&headers_with("IDENTITY")),
            Ok(())
        );
        assert_eq!(
            ensure_identity_content_coding(&headers_with(" Identity ")),
            Ok(())
        );
    }

    #[test]
    fn any_other_coding_rejects_with_unsupported_content_coding() {
        for value in ["gzip", "identity, gzip", "deflate", "", "identical"] {
            let rejection = ensure_identity_content_coding(&headers_with(value)).expect_err(value);
            assert_eq!(rejection.kind, RejectionKind::UnsupportedContentCoding);
            assert_eq!(
                rejection.status(),
                http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{value} must yield 415"
            );
        }
    }

    #[test]
    fn multiple_header_lines_reject_even_when_each_reads_identity() {
        let mut headers = http::HeaderMap::new();
        headers.append(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_static("identity"),
        );
        headers.append(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_static("identity"),
        );
        let rejection = ensure_identity_content_coding(&headers).expect_err("two lines");
        assert_eq!(rejection.kind, RejectionKind::UnsupportedContentCoding);
    }

    #[test]
    fn non_utf8_header_value_rejects() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::CONTENT_ENCODING,
            http::HeaderValue::from_bytes(b"\xff").expect("raw bytes form a header value"),
        );
        assert!(
            ensure_identity_content_coding(&headers).is_err(),
            "undecodable value cannot be identity"
        );
    }
}
