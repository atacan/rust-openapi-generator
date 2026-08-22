//! Pre-handler protocol rejections (main spec §39).
//!
//! Rejections live outside the documented operation response enum: the router
//! emits them directly and the handler never observes them (§39 rule 1). The
//! default wire form is the canonical status with an empty body (§39 rule 3);
//! canned problem documents are a later generator-config concern.

use std::borrow::Cow;

/// Category of a pre-handler validation failure (main spec §39).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionKind {
    /// Path/query/header parameter syntax or missing required parameter.
    InvalidParameter,
    /// Syntactically invalid JSON/form/multipart framing.
    MalformedBody,
    /// Well-formed body failing schema validation.
    SchemaViolation,
    /// Bounded collection limit exceeded.
    BodyTooLarge,
    /// Missing, unparsable, wildcard, or unmatched `Content-Type`.
    UnsupportedMediaType,
    /// Request `Content-Encoding` other than absent/`identity` (section 30.4).
    UnsupportedContentCoding,
}

impl RejectionKind {
    /// Canonical status for this kind per the §39 mapping table.
    #[must_use]
    pub fn status(self) -> http::StatusCode {
        match self {
            Self::InvalidParameter | Self::MalformedBody => http::StatusCode::BAD_REQUEST,
            Self::SchemaViolation => http::StatusCode::UNPROCESSABLE_ENTITY,
            Self::BodyTooLarge => http::StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType | Self::UnsupportedContentCoding => {
                http::StatusCode::UNSUPPORTED_MEDIA_TYPE
            }
        }
    }
}

/// Pre-handler protocol rejection reported outside the documented response
/// enum (main spec §39).
///
/// `detail` is diagnostic material for logs/observation; it is never written
/// to the wire by [`IntoResponse`](axum::response::IntoResponse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolRejection {
    pub kind: RejectionKind,
    pub detail: Option<Cow<'static, str>>,
}

impl ProtocolRejection {
    #[must_use]
    pub fn new(kind: RejectionKind) -> Self {
        Self { kind, detail: None }
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<Cow<'static, str>>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Canonical status for this rejection per the §39 mapping table.
    #[must_use]
    pub fn status(&self) -> http::StatusCode {
        self.kind.status()
    }
}

#[cfg(feature = "server")]
impl axum::response::IntoResponse for ProtocolRejection {
    /// Emits only the canonical status with an empty body (§39 rule 3);
    /// documented body types are never synthesized (§39 rule 1).
    fn into_response(self) -> axum::response::Response {
        (self.status(), axum::body::Body::empty()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_maps_to_its_canonical_status() {
        assert_eq!(
            RejectionKind::InvalidParameter.status(),
            http::StatusCode::BAD_REQUEST
        );
        assert_eq!(
            RejectionKind::MalformedBody.status(),
            http::StatusCode::BAD_REQUEST
        );
        assert_eq!(
            RejectionKind::SchemaViolation.status(),
            http::StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            RejectionKind::BodyTooLarge.status(),
            http::StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            RejectionKind::UnsupportedMediaType.status(),
            http::StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            RejectionKind::UnsupportedContentCoding.status(),
            http::StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }

    #[test]
    fn constructors_set_kind_and_optional_detail() {
        let bare = ProtocolRejection::new(RejectionKind::SchemaViolation);
        assert_eq!(bare.kind, RejectionKind::SchemaViolation);
        assert_eq!(bare.detail, None);
        assert_eq!(bare.status(), http::StatusCode::UNPROCESSABLE_ENTITY);

        let detailed = bare.with_detail("missing required field `id`");
        assert_eq!(detailed.kind, RejectionKind::SchemaViolation);
        assert_eq!(
            detailed.detail.as_deref(),
            Some("missing required field `id`")
        );

        let owned = ProtocolRejection::new(RejectionKind::BodyTooLarge)
            .with_detail(String::from("limit is 8 MiB"));
        assert_eq!(owned.detail.as_deref(), Some("limit is 8 MiB"));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn into_response_emits_only_the_canonical_status_and_an_empty_body() {
        use axum::body::to_bytes;
        use axum::response::IntoResponse;

        for kind in [
            RejectionKind::InvalidParameter,
            RejectionKind::MalformedBody,
            RejectionKind::SchemaViolation,
            RejectionKind::BodyTooLarge,
            RejectionKind::UnsupportedMediaType,
            RejectionKind::UnsupportedContentCoding,
        ] {
            let rejection =
                ProtocolRejection::new(kind).with_detail("diagnostic text must stay off the wire");
            let response = rejection.into_response();
            assert_eq!(response.status(), kind.status(), "{kind:?}");
            let bytes = to_bytes(response.into_body(), 1024)
                .await
                .expect("empty body reads back");
            assert!(bytes.is_empty(), "body must be empty for {kind:?}");
        }
    }
}
