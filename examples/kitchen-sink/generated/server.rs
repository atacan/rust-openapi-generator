/// Axum server generated from the OpenAPI document (main spec §8 Output B).
///
/// Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form extractor is never used — routes self-decode after the §28 Content-Type dispatch), streaming raw payloads (§32), typed documented response headers (§15: IntoResponse converts stored domain values through the well-defined internal error path of §48, firing the encode hook and emitting the fixed empty 500 on failure), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
///
/// Directional views (companion §5, main spec §50 test 50): request bodies decode into `<M>Write` (required write-only fields are mandatory there; required read-only fields are structurally absent and surplus keys are ignored unless a schema declares `additionalProperties: false`), response payloads carry `<M>Read` (write-only fields never reach the wire), and decoded request views run `validate_request()` before the handler. Recorded trait contract: when `<M>Write` reconstructs the shared model losslessly the router converts before invoking the trait; otherwise the trait takes the view itself.
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{
    Account,
    Ack,
    CreateSessionForm,
    CreateWidget,
    Document,
    DocumentMetadata,
    Event,
    FullWidget,
    MatrixRecord,
    Metric,
    Pet,
    ProblemDetails,
    Record,
    Session,
    SuccessEnvelope,
    ThumbnailMetadata,
    Widget,
};
use super::views::{
    AccountRead, AccountWrite, AuditEntryRead, SyncedRecordRead, SyncedRecordWrite,
};
use ::axum::response::IntoResponse;
use ::openapi_support::collect::{collect_body_limited, CollectLimitedError};
use ::openapi_support::content_coding::ensure_identity_content_coding;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::form::decode_form_limited;
use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use ::openapi_support::jsonseq::decode_jsonseq;
use ::openapi_support::jsonseq::encode_jsonseq_item;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::{
    is_wildcard_incoming, match_entry, parse_content_type, EntryMatch, ParsedMediaType,
};
use ::openapi_support::multipart::{
    extract_boundary, stream_multipart, MultipartError, MultipartEvent, MultipartLimits,
};
use ::openapi_support::ndjson::encode_ndjson_item;
use ::openapi_support::params::{decode_path_segment, ParamSpec, ParamStyle, ParamValue};
use ::openapi_support::peek::{detect_body_presence, BodyPresence};
use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};
use ::openapi_support::sse::encode_sse_event;
use ::openapi_support::stream_errors::JsonSeqDecodeError;
use ::openapi_support::stream_errors::ServerStreamError;
use ::std::collections::HashMap;

/// Boxed erased item stream shared by every generated streaming                  wrapper (§18–§20): producers box their producer stream into                  this type.
#[doc(hidden)]
pub type ErasedItems<T> =
    ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>;

/// Documented outcomes for `create_widget` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateWidgetResponse {
    /// HTTP 201 Created.
    Created201(Widget),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Bounded encoder for [`CreateWidgetResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl CreateWidgetResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "createWidget",
                "Created201",
            ),
            Self::BadRequest400(value) => encode_json_limited(
                ::http::StatusCode::BAD_REQUEST,
                "application/problem+json",
                &value,
                limits,
                hook,
                "createWidget",
                "BadRequest400",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for CreateWidgetResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Payload for status 201 of `create_session`: typed payload (main spec §15 Output B): required documented headers as plain fields, optional ones as `Option<T>`, then the body stored as a domain value for the bounded encoder.
#[derive(Debug)]
pub struct CreateSession201 {
    /// Documented response header `Location` (required).
    pub location: String,
    /// Documented response header `ETag` (optional).
    pub e_tag: Option<String>,
    pub body: Session,
}

/// Documented outcomes for `create_session` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateSessionResponse {
    /// HTTP 201 Created.
    Created201(CreateSession201),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Bounded encoder for [`CreateSessionResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl CreateSessionResponse {
    /// Encodes the documented outcome with the configured limits.
    #[allow(clippy::vec_init_then_push)]
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(wrapper) => {
                let mut typed_headers = Vec::<(&'static str, String)>::new();
                typed_headers.push(("location", wrapper.location.clone()));
                if let Some(value) = wrapper.e_tag.as_ref() {
                    typed_headers.push(("etag", value.to_owned()));
                }
                let encoded = encode_json_limited(
                    ::http::StatusCode::CREATED,
                    "application/json",
                    &wrapper.body,
                    limits,
                    hook,
                    "createSession",
                    "Created201",
                );
                write_typed_headers(encoded, hook, "createSession", "Created201", &typed_headers)
            }
            Self::Unauthorized401(value) => encode_json_limited(
                ::http::StatusCode::UNAUTHORIZED,
                "application/problem+json",
                &value,
                limits,
                hook,
                "createSession",
                "Unauthorized401",
            ),
        }
    }
}

/// Checked payload constructors validating every convertible documented header eagerly (main spec §15/§48).
/// Checked constructor for [`CreateSession201`] (§48).
impl CreateSession201 {
    pub fn new(
        location: String,
        e_tag: Option<String>,
        body: Session,
    ) -> Result<Self, ::openapi_support::response_headers::InvalidResponseHeader> {
        ::openapi_support::response_headers::checked_value("location", &location)?;
        if let Some(value) = e_tag.as_ref() {
            ::openapi_support::response_headers::checked_value("etag", value)?;
        }
        Ok(Self {
            location,
            e_tag,
            body,
        })
    }
}

impl ::axum::response::IntoResponse for CreateSessionResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `put_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutNoteResponse {
    /// HTTP 204 NoContent.
    NoContent204,
}

/// Bounded encoder for [`PutNoteResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PutNoteResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        _limits: &BodyLimits,
        _hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::NoContent204 => ::http::StatusCode::NO_CONTENT.into_response(),
        }
    }
}

impl ::axum::response::IntoResponse for PutNoteResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `put_object` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutObjectResponse {
    /// HTTP 201 Created.
    Created201,
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Bounded encoder for [`PutObjectResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PutObjectResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201 => ::http::StatusCode::CREATED.into_response(),
            Self::BadRequest400(value) => encode_json_limited(
                ::http::StatusCode::BAD_REQUEST,
                "application/problem+json",
                &value,
                limits,
                hook,
                "putObject",
                "BadRequest400",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for PutObjectResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Payload for status 200 of `get_object` (main spec §32): the body streams verbatim; typed documented-header fields ride beside it where documented (main spec §15/§32).
#[derive(Debug)]
pub struct GetObject200 {
    /// Documented response header `ETag` (optional).
    pub e_tag: Option<String>,
    /// Documented response header `Content-Length` (optional).
    pub content_length: Option<i64>,
    pub body: ::axum::body::Body,
}

/// Documented outcomes for `get_object` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetObjectResponse {
    /// HTTP 200 Ok.
    Ok200(GetObject200),
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Bounded encoder for [`GetObjectResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl GetObjectResponse {
    /// Encodes the documented outcome with the configured limits.
    #[allow(clippy::vec_init_then_push)]
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(wrapper) => {
                let mut typed_headers = Vec::<(&'static str, String)>::new();
                if let Some(value) = wrapper.e_tag.as_ref() {
                    typed_headers.push(("etag", value.to_owned()));
                }
                if let Some(value) = wrapper.content_length.as_ref() {
                    typed_headers.push(("content-length", value.to_string()));
                }
                let encoded = stream_response(
                    ::http::StatusCode::OK,
                    "application/octet-stream",
                    wrapper.body,
                );
                write_typed_headers(encoded, hook, "getObject", "Ok200", &typed_headers)
            }
            Self::NotFound404(value) => encode_json_limited(
                ::http::StatusCode::NOT_FOUND,
                "application/problem+json",
                &value,
                limits,
                hook,
                "getObject",
                "NotFound404",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for GetObjectResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented representations for status 200 of `get_thumbnail` (main spec §11): the router selects through Content-Type matching (§28).
#[derive(Debug)]
pub enum GetThumbnail200Content {
    Json(ThumbnailMetadata),
    Any {
        content_type: ::mime::Mime,
        body: ::axum::body::Body,
    },
}

/// Documented outcomes for `get_thumbnail` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetThumbnailResponse {
    /// HTTP 200 Ok.
    Ok200(GetThumbnail200Content),
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Bounded encoder for [`GetThumbnailResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl GetThumbnailResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(content) => match content {
                GetThumbnail200Content::Json(value) => encode_json_limited(
                    ::http::StatusCode::OK,
                    "application/json",
                    &value,
                    limits,
                    hook,
                    "getThumbnail",
                    "Ok200",
                ),
                GetThumbnail200Content::Any { content_type, body } => {
                    any_response(::http::StatusCode::OK, content_type, body)
                }
            },
            Self::NotFound404(value) => encode_json_limited(
                ::http::StatusCode::NOT_FOUND,
                "application/problem+json",
                &value,
                limits,
                hook,
                "getThumbnail",
                "NotFound404",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for GetThumbnailResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Live streaming view over binary part `file` of `upload_document` (main spec §17 Output B): `next_chunk` yields payload bytes with backpressure as they arrive; nothing aggregates the part.
/// Sequential semantics (§51.4): while this part is open the rest of the message cannot advance. Trailing parts flow through this type's tail scan instead of the pre-handler pass: declared scalar/JSON parts arriving behind the stream decode bounded onto `trailing_parts`, required wire names the clean end-of-message still lacks surface exactly one terminal SchemaViolation from `next_chunk` (§17.1 enforced on wire arrival), duplicate single-valued reopenings reject, every observed name is recorded for `unknown_part_names`, and all remaining payloads drain without buffering.
pub struct UploadDocumentFilePart {
    pub file_name: Option<String>,
    pub content_type: Option<::mime::Mime>,
    /// Scalar/JSON parts that arrived BEHIND this streaming part, decoded bounded as their boundaries closed (§38 application-owned tail); their pre-handoff siblings live on [`UploadDocumentMultipartInput`].
    pub trailing_parts: UploadDocumentTrailingParts,
    events: MultipartEvents,
    log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,
    stage: UploadDocumentFilePartTailStage,
    scalar_limit: usize,
    buffer: Vec<u8>,
    finished: bool,
    /// Required scalar/JSON wire names still unseen at the streaming handoff (§17.1): satisfied by trailing arrivals, otherwise reported once at the clean end-of-message.
    pending_required: Vec<&'static str>,
    /// Declared SINGLE-VALUED wire names already consumed anywhere before now (pre-handoff or behind the stream): any reopening violates §17.1.
    seen_single_valued: Vec<String>,
}

/// Tail-scan stages of [`UploadDocumentFilePart`] (§51.4 sequential semantics): `Idle` delivers payload chunks to the application; `Drain` discards them; the remaining stages bounded-buffer one trailing scalar/JSON part behind the stream.
#[derive(Debug, Clone, Copy)]
enum UploadDocumentFilePartTailStage {
    Idle,
    Drain,
    Metadata,
    TagsElement,
}

/// Scalar/JSON parts observed BEHIND [`UploadDocumentFilePart`] (main spec §17 Output B): decoded bounded as their boundaries closed. Parts consumed BEFORE the streaming handoff live on [`UploadDocumentMultipartInput`]; the split mirrors wire arrival (§38 application-owned tail).
#[derive(Debug, Default)]
pub struct UploadDocumentTrailingParts {
    /// JSON part `metadata`.
    pub metadata: Option<DocumentMetadata>,
    /// Textual part `tags`; repeated parts collect in wire order.
    pub tags: Vec<String>,
}

impl UploadDocumentFilePart {
    /// Advances to the next payload chunk of this part (`None` at its clean end). Violations encountered while scanning trailing parts surface here as protocol rejections because sequential streaming cannot validate them any earlier; at the clean end-of-message, required parts still pending produce exactly one terminal SchemaViolation naming them (§17.1) and later calls keep returning `None`.
    #[allow(clippy::missing_errors_doc)]
    pub async fn next_chunk(&mut self) -> Result<Option<::bytes::Bytes>, ProtocolRejection> {
        if self.finished {
            return Ok(None);
        }
        loop {
            let event = match next_multipart_event(&mut self.events).await {
                Some(event) => event,
                None => {
                    self.finished = true;
                    if !self.pending_required.is_empty() {
                        let names = self.pending_required.join("`, `");
                        return Err(schema_violation(format!(
                            "missing required part(s) `{names}`",
                        )));
                    }
                    return Ok(None);
                }
            };
            match event {
                Err(error) => return Err(multipart_rejection(&error)),
                Ok(MultipartEvent::PartBegin(headers)) => {
                    if self.seen_single_valued.contains(&headers.name) {
                        return Err(schema_violation(format!(
                            "duplicate single-valued part `{}` after the streaming part",
                            headers.name
                        )));
                    }
                    match headers.name.as_str() {
                        "metadata" => {
                            self.seen_single_valued.push("metadata".to_owned());
                            self.pending_required.retain(|name| *name != "metadata");
                            self.buffer.clear();
                            self.stage = UploadDocumentFilePartTailStage::Metadata;
                        }
                        "tags" => {
                            self.buffer.clear();
                            self.stage = UploadDocumentFilePartTailStage::TagsElement;
                        }
                        other => {
                            multipart_record_unknown(&self.log, other);
                            self.stage = UploadDocumentFilePartTailStage::Drain;
                        }
                    }
                }
                Ok(MultipartEvent::PartChunk(chunk)) => match self.stage {
                    UploadDocumentFilePartTailStage::Idle => return Ok(Some(chunk)),
                    UploadDocumentFilePartTailStage::Drain => {}
                    UploadDocumentFilePartTailStage::Metadata
                    | UploadDocumentFilePartTailStage::TagsElement => {
                        self.buffer.extend_from_slice(&chunk);
                        if self.buffer.len() > self.scalar_limit {
                            return Err(ProtocolRejection::new(RejectionKind::BodyTooLarge));
                        }
                    }
                },
                Ok(MultipartEvent::PartEnd) => match self.stage {
                    UploadDocumentFilePartTailStage::Idle
                    | UploadDocumentFilePartTailStage::Drain => {}
                    UploadDocumentFilePartTailStage::Metadata => {
                        let value: DocumentMetadata = decode_json_body(&self.buffer)?;
                        self.trailing_parts.metadata = Some(value);
                        self.stage = UploadDocumentFilePartTailStage::Idle;
                    }
                    UploadDocumentFilePartTailStage::TagsElement => {
                        let value = multipart_scalar_text::<String>("tags", &self.buffer)?;
                        self.trailing_parts.tags.push(value);
                        self.stage = UploadDocumentFilePartTailStage::Idle;
                    }
                },
            }
        }
    }
}

/// Streaming multipart input for `upload_document` (main spec §17 Output B): scalar/JSON parts were bounded-buffered and decoded during the router's single incremental pass up to the streaming handoff; binary parts stay live streams over the request body.
/// Required-part enforcement is wire-arrival-based (§17.1, §38): parts arriving BEFORE the first binary validate pre-handler in that pass; parts arriving behind the live stream decode onto its `trailing_parts`, and required names a clean end-of-message never delivers reject through the live part's terminal error instead of a pre-handler rejection.
pub struct UploadDocumentMultipartInput {
    /// JSON part `metadata`: `Some` when it arrived before the streaming part; otherwise decoded onto the live part's `trailing_parts`.
    pub metadata: Option<DocumentMetadata>,
    /// Textual part `tags`; repeated parts collect in wire order.
    pub tags: Vec<String>,
    /// Streaming binary part `file`.
    pub file: UploadDocumentFilePart,
    unknown_log: ::std::sync::Arc<::std::sync::Mutex<MultipartUnknownLog>>,
}

impl UploadDocumentMultipartInput {
    /// Wire names of every unrecognized or late-arriving part observed so far (§17.1 unknown-fields-ignore default): their payloads stream past without buffering and never reject. Names behind a streaming part appear once the application drains it through `next_chunk`.
    pub fn unknown_part_names(&self) -> Vec<String> {
        match self.unknown_log.lock() {
            Ok(guard) => guard.names.clone(),
            Err(poisoned) => poisoned.into_inner().names.clone(),
        }
    }
}

/// Payload for status 201 of `upload_document`: typed payload (main spec §15 Output B): required documented headers as plain fields, optional ones as `Option<T>`, then the body stored as a domain value for the bounded encoder.
#[derive(Debug)]
pub struct UploadDocument201 {
    /// Documented response header `Location` (required).
    pub location: String,
    pub body: Document,
}

/// Documented outcomes for `upload_document` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum UploadDocumentResponse {
    /// HTTP 201 Created.
    Created201(UploadDocument201),
}

/// Bounded encoder for [`UploadDocumentResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl UploadDocumentResponse {
    /// Encodes the documented outcome with the configured limits.
    #[allow(clippy::vec_init_then_push)]
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(wrapper) => {
                let mut typed_headers = Vec::<(&'static str, String)>::new();
                typed_headers.push(("location", wrapper.location.clone()));
                let encoded = encode_json_limited(
                    ::http::StatusCode::CREATED,
                    "application/json",
                    &wrapper.body,
                    limits,
                    hook,
                    "uploadDocument",
                    "Created201",
                );
                write_typed_headers(
                    encoded,
                    hook,
                    "uploadDocument",
                    "Created201",
                    &typed_headers,
                )
            }
        }
    }
}

/// Checked payload constructors validating every convertible documented header eagerly (main spec §15/§48).
/// Checked constructor for [`UploadDocument201`] (§48).
impl UploadDocument201 {
    pub fn new(
        location: String,
        body: Document,
    ) -> Result<Self, ::openapi_support::response_headers::InvalidResponseHeader> {
        ::openapi_support::response_headers::checked_value("location", &location)?;
        Ok(Self { location, body })
    }
}

impl ::axum::response::IntoResponse for UploadDocumentResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Erased `sse` item stream for status 200 of `stream_events` (main spec §18 Output B / §40): failures after commit ride `ServerStreamError` — no fabricated statuses.
pub type StreamEvents200Stream = ErasedItems<Result<Event, ServerStreamError>>;

/// Documented outcomes for `stream_events` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
pub enum StreamEventsResponse {
    /// HTTP 200 Ok.
    Ok200(StreamEvents200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Bounded encoder for [`StreamEventsResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl StreamEventsResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
        stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(items) => {
                let mut encoded = ::http::StatusCode::OK.into_response();
                encoded.headers_mut().insert(
                    ::http::header::CONTENT_TYPE,
                    ::http::HeaderValue::from_static("text/event-stream"),
                );
                *encoded.body_mut() = ::axum::body::Body::from_stream(stream_body_encoder(
                    items,
                    limits.max_stream_record_bytes,
                    ::std::sync::Arc::clone(&stream_failure_hook),
                    "streamEvents",
                    encode_sse_event::<Event>,
                ));
                encoded
            }
            Self::Unauthorized401(value) => encode_json_limited(
                ::http::StatusCode::UNAUTHORIZED,
                "application/problem+json",
                &value,
                limits,
                hook,
                "streamEvents",
                "Unauthorized401",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for StreamEventsResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(
            &BodyLimits::process_default(),
            &NoOpEncodeOverflowHook,
            ::std::sync::Arc::new(::openapi_support::hooks::NoOpStreamFailureHook),
        )
    }
}

/// Erased `ndjson` item stream for status 200 of `export_records` (main spec §19 Output B / §40): failures after commit ride `ServerStreamError` — no fabricated statuses.
pub type ExportRecords200Stream = ErasedItems<Result<Record, ServerStreamError>>;

/// Documented outcomes for `export_records` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
pub enum ExportRecordsResponse {
    /// HTTP 200 Ok.
    Ok200(ExportRecords200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Bounded encoder for [`ExportRecordsResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl ExportRecordsResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
        stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(items) => {
                let mut encoded = ::http::StatusCode::OK.into_response();
                encoded.headers_mut().insert(
                    ::http::header::CONTENT_TYPE,
                    ::http::HeaderValue::from_static("application/x-ndjson"),
                );
                *encoded.body_mut() = ::axum::body::Body::from_stream(stream_body_encoder(
                    items,
                    limits.max_stream_record_bytes,
                    ::std::sync::Arc::clone(&stream_failure_hook),
                    "exportRecords",
                    encode_ndjson_item::<Record>,
                ));
                encoded
            }
            Self::Unauthorized401(value) => encode_json_limited(
                ::http::StatusCode::UNAUTHORIZED,
                "application/problem+json",
                &value,
                limits,
                hook,
                "exportRecords",
                "Unauthorized401",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for ExportRecordsResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(
            &BodyLimits::process_default(),
            &NoOpEncodeOverflowHook,
            ::std::sync::Arc::new(::openapi_support::hooks::NoOpStreamFailureHook),
        )
    }
}

/// Streaming jsonseq request input for `push_metrics` (main spec §6): `next_item` decodes one record at a time bounded by `max_stream_record_bytes`; decode failures reject per §39 (oversized → 413, malformed/truncated/non-UTF-8/aborted → 400) and nothing aggregates the body.
pub struct PushMetricsJsonSeqInput {
    stream: ErasedItems<Result<Metric, JsonSeqDecodeError>>,
}

impl PushMetricsJsonSeqInput {
    /// Wraps the raw body chunk stream behind the incremental decoder.
    fn new(chunks: ::axum::body::BodyDataStream, limit: usize) -> Self {
        Self {
            stream: Box::pin(decode_jsonseq::<Metric, _, _>(chunks, limit)),
        }
    }

    /// Next decoded item (`None` at the clean end-of-stream).
    #[allow(clippy::missing_errors_doc)]
    pub async fn next_item(&mut self) -> Result<Option<Metric>, ProtocolRejection> {
        let next = ::std::future::poll_fn(|cx| {
            ::futures_core::Stream::poll_next(self.stream.as_mut(), cx)
        })
        .await;
        match next {
            Some(Ok(item)) => Ok(Some(item)),
            Some(Err(error)) => Err(match error {
                JsonSeqDecodeError::RecordTooLarge { .. } => {
                    ProtocolRejection::new(RejectionKind::BodyTooLarge)
                }
                _ => malformed_body("jsonseq record failed to decode"),
            }),
            None => Ok(None),
        }
    }
}

/// Documented outcomes for `push_metrics` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PushMetricsResponse {
    /// HTTP 202 Accepted.
    Accepted202(Ack),
}

/// Bounded encoder for [`PushMetricsResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PushMetricsResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Accepted202(value) => encode_json_limited(
                ::http::StatusCode::ACCEPTED,
                "application/json",
                &value,
                limits,
                hook,
                "pushMetrics",
                "Accepted202",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for PushMetricsResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Erased `jsonseq` item stream for status 200 of `export_metrics` (main spec §20 Output B / §40): failures after commit ride `ServerStreamError` — no fabricated statuses.
pub type ExportMetrics200Stream = ErasedItems<Result<Metric, ServerStreamError>>;

/// Documented outcomes for `export_metrics` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
pub enum ExportMetricsResponse {
    /// HTTP 200 Ok.
    Ok200(ExportMetrics200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Bounded encoder for [`ExportMetricsResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl ExportMetricsResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
        stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(items) => {
                let mut encoded = ::http::StatusCode::OK.into_response();
                encoded.headers_mut().insert(
                    ::http::header::CONTENT_TYPE,
                    ::http::HeaderValue::from_static("application/json-seq"),
                );
                *encoded.body_mut() = ::axum::body::Body::from_stream(stream_body_encoder(
                    items,
                    limits.max_stream_record_bytes,
                    ::std::sync::Arc::clone(&stream_failure_hook),
                    "exportMetrics",
                    encode_jsonseq_item::<Metric>,
                ));
                encoded
            }
            Self::Unauthorized401(value) => encode_json_limited(
                ::http::StatusCode::UNAUTHORIZED,
                "application/problem+json",
                &value,
                limits,
                hook,
                "exportMetrics",
                "Unauthorized401",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for ExportMetricsResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(
            &BodyLimits::process_default(),
            &NoOpEncodeOverflowHook,
            ::std::sync::Arc::new(::openapi_support::hooks::NoOpStreamFailureHook),
        )
    }
}

/// Payload for status 200 of `post_vendor_document` (main spec §32): the body streams verbatim; typed documented-header fields ride beside it where documented (main spec §15/§32).
#[derive(Debug)]
pub struct PostVendorDocument200 {
    pub body: ::axum::body::Body,
}

/// Documented outcomes for `post_vendor_document` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostVendorDocumentResponse {
    /// HTTP 200 Ok.
    Ok200(PostVendorDocument200),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Bounded encoder for [`PostVendorDocumentResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PostVendorDocumentResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(wrapper) => stream_response(
                ::http::StatusCode::OK,
                "application/vnd.acme.document-v7",
                wrapper.body,
            ),
            Self::BadRequest400(value) => encode_json_limited(
                ::http::StatusCode::BAD_REQUEST,
                "application/problem+json",
                &value,
                limits,
                hook,
                "postVendorDocument",
                "BadRequest400",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for PostVendorDocumentResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `probe_status` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum ProbeStatusResponse {
    /// HTTP 200 Ok.
    Ok200(Widget),
    /// Any HTTP 2XX success status.
    Success2xx {
        status: ::http::StatusCode,
        body: SuccessEnvelope,
    },
    /// Any HTTP 4XX client-error status.
    ClientError4xx {
        status: ::http::StatusCode,
        body: ProblemDetails,
    },
    /// Any other status (`default`).
    Default {
        status: ::http::StatusCode,
        body: ProblemDetails,
    },
}

/// A carried status fell outside its variant's documented range (main spec §48).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidStatusRange;

impl std::fmt::Display for InvalidStatusRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("status outside the variant's documented range")
    }
}

impl std::error::Error for InvalidStatusRange {}

/// Bounded encoder for [`ProbeStatusResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl ProbeStatusResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(value) => encode_json_limited(
                ::http::StatusCode::OK,
                "application/json",
                &value,
                limits,
                hook,
                "probeStatus",
                "Ok200",
            ),
            Self::Success2xx { status, body } => {
                debug_assert!(
                    (200..300).contains(&status.as_u16()),
                    "Success2xx carries a status outside 200..300",
                );
                encode_json_limited(
                    status,
                    "application/json",
                    &body,
                    limits,
                    hook,
                    "probeStatus",
                    "Success2xx",
                )
            }
            Self::ClientError4xx { status, body } => {
                debug_assert!(
                    (400..500).contains(&status.as_u16()),
                    "ClientError4xx carries a status outside 400..500",
                );
                encode_json_limited(
                    status,
                    "application/problem+json",
                    &body,
                    limits,
                    hook,
                    "probeStatus",
                    "ClientError4xx",
                )
            }
            Self::Default { status, body } => {
                debug_assert!(
                    status.as_u16() >= 200,
                    "Default carries an informational status",
                );
                encode_json_limited(
                    status,
                    "application/problem+json",
                    &body,
                    limits,
                    hook,
                    "probeStatus",
                    "Default",
                )
            }
        }
    }

    /// Checked constructors validating the carried status (main spec §24/§48); the `IntoResponse` path only asserts in debug builds.
    /// Checked constructor: `Success2xx` accepts only statuses inside 200..300.
    pub fn success_2xx(
        status: ::http::StatusCode,
        body: SuccessEnvelope,
    ) -> Result<Self, InvalidStatusRange> {
        if (200..300).contains(&status.as_u16()) {
            Ok(Self::Success2xx { status, body })
        } else {
            Err(InvalidStatusRange)
        }
    }
    /// Checked constructor: `ClientError4xx` accepts only statuses inside 400..500.
    pub fn client_error_4xx(
        status: ::http::StatusCode,
        body: ProblemDetails,
    ) -> Result<Self, InvalidStatusRange> {
        if (400..500).contains(&status.as_u16()) {
            Ok(Self::ClientError4xx { status, body })
        } else {
            Err(InvalidStatusRange)
        }
    }
    /// Checked constructor: `Default` accepts every status no other documented variant covers and no informational status (main spec §24/§35).
    pub fn default_status(
        status: ::http::StatusCode,
        body: ProblemDetails,
    ) -> Result<Self, InvalidStatusRange> {
        if status.as_u16() < 200 {
            return Err(InvalidStatusRange);
        }
        if status.as_u16() == 200 {
            return Err(InvalidStatusRange);
        }
        if (200..300).contains(&status.as_u16()) {
            return Err(InvalidStatusRange);
        }
        if (400..500).contains(&status.as_u16()) {
            return Err(InvalidStatusRange);
        }
        Ok(Self::Default { status, body })
    }
}

impl ::axum::response::IntoResponse for ProbeStatusResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `delete_task` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum DeleteTaskResponse {
    /// HTTP 204 NoContent.
    NoContent204,
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Bounded encoder for [`DeleteTaskResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl DeleteTaskResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::NoContent204 => ::http::StatusCode::NO_CONTENT.into_response(),
            Self::NotFound404(value) => encode_json_limited(
                ::http::StatusCode::NOT_FOUND,
                "application/problem+json",
                &value,
                limits,
                hook,
                "deleteTask",
                "NotFound404",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for DeleteTaskResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `get_widget` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetWidgetResponse {
    /// HTTP 200 Ok.
    Ok200(Widget),
    /// HTTP 404 NotFound.
    NotFound404,
}

/// Bounded encoder for [`GetWidgetResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl GetWidgetResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(value) => encode_json_limited(
                ::http::StatusCode::OK,
                "application/json",
                &value,
                limits,
                hook,
                "getWidget",
                "Ok200",
            ),
            Self::NotFound404 => ::http::StatusCode::NOT_FOUND.into_response(),
        }
    }
}

impl ::axum::response::IntoResponse for GetWidgetResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `head_widget` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum HeadWidgetResponse {
    /// HTTP 200 Ok.
    Ok200 {
        /// Documented response header `ETag` (required).
        e_tag: String,
        /// Documented response header `Content-Length` (required).
        content_length: i64,
    },
    /// HTTP 404 NotFound.
    NotFound404,
}

/// Bounded encoder for [`HeadWidgetResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl HeadWidgetResponse {
    /// Encodes the documented outcome with the configured limits.
    #[allow(clippy::vec_init_then_push)]
    pub fn into_response_with_limits(
        self,
        _limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200 {
                e_tag,
                content_length,
            } => {
                let mut typed_headers = Vec::<(&'static str, String)>::new();
                typed_headers.push(("etag", e_tag.clone()));
                typed_headers.push(("content-length", content_length.to_string()));
                let encoded = ::http::StatusCode::OK.into_response();
                write_typed_headers(encoded, hook, "headWidget", "Ok200", &typed_headers)
            }
            Self::NotFound404 => ::http::StatusCode::NOT_FOUND.into_response(),
        }
    }
}

impl ::axum::response::IntoResponse for HeadWidgetResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `echo_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum EchoNoteResponse {
    /// HTTP 200 Ok.
    Ok200(Option<String>),
}

/// Bounded encoder for [`EchoNoteResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl EchoNoteResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(value) => encode_json_limited(
                ::http::StatusCode::OK,
                "application/json",
                &value,
                limits,
                hook,
                "echoNote",
                "Ok200",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for EchoNoteResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `create_account` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateAccountResponse {
    /// HTTP 201 Created.
    Created201(AccountRead),
}

/// Bounded encoder for [`CreateAccountResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl CreateAccountResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "createAccount",
                "Created201",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for CreateAccountResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `list_audit_entries` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum ListAuditEntriesResponse {
    /// HTTP 200 Ok.
    Ok200(AuditEntryRead),
}

/// Bounded encoder for [`ListAuditEntriesResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl ListAuditEntriesResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(value) => encode_json_limited(
                ::http::StatusCode::OK,
                "application/json",
                &value,
                limits,
                hook,
                "listAuditEntries",
                "Ok200",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for ListAuditEntriesResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `sync_record` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum SyncRecordResponse {
    /// HTTP 200 Ok.
    Ok200(SyncedRecordRead),
}

/// Bounded encoder for [`SyncRecordResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl SyncRecordResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(value) => encode_json_limited(
                ::http::StatusCode::OK,
                "application/json",
                &value,
                limits,
                hook,
                "syncRecord",
                "Ok200",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for SyncRecordResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `create_pet` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreatePetResponse {
    /// HTTP 201 Created.
    Created201(FullWidget),
}

/// Bounded encoder for [`CreatePetResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl CreatePetResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "createPet",
                "Created201",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for CreatePetResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `create_record` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateRecordResponse {
    /// HTTP 201 Created.
    Created201(MatrixRecord),
}

/// Bounded encoder for [`CreateRecordResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl CreateRecordResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => encode_json_limited(
                ::http::StatusCode::CREATED,
                "application/json",
                &value,
                limits,
                hook,
                "createRecord",
                "Created201",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for CreateRecordResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `POST` `/widgets`.
    /// Operation `createWidget`.
    async fn create_widget(&self, body: CreateWidget) -> CreateWidgetResponse;

    /// `POST` `/sessions`.
    /// Operation `createSession`.
    async fn create_session(&self, body: CreateSessionForm) -> CreateSessionResponse;

    /// `PUT` `/notes/{id}`.
    /// Operation `putNote`.
    async fn put_note(&self, id: String, body: String) -> PutNoteResponse;

    /// `PUT` `/objects/{id}`.
    /// Operation `putObject`.
    async fn put_object(&self, id: String, body: ::axum::body::Body) -> PutObjectResponse;

    /// `GET` `/objects/{id}`.
    /// Operation `getObject`.
    async fn get_object(&self, id: String) -> GetObjectResponse;

    /// `GET` `/thumbnails/{id}`.
    /// Operation `getThumbnail`.
    async fn get_thumbnail(&self, id: String) -> GetThumbnailResponse;

    /// `POST` `/documents`.
    /// Operation `uploadDocument`.
    async fn upload_document(&self, body: UploadDocumentMultipartInput) -> UploadDocumentResponse;

    /// `GET` `/events`.
    /// Operation `streamEvents`.
    async fn stream_events(&self) -> StreamEventsResponse;

    /// `GET` `/records/export`.
    /// Operation `exportRecords`.
    async fn export_records(&self) -> ExportRecordsResponse;

    /// `POST` `/metrics`.
    /// Operation `pushMetrics`.
    async fn push_metrics(&self, body: PushMetricsJsonSeqInput) -> PushMetricsResponse;

    /// `GET` `/metrics/export`.
    /// Operation `exportMetrics`.
    async fn export_metrics(&self) -> ExportMetricsResponse;

    /// `POST` `/vendor-documents`.
    /// Operation `postVendorDocument`.
    async fn post_vendor_document(&self, body: ::axum::body::Body) -> PostVendorDocumentResponse;

    /// `GET` `/status-probes/{id}`.
    /// Operation `probeStatus`.
    async fn probe_status(&self, id: String) -> ProbeStatusResponse;

    /// `DELETE` `/tasks/{id}`.
    /// Operation `deleteTask`.
    async fn delete_task(&self, id: String) -> DeleteTaskResponse;

    /// `GET` `/widgets/{id}`.
    /// Operation `getWidget`.
    async fn get_widget(&self, id: String) -> GetWidgetResponse;

    /// `HEAD` `/widgets/{id}`.
    /// Operation `headWidget`.
    async fn head_widget(&self, id: String) -> HeadWidgetResponse;

    /// `POST` `/echo-note`.
    /// Operation `echoNote`.
    async fn echo_note(&self, body: Option<Option<String>>) -> EchoNoteResponse;

    /// `POST` `/accounts`.
    /// Operation `createAccount`.
    async fn create_account(&self, body: Account) -> CreateAccountResponse;

    /// `GET` `/audit/{id}`.
    /// Operation `listAuditEntries`.
    async fn list_audit_entries(&self, id: String) -> ListAuditEntriesResponse;

    /// `PUT` `/synced`.
    /// Operation `syncRecord`.
    async fn sync_record(&self, body: SyncedRecordWrite) -> SyncRecordResponse;

    /// `POST` `/pets`.
    /// Operation `createPet`.
    async fn create_pet(&self, body: Pet) -> CreatePetResponse;

    /// `POST` `/records`.
    /// Operation `createRecord`.
    async fn create_record(&self, body: MatrixRecord) -> CreateRecordResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
    stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
}

/// Route handler for `POST` `/widgets` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_create_widget(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.structured_request_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: CreateWidget = decode_json_body(&bytes)?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.create_widget(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/sessions` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_create_session(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body =
        match classify_request_entry(parsed.as_ref(), &["application/x-www-form-urlencoded"]) {
            RequestEntryMatch::AbsentContentType => {
                let probe = body_bytes(body, limits.structured_request_bytes).await?;
                if probe.is_empty() {
                    return Err(malformed_body("required request body arrived empty"));
                }
                return Err(unsupported_media_type(
                    "request arrived without a Content-Type",
                ));
            }
            RequestEntryMatch::Unmatched => {
                return Err(unsupported_media_type(
                    "no documented request media type matches the request Content-Type",
                ));
            }
            RequestEntryMatch::Entry(0) => {
                ensure_utf8_charset(parsed.as_ref())?;
                let bytes = body_bytes(body, limits.structured_request_bytes).await?;
                if bytes.is_empty() {
                    return Err(malformed_body("documented request body arrived empty"));
                }
                let value: CreateSessionForm =
                    decode_form_body(&bytes, limits.structured_request_bytes)?;
                value
            }
            RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
        };
    let api = __state.api.as_ref();
    let response = api.create_session(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `PUT` `/notes/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_put_note(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["text/plain"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.text_body_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.text_body_bytes).await?;
            decode_text_body(bytes)?
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.put_note(id, request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `PUT` `/objects/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_put_object(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/octet-stream"])
    {
        RequestEntryMatch::AbsentContentType => {
            let (presence, _replay) =
                detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;
            if matches!(presence, BodyPresence::Empty) {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => body,
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.put_object(id, request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `GET` `/objects/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_get_object(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let api = __state.api.as_ref();
    let response = api.get_object(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `GET` `/thumbnails/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_get_thumbnail(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let api = __state.api.as_ref();
    let response = api.get_thumbnail(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/documents` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_upload_document(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["multipart/form-data"]) {
        RequestEntryMatch::AbsentContentType => {
            let (presence, _replay) =
                detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;
            if matches!(presence, BodyPresence::Empty) {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            collect_upload_document_multipart(body, parsed.as_ref(), &limits).await?
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.upload_document(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Runs the §38 pre-handler pipeline for the `multipart/form-data` body of `upload_document` (main spec §5.5/§17/§17.1): one incremental pass up to the streaming handoff; scalar/JSON parts buffer only up to `multipart_scalar_part_bytes`; duplicate-single-valued parts reject before the trait runs, and required names the pass never observed reject here unless they may still arrive behind the live stream — those ride its `pending_required` set (wire-arrival-based enforcement, §17.1/§38).
#[allow(clippy::missing_errors_doc)]
#[allow(clippy::too_many_lines)]
async fn collect_upload_document_multipart(
    body: ::axum::body::Body,
    parsed: Option<&ParsedMediaType>,
    limits: &BodyLimits,
) -> Result<UploadDocumentMultipartInput, ProtocolRejection> {
    let parsed = match parsed {
        Some(parsed) => parsed,
        None => {
            return Err(malformed_body("multipart body requires a Content-Type"));
        }
    };
    let boundary = match extract_boundary(parsed) {
        Ok(boundary) => boundary,
        Err(_) => {
            return Err(malformed_body(
                "multipart Content-Type lacks a usable boundary",
            ));
        }
    };
    let mut events: MultipartEvents = Box::pin(stream_multipart(
        body.into_data_stream(),
        boundary,
        MultipartLimits::from_body_limits(limits),
    ));
    let unknown_log =
        ::std::sync::Arc::new(::std::sync::Mutex::new(MultipartUnknownLog::default()));

    #[derive(Clone, Copy)]
    enum Stage {
        Idle,
        Drain,
        Metadata,
        TagsElement,
    }
    let mut stage = Stage::Idle;
    let mut metadata: Option<DocumentMetadata> = None;
    let mut seen_metadata = false;
    let mut tags: Vec<String> = Vec::new();
    let mut file_part: Option<UploadDocumentFilePart> = None;
    let mut buffer: Vec<u8> = Vec::new();

    loop {
        let event = match next_multipart_event(&mut events).await {
            Some(event) => event,
            None => break,
        };
        match event {
            Err(error) => return Err(multipart_rejection(&error)),
            Ok(MultipartEvent::PartBegin(headers)) => {
                stage = match headers.name.as_str() {
                    "metadata" => {
                        if seen_metadata {
                            return Err(schema_violation(format!(
                                "duplicate single-valued part `{}`",
                                headers.name
                            )));
                        }
                        seen_metadata = true;
                        buffer.clear();
                        Stage::Metadata
                    }
                    "tags" => {
                        buffer.clear();
                        Stage::TagsElement
                    }
                    "file" => {
                        let mut pending_required: Vec<&'static str> = Vec::new();
                        if metadata.is_none() {
                            pending_required.push("metadata");
                        }
                        let mut seen_single_valued: Vec<String> = Vec::new();
                        if seen_metadata {
                            seen_single_valued.push("metadata".to_owned());
                        }
                        seen_single_valued.push("file".to_owned());
                        file_part = Some(UploadDocumentFilePart {
                            file_name: headers.filename,
                            content_type: headers.content_type,
                            trailing_parts: UploadDocumentTrailingParts::default(),
                            events,
                            log: ::std::sync::Arc::clone(&unknown_log),
                            stage: UploadDocumentFilePartTailStage::Idle,
                            scalar_limit: limits.multipart_scalar_part_bytes,
                            buffer: Vec::new(),
                            finished: false,
                            pending_required,
                            seen_single_valued,
                        });
                        break;
                    }
                    other => {
                        multipart_record_unknown(&unknown_log, other);
                        Stage::Drain
                    }
                };
            }
            Ok(MultipartEvent::PartChunk(chunk)) => match stage {
                Stage::Idle | Stage::Drain => {}
                _ => {
                    buffer.extend_from_slice(&chunk);
                    if buffer.len() > limits.multipart_scalar_part_bytes {
                        return Err(ProtocolRejection::new(RejectionKind::BodyTooLarge));
                    }
                }
            },
            Ok(MultipartEvent::PartEnd) => match stage {
                Stage::Idle | Stage::Drain => {}
                Stage::Metadata => {
                    let value: DocumentMetadata = decode_json_body(&buffer)?;
                    metadata = Some(value);
                    stage = Stage::Idle;
                }
                Stage::TagsElement => {
                    let value = multipart_scalar_text::<String>("tags", &buffer)?;
                    tags.push(value);
                    stage = Stage::Idle;
                }
            },
        }
    }
    let handed_off = file_part.is_some();
    let metadata = match metadata {
        Some(value) => Some(value),
        None if handed_off => None,
        None => {
            return Err(schema_violation("missing required part `metadata`"));
        }
    };
    let file = match file_part {
        Some(part) => part,
        None => {
            return Err(schema_violation("missing required part `file`"));
        }
    };

    Ok(UploadDocumentMultipartInput {
        metadata,
        tags,
        file,
        unknown_log,
    })
}

/// Route handler for `GET` `/events` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_stream_events(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let api = __state.api.as_ref();
    let response = api.stream_events().await;
    Ok(response.into_response_with_limits(
        &limits,
        hook,
        ::std::sync::Arc::clone(&__state.stream_failure_hook),
    ))
}

/// Route handler for `GET` `/records/export` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_export_records(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let api = __state.api.as_ref();
    let response = api.export_records().await;
    Ok(response.into_response_with_limits(
        &limits,
        hook,
        ::std::sync::Arc::clone(&__state.stream_failure_hook),
    ))
}

/// Route handler for `POST` `/metrics` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_push_metrics(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json-seq"]) {
        RequestEntryMatch::AbsentContentType => {
            let (presence, _replay) =
                detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;
            if matches!(presence, BodyPresence::Empty) {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            PushMetricsJsonSeqInput::new(body.into_data_stream(), limits.max_stream_record_bytes)
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.push_metrics(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `GET` `/metrics/export` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_export_metrics(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let api = __state.api.as_ref();
    let response = api.export_metrics().await;
    Ok(response.into_response_with_limits(
        &limits,
        hook,
        ::std::sync::Arc::clone(&__state.stream_failure_hook),
    ))
}

/// Route handler for `POST` `/vendor-documents` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_post_vendor_document(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body =
        match classify_request_entry(parsed.as_ref(), &["application/vnd.acme.document-v7"]) {
            RequestEntryMatch::AbsentContentType => {
                let (presence, _replay) =
                    detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;
                if matches!(presence, BodyPresence::Empty) {
                    return Err(malformed_body("required request body arrived empty"));
                }
                return Err(unsupported_media_type(
                    "request arrived without a Content-Type",
                ));
            }
            RequestEntryMatch::Unmatched => {
                return Err(unsupported_media_type(
                    "no documented request media type matches the request Content-Type",
                ));
            }
            RequestEntryMatch::Entry(0) => body,
            RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
        };
    let api = __state.api.as_ref();
    let response = api.post_vendor_document(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `GET` `/status-probes/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_probe_status(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let api = __state.api.as_ref();
    let response = api.probe_status(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `DELETE` `/tasks/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_delete_task(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let api = __state.api.as_ref();
    let response = api.delete_task(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `GET` `/widgets/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_get_widget(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let api = __state.api.as_ref();
    let response = api.get_widget(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `HEAD` `/widgets/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_head_widget(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let api = __state.api.as_ref();
    let response = api.head_widget(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/echo-note` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_echo_note(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let (presence, replay) =
        detect_body_presence(body.into_data_stream(), limits.peek_buffer_bytes).await;
    let request_body = match presence {
        BodyPresence::Empty => None,
        BodyPresence::Failed => {
            return Err(malformed_body("request body stream failed"));
        }
        BodyPresence::NonEmpty(_) => {
            let parsed = parse_single_content_type(&__headers)?;
            match classify_request_entry(parsed.as_ref(), &["application/json"]) {
                RequestEntryMatch::AbsentContentType | RequestEntryMatch::Unmatched => {
                    return Err(unsupported_media_type(
                        "nonempty optional body arrived without a usable Content-Type",
                    ));
                }
                RequestEntryMatch::Entry(0) => {
                    ensure_utf8_charset(parsed.as_ref())?;
                    let bytes = stream_bytes(replay, limits.structured_request_bytes).await?;
                    let value: Option<String> = decode_json_body(&bytes)?;
                    Some(value)
                }
                RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
            }
        }
    };
    let api = __state.api.as_ref();
    let response = api.echo_note(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/accounts` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_create_account(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.structured_request_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: AccountWrite = decode_json_body(&bytes)?;
            Account::from(&value)
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.create_account(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `GET` `/audit/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_list_audit_entries(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("id") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `id`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `id` is malformed"));
        }
    };
    let id: String = expect_text(decoded, "id")?;
    let api = __state.api.as_ref();
    let response = api.list_audit_entries(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `PUT` `/synced` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_sync_record(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.structured_request_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: SyncedRecordWrite = decode_json_body(&bytes)?;
            require_valid_request("body", value.validate_request())?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.sync_record(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/pets` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_create_pet(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.structured_request_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: Pet = decode_json_body(&bytes)?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.create_pet(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/records` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_create_record(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/json"]) {
        RequestEntryMatch::AbsentContentType => {
            let probe = body_bytes(body, limits.structured_request_bytes).await?;
            if probe.is_empty() {
                return Err(malformed_body("required request body arrived empty"));
            }
            return Err(unsupported_media_type(
                "request arrived without a Content-Type",
            ));
        }
        RequestEntryMatch::Unmatched => {
            return Err(unsupported_media_type(
                "no documented request media type matches the request Content-Type",
            ));
        }
        RequestEntryMatch::Entry(0) => {
            ensure_utf8_charset(parsed.as_ref())?;
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: MatrixRecord = decode_json_body(&bytes)?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.create_record(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Builds the Axum router serving every documented operation (main spec §38): buffered-body routes install `DefaultBodyLimit` at `structured_request_bytes`; streaming-body routes remain exempt because nothing aggregates them.
pub fn router(
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
    stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
) -> ::axum::Router {
    let state = ServerState {
        api,
        limits,
        encode_overflow_hook,
        stream_failure_hook,
    };
    ::axum::Router::new()
        .route(
            "/widgets",
            ::axum::routing::post(route_create_widget).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route(
            "/sessions",
            ::axum::routing::post(route_create_session).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route(
            "/notes/{id}",
            ::axum::routing::put(route_put_note).layer(::axum::extract::DefaultBodyLimit::max(
                limits.structured_request_bytes,
            )),
        )
        .route("/objects/{id}", ::axum::routing::put(route_put_object))
        .route("/objects/{id}", ::axum::routing::get(route_get_object))
        .route(
            "/thumbnails/{id}",
            ::axum::routing::get(route_get_thumbnail),
        )
        .route("/documents", ::axum::routing::post(route_upload_document))
        .route("/events", ::axum::routing::get(route_stream_events))
        .route(
            "/records/export",
            ::axum::routing::get(route_export_records),
        )
        .route("/metrics", ::axum::routing::post(route_push_metrics))
        .route(
            "/metrics/export",
            ::axum::routing::get(route_export_metrics),
        )
        .route(
            "/vendor-documents",
            ::axum::routing::post(route_post_vendor_document),
        )
        .route(
            "/status-probes/{id}",
            ::axum::routing::get(route_probe_status),
        )
        .route("/tasks/{id}", ::axum::routing::delete(route_delete_task))
        .route("/widgets/{id}", ::axum::routing::get(route_get_widget))
        .route("/widgets/{id}", ::axum::routing::head(route_head_widget))
        .route(
            "/echo-note",
            ::axum::routing::post(route_echo_note).layer(::axum::extract::DefaultBodyLimit::max(
                limits.structured_request_bytes,
            )),
        )
        .route(
            "/accounts",
            ::axum::routing::post(route_create_account).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route(
            "/audit/{id}",
            ::axum::routing::get(route_list_audit_entries),
        )
        .route(
            "/synced",
            ::axum::routing::put(route_sync_record).layer(::axum::extract::DefaultBodyLimit::max(
                limits.structured_request_bytes,
            )),
        )
        .route(
            "/pets",
            ::axum::routing::post(route_create_pet).layer(::axum::extract::DefaultBodyLimit::max(
                limits.structured_request_bytes,
            )),
        )
        .route(
            "/records",
            ::axum::routing::post(route_create_record).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .with_state(state)
}

/// Canonical §39 mapping row 1: invalid or missing required  path/query/header parameter → 400.
fn invalid_parameter(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::InvalidParameter).with_detail(detail)
}

/// §39 mapping row 2: syntactically malformed framing → 400; empty  bodies on required-body operations count as missing (§28.3).
fn malformed_body(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::MalformedBody).with_detail(detail)
}

/// Missing, unparsable, wildcard, or unmatched Content-Type on a  body-bearing request → 415 (§28.2, §28.5, §39 table).
fn unsupported_media_type(
    detail: impl Into<::std::borrow::Cow<'static, str>>,
) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::UnsupportedMediaType).with_detail(detail)
}

/// Reads exactly one parsable `Content-Type` header (§28 steps 1–2, §28.1): duplicate headers are ambiguous, a missing header yields `Ok(None)`, and malformed values are never ignored or defaulted.
fn parse_single_content_type(
    headers: &::http::HeaderMap,
) -> Result<Option<ParsedMediaType>, ProtocolRejection> {
    let mut lines = headers.get_all(::http::header::CONTENT_TYPE).iter();
    let Some(first) = lines.next() else {
        return Ok(None);
    };
    if lines.next().is_some() {
        return Err(malformed_body("duplicate Content-Type headers"));
    };
    let text = first
        .to_str()
        .map_err(|_| malformed_body("Content-Type is not valid UTF-8"))?;
    parse_content_type(text)
        .map(Some)
        .map_err(|_| malformed_body("malformed Content-Type"))
}

/// Outcome of matching the incoming Content-Type against the documented request entries (§28 precedence list).
enum RequestEntryMatch {
    /// No usable Content-Type header arrived.
    AbsentContentType,
    /// A parsable Content-Type matched no documented entry.
    Unmatched,
    /// Index into the operation's documented entries.
    Entry(usize),
}

/// Ranks Exact beats suffix family beats range beats wildcard (§28); a wildcard INCOMING type never selects among multiple entries (§28.5) unless exactly one entry exists.
fn classify_request_entry(parsed: Option<&ParsedMediaType>, entries: &[&str]) -> RequestEntryMatch {
    let Some(parsed) = parsed else {
        return RequestEntryMatch::AbsentContentType;
    };
    match best_request_entry(parsed, entries) {
        Some(index) => RequestEntryMatch::Entry(index),
        None => RequestEntryMatch::Unmatched,
    }
}

/// Best documented entry for one parsed incoming type; ties resolve to the earliest declaration position.
#[must_use]
fn best_request_entry(parsed: &ParsedMediaType, entries: &[&str]) -> Option<usize> {
    if is_wildcard_incoming(parsed) {
        return if entries.len() == 1 { Some(0) } else { None };
    }
    let mut best: Option<(u8, usize)> = None;
    for (index, entry) in entries.iter().enumerate() {
        if let Some(matched) = match_entry(parsed, entry) {
            let rank = negotiation_rank(matched);
            if best.is_none_or(|(seen, _)| rank < seen) {
                best = Some((rank, index));
            }
        }
    }
    best.map(|(_, index)| index)
}

/// §28 dispatch ranking.
#[must_use]
fn negotiation_rank(matched: EntryMatch) -> u8 {
    match matched {
        EntryMatch::Exact => 0,
        EntryMatch::SuffixFamily => 1,
        EntryMatch::RangeMatch => 2,
        EntryMatch::Wildcard => 3,
    }
}

/// Bounded collection of an aggregated request body (§30.2, §38):  over-limit → 413, transport failure → 400.
async fn body_bytes(
    body: ::axum::body::Body,
    limit: usize,
) -> Result<::bytes::Bytes, ProtocolRejection> {
    match collect_body_limited(body.into_data_stream(), limit).await {
        Ok(bytes) => Ok(bytes),
        Err(CollectLimitedError::TooLarge { .. }) => {
            Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))
        }
        Err(CollectLimitedError::Source(_)) => Err(malformed_body("request body stream failed")),
    }
}

/// Bounded collection over the replayed peek stream (§28.2): the  peeked prefix is consumed exactly once.
async fn stream_bytes(
    stream: ::axum::body::BodyDataStream,
    limit: usize,
) -> Result<::bytes::Bytes, ProtocolRejection> {
    match collect_body_limited(stream, limit).await {
        Ok(bytes) => Ok(bytes),
        Err(CollectLimitedError::TooLarge { .. }) => {
            Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))
        }
        Err(CollectLimitedError::Source(_)) => Err(malformed_body("request body stream failed")),
    }
}

/// Maps bounded JSON decode failures onto §39 kinds: syntax/io →  MalformedBody 400, data errors (missing fields/types) →  SchemaViolation 422 (D-impl-runtime-validation-timing).
fn decode_json_body<T>(bytes: &[u8]) -> Result<T, ProtocolRejection>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| {
        if error.is_data() {
            ProtocolRejection::new(RejectionKind::SchemaViolation)
                .with_detail("well-formed body failed schema validation")
        } else {
            malformed_body("malformed JSON body")
        }
    })
}

/// Strict UTF-8 for bounded textual bodies (§28.4): invalid bytes  are protocol failures, never replacement characters.
fn decode_text_body(bytes: ::bytes::Bytes) -> Result<String, ProtocolRejection> {
    String::from_utf8(bytes.to_vec()).map_err(|_| malformed_body("text body is not UTF-8"))
}

fn decode_form_body<T>(bytes: &[u8], limit: usize) -> Result<T, ProtocolRejection>
where
    T: serde::de::DeserializeOwned,
{
    match decode_form_limited(bytes, limit) {
        Ok(value) => Ok(value),
        Err(::openapi_support::form::FormDecodeError::TooLarge { .. }) => {
            Err(ProtocolRejection::new(RejectionKind::BodyTooLarge))
        }
        Err(error) => {
            if error.is_syntax() {
                Err(malformed_body("malformed form body"))
            } else {
                Err(ProtocolRejection::new(RejectionKind::SchemaViolation)
                    .with_detail("well-formed body failed schema validation"))
            }
        }
    }
}

/// Boxed multipart event stream shared between the router's single-pass validation and the live streaming part handed to the application (main spec §17).
type MultipartEvents = ::std::pin::Pin<
    Box<
        dyn ::futures_core::Stream<Item = Result<MultipartEvent, MultipartError>>
            + ::std::marker::Send,
    >,
>;

/// Polls the next framing event without extension traits.
#[allow(clippy::needless_pass_by_ref_mut)]
async fn next_multipart_event(
    events: &mut MultipartEvents,
) -> Option<Result<MultipartEvent, MultipartError>> {
    ::std::future::poll_fn(|cx| events.as_mut().poll_next(cx)).await
}

/// Observability log of unrecognized/late part names (§17.1 unknown-fields-ignore default); payloads are never retained.
#[derive(Default)]
struct MultipartUnknownLog {
    names: Vec<String>,
}

/// Records one observed name, surviving mutex poisoning.
fn multipart_record_unknown(log: &::std::sync::Mutex<MultipartUnknownLog>, name: &str) {
    match log.lock() {
        Ok(mut guard) => guard.names.push(name.to_owned()),
        Err(poisoned) => poisoned.into_inner().names.push(name.to_owned()),
    }
}

/// Maps a multipart framing failure onto the §39 mapping table: cardinality limits (part count, header budget, name lengths) are bounded-collection rejections; truncation and malformed framing are syntactic failures.
fn multipart_rejection(error: &MultipartError) -> ProtocolRejection {
    match error {
        MultipartError::TooManyParts { .. }
        | MultipartError::PartHeaderTooLarge { .. }
        | MultipartError::FieldNameTooLong { .. }
        | MultipartError::FileNameTooLong { .. } => {
            ProtocolRejection::new(RejectionKind::BodyTooLarge)
        }
        MultipartError::Truncated => {
            malformed_body("multipart stream ended before the closing boundary")
        }
        MultipartError::MalformedFraming => malformed_body("malformed multipart framing"),
    }
}

/// Well-formed multipart content failing schema validation (missing required part, duplicate single-valued part, bad scalar value) → 422 (§17.1, §39 mapping row 6).
fn schema_violation(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::SchemaViolation).with_detail(detail)
}

/// Decodes one bounded textual scalar part (§17.1): non-UTF-8 bytes are MalformedBody 400; well-formed text failing its Rust type is SchemaViolation 422.
fn multipart_scalar_text<T>(wire: &'static str, bytes: &[u8]) -> Result<T, ProtocolRejection>
where
    T: ::std::str::FromStr,
{
    let text = std::str::from_utf8(bytes)
        .map_err(|_| malformed_body("multipart part is not valid UTF-8"))?;
    text.parse()
        .map_err(|_| schema_violation(format!("part `{wire}` failed schema validation")))
}

/// Runs one companion §9 request-body/part validator after decode: a violation rejects 422 SchemaViolation outside the documented enum, with a location-prefixed diagnostic detail (§39 rows 6; details stay off the wire per rule 3).
fn require_valid_request(
    location: &str,
    validation: ::std::result::Result<(), ::openapi_support::validation::Violation>,
) -> ::std::result::Result<(), ProtocolRejection> {
    validation.map_err(|violation| {
        ProtocolRejection::new(RejectionKind::SchemaViolation).with_detail(format!(
            "request body failed schema validation at `{location}`: {violation}",
        ))
    })
}

/// §28.4 charset policy (D-impl-charset-rejection): textual media  decode as UTF-8; any other declared charset is MalformedBody 400.
fn ensure_utf8_charset(parsed: Option<&ParsedMediaType>) -> Result<(), ProtocolRejection> {
    let Some(parsed) = parsed else {
        return Ok(());
    };
    if let Some((_, value)) = parsed.parameters.iter().find(|(name, _)| name == "charset") {
        let lowered = value.to_ascii_lowercase();
        if lowered != "utf-8" && lowered != "utf8" {
            return Err(malformed_body("charset outside the UTF-8 family"));
        }
    }
    Ok(())
}

/// Unwraps a scalar decode product; `None` means the parameter was  absent (required parameters reject with 400, §39 row 1).
fn expect_text(
    decoded: Option<ParamValue>,
    parameter: &'static str,
) -> Result<String, ProtocolRejection> {
    match decoded {
        Some(ParamValue::Text(text)) => Ok(text),
        Some(_) => Err(invalid_parameter(format!(
            "parameter `{parameter}` has an unexpected shape"
        ))),
        None => Err(invalid_parameter(format!(
            "missing required parameter `{parameter}`"
        ))),
    }
}

/// §34.1 steps 1–3: partial output is discarded, nothing partial  reaches the wire, and the hook carries the operation id, variant,  and limit for observability.
fn encode_overflow_fallback(
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
    limit: usize,
) -> ::axum::response::Response {
    hook.on_encode_overflow(operation_id, variant, limit);
    ::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// Bounded JSON response encoding (§34/§41); the literal keeps  distinct types such as application/problem+json separate from  application/json.
fn encode_json_limited<T>(
    status: ::http::StatusCode,
    content_type: &'static str,
    value: &T,
    limits: &BodyLimits,
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
) -> ::axum::response::Response
where
    T: serde::Serialize,
{
    let budget = limits.structured_encode_bytes;
    match serialize_json_limited(value, budget) {
        Ok(bytes) => {
            let mut response = (status, bytes).into_response();
            response.headers_mut().insert(
                ::http::header::CONTENT_TYPE,
                ::http::HeaderValue::from_static(content_type),
            );
            response
        }
        Err(error) => encode_overflow_fallback(hook, operation_id, variant, error.limit),
    }
}

/// Streams a documented binary/raw payload verbatim behind its  literal Content-Type (§32/§41); nothing aggregates it.
fn stream_response(
    status: ::http::StatusCode,
    content_type: &'static str,
    body: ::axum::body::Body,
) -> ::axum::response::Response {
    let mut response = (status, body).into_response();
    response.headers_mut().insert(
        ::http::header::CONTENT_TYPE,
        ::http::HeaderValue::from_static(content_type),
    );
    response
}

/// Per-item encoder over one erased item-stream (main spec §40): each item serializes under `max_stream_record_bytes`; overflow or an application error fires the stream-failure hook and ends the body abnormally — the committed status can never change.
struct StreamBodyEncoder<T> {
    items: ErasedItems<Result<T, ServerStreamError>>,
    limit: usize,
    hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
    operation_id: &'static str,
    encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,
    finished: bool,
}

impl<T: serde::Serialize> ::futures_core::Stream for StreamBodyEncoder<T> {
    type Item = Result<::bytes::Bytes, ServerStreamError>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            ::core::task::Poll::Ready(None)
        } else {
            match ::futures_core::Stream::poll_next(this.items.as_mut(), cx) {
                ::core::task::Poll::Pending => ::core::task::Poll::Pending,
                ::core::task::Poll::Ready(None) => {
                    this.finished = true;
                    ::core::task::Poll::Ready(None)
                }
                ::core::task::Poll::Ready(Some(Ok(item))) => {
                    let encoded = (this.encode)(&item, this.limit);
                    match encoded {
                        Ok(bytes) => ::core::task::Poll::Ready(Some(Ok(bytes))),
                        Err(error) => {
                            this.finished = true;
                            this.hook.on_stream_failure(this.operation_id, &error);
                            let failure = ServerStreamError::new(error);
                            ::core::task::Poll::Ready(Some(Err(failure)))
                        }
                    }
                }
                ::core::task::Poll::Ready(Some(Err(error))) => {
                    this.finished = true;
                    this.hook.on_stream_failure(this.operation_id, &error);
                    ::core::task::Poll::Ready(Some(Err(error)))
                }
            }
        }
    }
}

/// Builds one [`StreamBodyEncoder`] over an application producer (§40).
#[allow(clippy::too_many_arguments)]
fn stream_body_encoder<T>(
    items: ErasedItems<Result<T, ServerStreamError>>,
    limit: usize,
    hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
    operation_id: &'static str,
    encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,
) -> StreamBodyEncoder<T> {
    StreamBodyEncoder {
        items,
        limit,
        hook,
        operation_id,
        encode,
        finished: false,
    }
}

/// Streams a wildcard payload behind the application-supplied media  type (§22); `essence_str` drops parameters such as charset.
fn any_response(
    status: ::http::StatusCode,
    content_type: ::mime::Mime,
    body: ::axum::body::Body,
) -> ::axum::response::Response {
    let declared = content_type.essence_str().to_owned();
    let mut response = (status, body).into_response();
    let header =
        ::http::HeaderValue::try_from(declared).unwrap_or(::http::HeaderValue::from_static("*/*"));
    response
        .headers_mut()
        .insert(::http::header::CONTENT_TYPE, header);
    response
}

/// Appends typed documented response headers (main spec §15). A value that cannot become a `HeaderValue` fires the encode hook and emits the fixed empty 500 (§34.1 machinery; limit `0` is the recorded sentinel for non-size encode failures such as this one).
fn write_typed_headers(
    mut response: ::axum::response::Response,
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
    headers: &[(&'static str, String)],
) -> ::axum::response::Response {
    for (wire, value) in headers {
        match ::http::HeaderValue::try_from(value.as_str()) {
            Ok(header) => {
                response
                    .headers_mut()
                    .insert(::http::HeaderName::from_static(wire), header);
            }
            Err(_) => {
                return header_encode_failure(hook, operation_id, variant);
            }
        }
    }
    response
}

/// Fixed fallback for a documented header value that fails HTTP header conversion at encode time (main spec §48's internal error path): hook first, then the empty-bodied 500.
fn header_encode_failure(
    hook: &dyn EncodeOverflowHook,
    operation_id: &'static str,
    variant: &'static str,
) -> ::axum::response::Response {
    hook.on_encode_overflow(operation_id, variant, 0);
    ::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
