//! Single authoritative client error type (main spec §36).
//!
//! §36 is the only definition of [`ClientError`]: generated code never invents
//! ad-hoc variants outside it (DECISIONS D-impl-clienterror-location). A
//! documented response status is an enum variant, never a `ClientError`; these
//! variants cover failures that prevent the caller from obtaining any
//! documented response value.

/// Which transfer direction hit a byte limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyLimitDirection {
    /// Request serialization exceeded `structured_encode_bytes` (section 34.2).
    Encode,
    /// Response collection exceeded a decode limit.
    Decode,
}

impl std::fmt::Display for BodyLimitDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode => f.write_str("encode"),
            Self::Decode => f.write_str("decode"),
        }
    }
}

/// Failures that prevent obtaining a documented response value (main spec §36,
/// the single authoritative definition).
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Transport-level failure while sending the request or receiving the
    /// response.
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    /// The configured base URL or built request URL failed validation.
    #[error("invalid request URL: {0}")]
    InvalidUrl(String),
    /// A bounded body limit was exceeded in the given direction.
    #[error("{direction} body exceeds the limit of {limit} bytes")]
    BodyTooLarge {
        /// Transfer direction that hit the limit.
        direction: BodyLimitDirection,
        /// Configured limit in bytes.
        limit: usize,
    },
    /// A redirect was encountered while a one-shot streaming body was
    /// partially sent and cannot be replayed (§30.1).
    #[error("redirect requires a replayable request body")]
    RedirectRequiresReplayableBody,
    /// Response body decoding failed; the charset policy of §28.4 surfaces
    /// here per DECISIONS D-impl-charset-rejection.
    #[error("failed to decode response body")]
    Decode {
        /// Response content type when known.
        content_type: Option<mime::Mime>,
        /// Underlying decode failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A required request header was not supplied.
    #[error("missing required header {name}")]
    MissingRequiredHeader {
        /// Name of the missing header.
        name: http::HeaderName,
    },
    /// A request header value failed typed serialization.
    #[error("invalid value for header {name}")]
    InvalidHeader {
        /// Name of the rejected header.
        name: http::HeaderName,
        /// Underlying serialization failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The response content type matched no documented entry for the status
    /// (§28 precedence exhausted without a match).
    #[error("unexpected response content type: expected one of {expected:?}, got {actual:?}")]
    UnexpectedContentType {
        /// Documented media types considered acceptable.
        expected: Vec<String>,
        /// Content type actually received, when parseable.
        actual: Option<mime::Mime>,
    },
    /// The response status matches no documented variant, range, or `default`
    /// (§36: a documented status is a variant, never this error).
    #[error("undocumented response status {status}")]
    UndocumentedStatus {
        /// Status received on the wire.
        status: http::StatusCode,
    },
    /// The response `Content-Type` header was syntactically unparseable
    /// (§28.1: never ignored or defaulted).
    #[error(transparent)]
    MalformedContentType(#[from] crate::mediatype::MalformedContentType),
}

impl From<crate::collect::ReqwestCollectError> for ClientError {
    fn from(error: crate::collect::ReqwestCollectError) -> Self {
        match error {
            crate::collect::ReqwestCollectError::TooLarge { limit } => Self::BodyTooLarge {
                direction: BodyLimitDirection::Decode,
                limit,
            },
            crate::collect::ReqwestCollectError::Transport(source) => Self::Transport(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reqwest_collect_error_maps_to_decode_direction_body_too_large() {
        let mapped = ClientError::from(crate::collect::ReqwestCollectError::TooLarge { limit: 42 });
        assert!(
            matches!(
                mapped,
                ClientError::BodyTooLarge {
                    direction: BodyLimitDirection::Decode,
                    limit: 42
                }
            ),
            "got {mapped:?}"
        );
    }

    #[test]
    fn malformed_content_type_converts_via_from() {
        let mapped =
            ClientError::from(crate::mediatype::parse_content_type("nonsense").unwrap_err());
        assert!(matches!(mapped, ClientError::MalformedContentType(_)));
    }
}
