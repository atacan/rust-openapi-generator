//! Reqwest client generated from the OpenAPI document (main spec §8 Output A).
//!
//! Bounded JSON/form bodies (§34), streaming raw payloads (§32), exhaustive documented-status enums (§2.4), typed documented response headers (§15), redirects off by default (§30.1), and the authoritative `ClientError` (§36). Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
//!
//! Servers (companion §8): operation-level `servers` override path-level, path-level overrides root-level, and within each effective array the first entry is that operation's default base. Every DISTINCT effective default URL becomes its own stored base: `base_url` is the primary (the first operation's first effective server); further bases live in `base_url_<key>` fields whose keys are documented under `ClientBuilder::secondary_base_url`. Recorded decision: an explicit `base_url` replaces ONLY the primary base; each other base needs its own `secondary_base_url` override, so a relative secondary still requires an absolute value there (D-impl-relative-servers).
//!
//! Directional views (companion §5, main spec §50 test 50): request payloads of view-carrying components take `<M>Write` (readOnly fields structurally absent from the wire) and response payloads take `<M>Read` (writeOnly fields absent); components without markers keep their shared models. Decode ignores unrecognized keys unless a schema declares `additionalProperties: false`, so off-direction fields sent out of place never fail decode.
//! Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use ::openapi_support::client_error::{BodyLimitDirection, ClientError};
use ::openapi_support::collect::collect_reqwest_limited;
use ::openapi_support::encode::serialize_form_limited;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::jsonseq::decode_jsonseq;
use ::openapi_support::jsonseq::encode_jsonseq_item;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::{match_entry, ParsedMediaType};
use ::openapi_support::ndjson::decode_ndjson;
use ::openapi_support::params::{encode_path, ParamSpec, ParamStyle, ParamValue};
use ::openapi_support::sse::decode_sse_json;
use ::openapi_support::stream_errors::{JsonSeqDecodeError, NdjsonDecodeError, SseDecodeError};
use ::reqwest::multipart::{Form, Part};
use kitchen_sink_models::models::{
    Ack, CreateSessionForm, CreateWidget, Document, DocumentMetadata, Event, FullWidget,
    MatrixRecord, Metric, Pet, ProblemDetails, Record, Session, SuccessEnvelope, ThumbnailMetadata,
    Widget,
};
use kitchen_sink_models::views::{
    AccountRead, AccountWrite, AuditEntryRead, SyncedRecordRead, SyncedRecordWrite,
};

/// Client carrying one resolved base per distinct effective default server (companion §8): `base_url` is the PRIMARY base (the first operation's first effective server); every further distinct URL gets its own `base_url_<key>` field, and each generated method sends through its operation's own base.
#[derive(Clone)]
pub struct Client {
    http: ::reqwest::Client,
    base_url: String,
    limits: BodyLimits,
}

/// Builder for `Client` (main spec §30.1): redirects disabled unless opted in through `follow_redirects`; relative default servers require explicit overrides (D-impl-relative-servers). Recorded decision (companion §8): an explicit `base_url` replaces ONLY the primary base; every additional base is overridden per key through `secondary_base_url`.
pub struct ClientBuilder {
    http: ::reqwest::ClientBuilder,
    base_url: Option<String>,
    limits: BodyLimits,
    default_server_url: String,
    default_server_variables: Vec<(String, String, Option<Vec<String>>)>,
    server_variables: ::std::collections::BTreeMap<String, String>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    /// Process-default transport: no redirects (§30.1) and process-default body limits (§33).
    #[must_use]
    pub fn new() -> Self {
        let default_server_url = "/".to_owned();
        let default_server_variables = Vec::new();
        Self {
            http: ::reqwest::Client::builder().redirect(::reqwest::redirect::Policy::none()),
            base_url: None,
            limits: BodyLimits::process_default(),
            default_server_url,
            default_server_variables,
            server_variables: ::std::collections::BTreeMap::new(),
        }
    }

    /// Overrides the resolved PRIMARY base URL only (recorded companion §8 decision); required before `build` when the primary default server is not absolute. Secondary bases are overridden through `secondary_base_url`.
    pub fn base_url(mut self, value: impl Into<String>) -> Self {
        self.base_url = Some(value.into());
        self
    }

    /// Replaces the process-default body limits (main spec §33).
    pub fn limits(mut self, limits: BodyLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Opts into redirect following (§30.1); generated decoding never buffers bodies to enable replay.
    pub fn follow_redirects(mut self, policy: ::reqwest::redirect::Policy) -> Self {
        self.http = self.http.redirect(policy);
        self
    }

    /// Merges extra headers into every request sent through the built client (issue #12 escape hatch): covers auth schemes without a typed method plus unrelated needs like `User-Agent`. Typed credentials, when configured, are applied at `build` time on top of these headers.
    pub fn default_headers(mut self, headers: ::http::HeaderMap) -> Self {
        self.http = self.http.default_headers(headers);
        self
    }

    /// Builds the client (main spec §30.1, companion §8): every distinct base resolves independently — builder overrides or declared defaults, validated against their enums — and a non-absolute base without its own override is `ClientError::InvalidUrl` (D-impl-relative-servers).
    pub fn build(self) -> Result<Client, ClientError> {
        let base_url = match self.base_url {
            Some(explicit) => explicit,
            None => substitute_server_variables(
                &self.default_server_url,
                &self.default_server_variables,
                &self.server_variables,
            )?,
        };
        let trimmed = base_url.trim_end_matches('/');
        if !is_absolute_url(trimmed) {
            return Err(ClientError::InvalidUrl(format!(
                "base URL `{trimmed}` is not absolute; call `base_url` because no \
         absolute default server exists"
            )));
        }
        let http = self.http.build().map_err(ClientError::Transport)?;
        Ok(Client {
            http,
            base_url: trimmed.to_owned(),
            limits: self.limits,
        })
    }
}

/// Documented outcomes for `create_widget` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateWidgetResponse {
    /// HTTP 201 Created.
    Created201(Widget),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Typed payload for status 201 of `create_session` (main spec §15 Output A): required headers as plain fields, optional headers as `Option<T>`, then the decoded body.
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

/// Documented outcomes for `put_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutNoteResponse {
    /// HTTP 204 NoContent.
    NoContent204,
}

/// Documented outcomes for `put_object` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutObjectResponse {
    /// HTTP 201 Created.
    Created201,
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Streaming payload for status 200 of `get_object` (main spec §32): owns the response plus its typed documented headers (§15, superseding D-impl-typed-headers-phase2).
#[derive(Debug)]
pub struct GetObject200 {
    /// Documented response header `ETag` (optional).
    pub e_tag: Option<String>,
    /// Documented response header `Content-Length` (optional).
    pub content_length: Option<i64>,
    pub response: ::reqwest::Response,
}

impl GetObject200 {
    /// Consumes the wrapper into the raw chunk stream (main spec §32).
    pub fn into_bytes_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = ::reqwest::Result<::bytes::Bytes>> {
        self.response.bytes_stream()
    }
}

/// Documented outcomes for `get_object` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetObjectResponse {
    /// HTTP 200 Ok.
    Ok200(GetObject200),
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Documented representations for status 200 of `get_thumbnail` (main spec §11): the client negotiates via Content-Type (§28).
#[derive(Debug)]
pub enum GetThumbnail200Content {
    Json(ThumbnailMetadata),
    Any(::reqwest::Response),
}

/// Documented outcomes for `get_thumbnail` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetThumbnailResponse {
    /// HTTP 200 Ok.
    Ok200(GetThumbnail200Content),
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Multipart input for `upload_document` (main spec §17 Output A): scalar/JSON parts are owned values; binary parts stay streaming (`::reqwest::Body`, never buffered by generated code).
#[derive(Debug)]
pub struct UploadDocumentRequest {
    /// JSON part `metadata`.
    pub metadata: DocumentMetadata,
    /// owned textual part `tags`; repeated parts collect in wire order.
    pub tags: Vec<String>,
    /// streaming binary part `file`.
    pub file: ::reqwest::Body,
    /// Upload filename reported for part `file`, when set.
    pub file_name: Option<String>,
    /// Content type for part `file`, when set.
    pub file_content_type: Option<::mime::Mime>,
}

impl UploadDocumentRequest {
    /// Opens `path` as the streaming payload of part `file` (main spec §17): bytes flow through tokio-util's ReaderStream without whole-file buffering; other binary parts start empty.
    /// Errors propagate `std::io::Error` from opening the file.
    #[allow(clippy::missing_errors_doc)]
    pub async fn from_file(
        metadata: DocumentMetadata,
        tags: Vec<String>,
        path: impl AsRef<::std::path::Path>,
    ) -> Result<Self, ::std::io::Error> {
        let file = ::tokio::fs::File::open(path.as_ref()).await?;
        let stream = ::tokio_util::io::ReaderStream::new(file);
        Ok(Self {
            metadata,
            tags,
            file: ::reqwest::Body::wrap_stream(stream),
            file_name: path
                .as_ref()
                .file_name()
                .map(|value| value.to_string_lossy().into_owned()),
            file_content_type: None,
        })
    }
}

/// Typed payload for status 201 of `upload_document` (main spec §15 Output A): required headers as plain fields, optional headers as `Option<T>`, then the decoded body.
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

/// Streaming payload for status 200 of `stream_events` (main spec §18 Output A): owns the response; `into_sse_stream` decodes items incrementally, bounding each record by `max_stream_record_bytes` — never collecting the body.
#[derive(Debug)]
pub struct StreamEvents200Stream {
    pub response: ::reqwest::Response,
    pub limits: BodyLimits,
}

impl StreamEvents200Stream {
    /// Consumes the wrapper into the incremental `sse` item stream.
    pub fn into_sse_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = Result<Event, SseDecodeError>> {
        classify_sse_premature_ends(decode_sse_json::<Event, _, _>(
            self.response.bytes_stream(),
            self.limits.max_stream_record_bytes,
        ))
    }
}

/// Documented outcomes for `stream_events` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum StreamEventsResponse {
    /// HTTP 200 Ok.
    Ok200(StreamEvents200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Streaming payload for status 200 of `export_records` (main spec §19 Output A): owns the response; `into_ndjson_stream` decodes items incrementally, bounding each record by `max_stream_record_bytes` — never collecting the body.
#[derive(Debug)]
pub struct ExportRecords200Stream {
    pub response: ::reqwest::Response,
    pub limits: BodyLimits,
}

impl ExportRecords200Stream {
    /// Consumes the wrapper into the incremental `ndjson` item stream.
    pub fn into_ndjson_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = Result<Record, NdjsonDecodeError>> {
        classify_ndjson_premature_ends(decode_ndjson::<Record, _, _>(
            self.response.bytes_stream(),
            self.limits.max_stream_record_bytes,
        ))
    }
}

/// Documented outcomes for `export_records` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ExportRecordsResponse {
    /// HTTP 200 Ok.
    Ok200(ExportRecords200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Streamed `jsonseq` request items for `push_metrics` (main spec §6/§18.1): boxed erased item stream handed to the generated method. Items encode lazily with a per-item bound of `max_stream_record_bytes` (§34.2 pre-send head check, then mid-send lazy encode).
pub type PushMetricsJsonSeqBody =
    ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = Metric> + ::std::marker::Send>>;

/// Documented outcomes for `push_metrics` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PushMetricsResponse {
    /// HTTP 202 Accepted.
    Accepted202(Ack),
}

/// Streaming payload for status 200 of `export_metrics` (main spec §20 Output A): owns the response; `into_jsonseq_stream` decodes items incrementally, bounding each record by `max_stream_record_bytes` — never collecting the body.
#[derive(Debug)]
pub struct ExportMetrics200Stream {
    pub response: ::reqwest::Response,
    pub limits: BodyLimits,
}

impl ExportMetrics200Stream {
    /// Consumes the wrapper into the incremental `jsonseq` item stream.
    pub fn into_jsonseq_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = Result<Metric, JsonSeqDecodeError>> {
        classify_jsonseq_premature_ends(decode_jsonseq::<Metric, _, _>(
            self.response.bytes_stream(),
            self.limits.max_stream_record_bytes,
        ))
    }
}

/// Documented outcomes for `export_metrics` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ExportMetricsResponse {
    /// HTTP 200 Ok.
    Ok200(ExportMetrics200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Streaming payload for status 200 of `post_vendor_document` (main spec §32): owns the response.
#[derive(Debug)]
pub struct PostVendorDocument200 {
    pub response: ::reqwest::Response,
}

impl PostVendorDocument200 {
    /// Consumes the wrapper into the raw chunk stream (main spec §32).
    pub fn into_bytes_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = ::reqwest::Result<::bytes::Bytes>> {
        self.response.bytes_stream()
    }
}

/// Documented outcomes for `post_vendor_document` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostVendorDocumentResponse {
    /// HTTP 200 Ok.
    Ok200(PostVendorDocument200),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
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

/// Documented outcomes for `delete_task` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum DeleteTaskResponse {
    /// HTTP 204 NoContent.
    NoContent204,
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Documented outcomes for `get_widget` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetWidgetResponse {
    /// HTTP 200 Ok.
    Ok200(Widget),
    /// HTTP 404 NotFound.
    NotFound404,
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

/// Documented outcomes for `echo_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum EchoNoteResponse {
    /// HTTP 200 Ok.
    Ok200(Option<String>),
}

/// Documented outcomes for `create_account` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateAccountResponse {
    /// HTTP 201 Created.
    Created201(AccountRead),
}

/// Documented outcomes for `list_audit_entries` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum ListAuditEntriesResponse {
    /// HTTP 200 Ok.
    Ok200(AuditEntryRead),
}

/// Documented outcomes for `sync_record` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum SyncRecordResponse {
    /// HTTP 200 Ok.
    Ok200(SyncedRecordRead),
}

/// Documented outcomes for `create_pet` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreatePetResponse {
    /// HTTP 201 Created.
    Created201(FullWidget),
}

/// Documented outcomes for `create_record` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateRecordResponse {
    /// HTTP 201 Created.
    Created201(MatrixRecord),
}

impl Client {
    /// `POST` `/widgets`.
    /// Operation `createWidget`.
    pub async fn create_widget(
        &self,
        body: &CreateWidget,
    ) -> Result<CreateWidgetResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/widgets");
        let payload = match serialize_json_limited(body, self.limits.structured_encode_bytes) {
            Ok(payload) => payload,
            Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
        };
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "application/json")
            .header(
                ::http::header::ACCEPT,
                "application/json, application/problem+json",
            )
            .body(payload)
            .send()
            .await?;
        self.decode_create_widget(response).await
    }

    /// Shared decode tail for `create_widget` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_create_widget(
        &self,
        response: ::reqwest::Response,
    ) -> Result<CreateWidgetResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Widget = json_decode(&bytes, Some(content_type))?;
                Ok(CreateWidgetResponse::Created201(value))
            }
            ::http::StatusCode::BAD_REQUEST => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(CreateWidgetResponse::BadRequest400(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/sessions`.
    /// Operation `createSession`.
    pub async fn create_session(
        &self,
        body: &CreateSessionForm,
    ) -> Result<CreateSessionResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/sessions");
        let payload = match serialize_form_limited(body, self.limits.structured_encode_bytes) {
            Ok(payload) => payload,
            Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
        };
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(
                ::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(
                ::http::header::ACCEPT,
                "application/json, application/problem+json",
            )
            .body(payload)
            .send()
            .await?;
        self.decode_create_session(response).await
    }

    /// Shared decode tail for `create_session` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_create_session(
        &self,
        response: ::reqwest::Response,
    ) -> Result<CreateSessionResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => {
                let location = parse_required_header::<String>(&response, "location")?;
                let e_tag = parse_optional_header::<String>(&response, "etag")?;
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Session = json_decode(&bytes, Some(content_type))?;
                Ok(CreateSessionResponse::Created201(CreateSession201 {
                    location,
                    e_tag,
                    body: value,
                }))
            }
            ::http::StatusCode::UNAUTHORIZED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(CreateSessionResponse::Unauthorized401(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `PUT` `/notes/{id}`.
    /// Operation `putNote`.
    pub async fn put_note(&self, id: &str, body: &str) -> Result<PutNoteResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/notes/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        if body.len() > self.limits.structured_encode_bytes {
            return Err(encode_overflow_error(self.limits.structured_encode_bytes));
        }
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::PUT, &url)
            .header(::http::header::CONTENT_TYPE, "text/plain")
            .body(body.to_owned())
            .send()
            .await?;
        self.decode_put_note(response).await
    }

    /// Shared decode tail for `put_note` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_put_note(
        &self,
        response: ::reqwest::Response,
    ) -> Result<PutNoteResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::NO_CONTENT => Ok(PutNoteResponse::NoContent204),
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `PUT` `/objects/{id}`.
    /// Operation `putObject`.
    pub async fn put_object(
        &self,
        id: &str,
        body: ::reqwest::Body,
    ) -> Result<PutObjectResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/objects/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::PUT, &url)
            .header(::http::header::CONTENT_TYPE, "application/octet-stream")
            .header(::http::header::ACCEPT, "application/problem+json")
            .body(body)
            .send()
            .await?;
        self.decode_put_object(response).await
    }

    /// Shared decode tail for `put_object` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    /// Called by `put_object` and its §31 `put_object_replaying` twin so both share one classification path.
    #[allow(clippy::unused_async)]
    async fn decode_put_object(
        &self,
        response: ::reqwest::Response,
    ) -> Result<PutObjectResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => Ok(PutObjectResponse::Created201),
            ::http::StatusCode::BAD_REQUEST => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(PutObjectResponse::BadRequest400(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `PUT` `/objects/{id}` with explicit-factory retries (§31/D-impl-retry).
    /// Operation `putObject`.
    /// Idempotency is the caller's responsibility: PUT-style operations are natural fits; retrying POST may duplicate effects.
    /// Every attempt rebuilds the streaming body through `body_factory`; multipart-free raw payloads are never buffered for replay.
    /// Only PRE-response transport failures classified by `openapi_support::retry::is_retryable_transport` are retried — once response headers arrive the outcome is final; factory errors abort without retry.
    pub async fn put_object_replaying<F, Fut>(
        &self,
        id: &str,
        body_factory: F,
        policy: ::openapi_support::retry::RetryPolicy,
    ) -> Result<PutObjectResponse, ClientError>
    where
        F: Fn() -> Fut,
        Fut: ::std::future::Future<Output = Result<::reqwest::Body, ClientError>>,
    {
        let mut url = self.base_url.clone();
        url.push_str("/objects/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        let budget = policy.max_attempts.max(1);
        let mut failed = 0_u32;
        loop {
            let body = (body_factory)().await?;
            let mut request = self.http.request(::http::Method::PUT, &url);
            request = request
                .header(::http::header::CONTENT_TYPE, "application/octet-stream")
                .body(body);
            request = request.header(::http::header::ACCEPT, "application/problem+json");
            let response = request.send().await;
            match response {
                Ok(response) => return self.decode_put_object(response).await,
                Err(error) => {
                    failed += 1;
                    let keep_retrying =
                        failed < budget && ::openapi_support::retry::is_retryable_transport(&error);
                    if !keep_retrying {
                        return Err(ClientError::Transport(error));
                    }
                    ::openapi_support::retry::backoff_sleep(policy, failed).await;
                }
            }
        }
    }

    /// `GET` `/objects/{id}`.
    /// Operation `getObject`.
    pub async fn get_object(&self, id: &str) -> Result<GetObjectResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/objects/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "application/octet-stream, application/problem+json",
            )
            .send()
            .await?;
        self.decode_get_object(response).await
    }

    /// Shared decode tail for `get_object` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_get_object(
        &self,
        response: ::reqwest::Response,
    ) -> Result<GetObjectResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let e_tag = parse_optional_header::<String>(&response, "etag")?;
                let content_length = parse_optional_header::<i64>(&response, "content-length")?;
                Ok(GetObjectResponse::Ok200(GetObject200 {
                    e_tag,
                    content_length,
                    response,
                }))
            }
            ::http::StatusCode::NOT_FOUND => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(GetObjectResponse::NotFound404(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/thumbnails/{id}`.
    /// Operation `getThumbnail`.
    pub async fn get_thumbnail(&self, id: &str) -> Result<GetThumbnailResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/thumbnails/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "application/json, image/*, application/problem+json",
            )
            .send()
            .await?;
        self.decode_get_thumbnail(response).await
    }

    /// Shared decode tail for `get_thumbnail` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_get_thumbnail(
        &self,
        response: ::reqwest::Response,
    ) -> Result<GetThumbnailResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned(), "image/*".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                let mut best_rank: Option<u8> = None;
                let mut best_index: usize = 0;
                if let Some(rank) = match_entry(&parsed, "application/json") {
                    let rank = negotiation_rank(rank);
                    if best_rank.is_none_or(|seen| rank < seen) {
                        best_rank = Some(rank);
                        best_index = 0;
                    }
                }
                if let Some(rank) = match_entry(&parsed, "image/*") {
                    let rank = negotiation_rank(rank);
                    if best_rank.is_none_or(|seen| rank < seen) {
                        best_rank = Some(rank);
                        best_index = 1;
                    }
                }
                let selected = best_rank.is_some().then_some(best_index);
                match selected {
                    Some(0) => {
                        ensure_utf8_charset(&parsed)?;
                        let limit = self.limits.structured_response_bytes;
                        let bytes = collect_reqwest_limited(response, limit).await?;
                        if bytes.is_empty() {
                            return Err(ClientError::Decode {
                                content_type: Some(content_type),
                                source: Box::new(EmptyJsonBody),
                            });
                        }
                        let value: ThumbnailMetadata = json_decode(&bytes, Some(content_type))?;
                        let payload = GetThumbnail200Content::Json(value);
                        Ok(GetThumbnailResponse::Ok200(payload))
                    }
                    Some(1) => {
                        let payload = GetThumbnail200Content::Any(response);
                        Ok(GetThumbnailResponse::Ok200(payload))
                    }
                    _ => Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned(), "image/*".to_owned()],
                        actual: Some(mime_of(&parsed)?),
                    }),
                }
            }
            ::http::StatusCode::NOT_FOUND => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(GetThumbnailResponse::NotFound404(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/documents`.
    /// Operation `uploadDocument`.
    pub async fn upload_document(
        &self,
        body: UploadDocumentRequest,
    ) -> Result<UploadDocumentResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/documents");
        let mut request = self.http.request(::http::Method::POST, &url);
        let mut form = Form::new();
        let payload =
            match serialize_json_limited(&body.metadata, self.limits.structured_encode_bytes) {
                Ok(payload) => payload,
                Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
            };
        form = form.part(
            "metadata",
            part_with_mime(Part::bytes(Vec::from(&payload[..])), "application/json")?,
        );
        for value in &body.tags {
            form = form.part("tags", Part::text(value.clone()));
        }
        form = form.part("file", {
            let mut part = Part::stream(body.file);
            if let Some(value) = body.file_name {
                part = part.file_name(value.clone());
            }
            if let Some(value) = body.file_content_type {
                part = part_with_mime(part, value.as_ref())?;
            }
            part
        });
        request = request.multipart(form);
        request = request.header(::http::header::ACCEPT, "application/json");
        let response = request.send().await?;
        self.decode_upload_document(response).await
    }

    /// Shared decode tail for `upload_document` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    /// Called by `upload_document` and its §31 `upload_document_replaying` twin so both share one classification path.
    #[allow(clippy::unused_async)]
    async fn decode_upload_document(
        &self,
        response: ::reqwest::Response,
    ) -> Result<UploadDocumentResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => {
                let location = parse_required_header::<String>(&response, "location")?;
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Document = json_decode(&bytes, Some(content_type))?;
                Ok(UploadDocumentResponse::Created201(UploadDocument201 {
                    location,
                    body: value,
                }))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/documents` with explicit-factory retries (§31/D-impl-retry).
    /// Operation `uploadDocument`.
    /// Idempotency is the caller's responsibility: PUT-style operations are natural fits; retrying POST may duplicate effects.
    /// Every attempt rebuilds the ENTIRE multipart form through `body_factory`, re-encoding scalar/JSON parts through the bounded serializers each time.
    /// Only PRE-response transport failures classified by `openapi_support::retry::is_retryable_transport` are retried — once response headers arrive the outcome is final; factory errors abort without retry.
    pub async fn upload_document_replaying<F, Fut>(
        &self,
        body_factory: F,
        policy: ::openapi_support::retry::RetryPolicy,
    ) -> Result<UploadDocumentResponse, ClientError>
    where
        F: Fn() -> Fut,
        Fut: ::std::future::Future<Output = Result<UploadDocumentRequest, ClientError>>,
    {
        let mut url = self.base_url.clone();
        url.push_str("/documents");
        let budget = policy.max_attempts.max(1);
        let mut failed = 0_u32;
        loop {
            let body = (body_factory)().await?;
            let mut request = self.http.request(::http::Method::POST, &url);
            let mut form = Form::new();
            let payload =
                match serialize_json_limited(&body.metadata, self.limits.structured_encode_bytes) {
                    Ok(payload) => payload,
                    Err(_) => {
                        return Err(encode_overflow_error(self.limits.structured_encode_bytes));
                    }
                };
            form = form.part(
                "metadata",
                part_with_mime(Part::bytes(Vec::from(&payload[..])), "application/json")?,
            );
            for value in &body.tags {
                form = form.part("tags", Part::text(value.clone()));
            }
            form = form.part("file", {
                let mut part = Part::stream(body.file);
                if let Some(value) = body.file_name {
                    part = part.file_name(value.clone());
                }
                if let Some(value) = body.file_content_type {
                    part = part_with_mime(part, value.as_ref())?;
                }
                part
            });
            request = request.multipart(form);
            request = request.header(::http::header::ACCEPT, "application/json");
            let response = request.send().await;
            match response {
                Ok(response) => return self.decode_upload_document(response).await,
                Err(error) => {
                    failed += 1;
                    let keep_retrying =
                        failed < budget && ::openapi_support::retry::is_retryable_transport(&error);
                    if !keep_retrying {
                        return Err(ClientError::Transport(error));
                    }
                    ::openapi_support::retry::backoff_sleep(policy, failed).await;
                }
            }
        }
    }

    /// `GET` `/events`.
    /// Operation `streamEvents`.
    pub async fn stream_events(&self) -> Result<StreamEventsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/events");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "text/event-stream, application/problem+json",
            )
            .send()
            .await?;
        self.decode_stream_events(response).await
    }

    /// Shared decode tail for `stream_events` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_stream_events(
        &self,
        response: ::reqwest::Response,
    ) -> Result<StreamEventsResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => Ok(StreamEventsResponse::Ok200(StreamEvents200Stream {
                response,
                limits: self.limits,
            })),
            ::http::StatusCode::UNAUTHORIZED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(StreamEventsResponse::Unauthorized401(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/records/export`.
    /// Operation `exportRecords`.
    pub async fn export_records(&self) -> Result<ExportRecordsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/records/export");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "application/x-ndjson, application/problem+json",
            )
            .send()
            .await?;
        self.decode_export_records(response).await
    }

    /// Shared decode tail for `export_records` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_export_records(
        &self,
        response: ::reqwest::Response,
    ) -> Result<ExportRecordsResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => Ok(ExportRecordsResponse::Ok200(ExportRecords200Stream {
                response,
                limits: self.limits,
            })),
            ::http::StatusCode::UNAUTHORIZED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(ExportRecordsResponse::Unauthorized401(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/metrics`.
    /// Operation `pushMetrics`.
    pub async fn push_metrics(
        &self,
        body: PushMetricsJsonSeqBody,
    ) -> Result<PushMetricsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/metrics");
        let mut request = self.http.request(::http::Method::POST, &url);
        let encoder = stream_request_encoder(
            body,
            encode_jsonseq_item::<Metric>,
            self.limits.max_stream_record_bytes,
        )
        .await?;
        request = request
            .header(::http::header::CONTENT_TYPE, "application/json-seq")
            .body(::reqwest::Body::wrap_stream(encoder));
        request = request.header(::http::header::ACCEPT, "application/json");
        let response = request.send().await?;
        self.decode_push_metrics(response).await
    }

    /// Shared decode tail for `push_metrics` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_push_metrics(
        &self,
        response: ::reqwest::Response,
    ) -> Result<PushMetricsResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::ACCEPTED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Ack = json_decode(&bytes, Some(content_type))?;
                Ok(PushMetricsResponse::Accepted202(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/metrics/export`.
    /// Operation `exportMetrics`.
    pub async fn export_metrics(&self) -> Result<ExportMetricsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/metrics/export");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "application/json-seq, application/problem+json",
            )
            .send()
            .await?;
        self.decode_export_metrics(response).await
    }

    /// Shared decode tail for `export_metrics` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_export_metrics(
        &self,
        response: ::reqwest::Response,
    ) -> Result<ExportMetricsResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => Ok(ExportMetricsResponse::Ok200(ExportMetrics200Stream {
                response,
                limits: self.limits,
            })),
            ::http::StatusCode::UNAUTHORIZED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(ExportMetricsResponse::Unauthorized401(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/vendor-documents`.
    /// Operation `postVendorDocument`.
    pub async fn post_vendor_document(
        &self,
        body: ::reqwest::Body,
    ) -> Result<PostVendorDocumentResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/vendor-documents");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(
                ::http::header::CONTENT_TYPE,
                "application/vnd.acme.document-v7",
            )
            .header(
                ::http::header::ACCEPT,
                "application/vnd.acme.document-v7, application/problem+json",
            )
            .body(body)
            .send()
            .await?;
        self.decode_post_vendor_document(response).await
    }

    /// Shared decode tail for `post_vendor_document` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    /// Called by `post_vendor_document` and its §31 `post_vendor_document_replaying` twin so both share one classification path.
    #[allow(clippy::unused_async)]
    async fn decode_post_vendor_document(
        &self,
        response: ::reqwest::Response,
    ) -> Result<PostVendorDocumentResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                Ok(PostVendorDocumentResponse::Ok200(PostVendorDocument200 {
                    response,
                }))
            }
            ::http::StatusCode::BAD_REQUEST => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(PostVendorDocumentResponse::BadRequest400(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/vendor-documents` with explicit-factory retries (§31/D-impl-retry).
    /// Operation `postVendorDocument`.
    /// Idempotency is the caller's responsibility: PUT-style operations are natural fits; retrying POST may duplicate effects.
    /// Every attempt rebuilds the streaming body through `body_factory`; multipart-free raw payloads are never buffered for replay.
    /// Only PRE-response transport failures classified by `openapi_support::retry::is_retryable_transport` are retried — once response headers arrive the outcome is final; factory errors abort without retry.
    pub async fn post_vendor_document_replaying<F, Fut>(
        &self,
        body_factory: F,
        policy: ::openapi_support::retry::RetryPolicy,
    ) -> Result<PostVendorDocumentResponse, ClientError>
    where
        F: Fn() -> Fut,
        Fut: ::std::future::Future<Output = Result<::reqwest::Body, ClientError>>,
    {
        let mut url = self.base_url.clone();
        url.push_str("/vendor-documents");
        let budget = policy.max_attempts.max(1);
        let mut failed = 0_u32;
        loop {
            let body = (body_factory)().await?;
            let mut request = self.http.request(::http::Method::POST, &url);
            request = request
                .header(
                    ::http::header::CONTENT_TYPE,
                    "application/vnd.acme.document-v7",
                )
                .body(body);
            request = request.header(
                ::http::header::ACCEPT,
                "application/vnd.acme.document-v7, application/problem+json",
            );
            let response = request.send().await;
            match response {
                Ok(response) => return self.decode_post_vendor_document(response).await,
                Err(error) => {
                    failed += 1;
                    let keep_retrying =
                        failed < budget && ::openapi_support::retry::is_retryable_transport(&error);
                    if !keep_retrying {
                        return Err(ClientError::Transport(error));
                    }
                    ::openapi_support::retry::backoff_sleep(policy, failed).await;
                }
            }
        }
    }

    /// `GET` `/status-probes/{id}`.
    /// Operation `probeStatus`.
    pub async fn probe_status(&self, id: &str) -> Result<ProbeStatusResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/status-probes/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "application/json, application/problem+json",
            )
            .send()
            .await?;
        self.decode_probe_status(response).await
    }

    /// Shared decode tail for `probe_status` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_probe_status(
        &self,
        response: ::reqwest::Response,
    ) -> Result<ProbeStatusResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Widget = json_decode(&bytes, Some(content_type))?;
                Ok(ProbeStatusResponse::Ok200(value))
            }
            status if (200..300).contains(&status.as_u16()) => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: SuccessEnvelope = json_decode(&bytes, Some(content_type))?;
                Ok(ProbeStatusResponse::Success2xx {
                    status,
                    body: value,
                })
            }
            status if (400..500).contains(&status.as_u16()) => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(ProbeStatusResponse::ClientError4xx {
                    status,
                    body: value,
                })
            }
            status => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(ProbeStatusResponse::Default {
                    status,
                    body: value,
                })
            }
        }
    }

    /// `DELETE` `/tasks/{id}`.
    /// Operation `deleteTask`.
    pub async fn delete_task(&self, id: &str) -> Result<DeleteTaskResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/tasks/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::DELETE, &url)
            .header(::http::header::ACCEPT, "application/problem+json")
            .send()
            .await?;
        self.decode_delete_task(response).await
    }

    /// Shared decode tail for `delete_task` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_delete_task(
        &self,
        response: ::reqwest::Response,
    ) -> Result<DeleteTaskResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::NO_CONTENT => Ok(DeleteTaskResponse::NoContent204),
            ::http::StatusCode::NOT_FOUND => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(DeleteTaskResponse::NotFound404(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/widgets/{id}`.
    /// Operation `getWidget`.
    pub async fn get_widget(&self, id: &str) -> Result<GetWidgetResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/widgets/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(::http::header::ACCEPT, "application/json")
            .send()
            .await?;
        self.decode_get_widget(response).await
    }

    /// Shared decode tail for `get_widget` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_get_widget(
        &self,
        response: ::reqwest::Response,
    ) -> Result<GetWidgetResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Widget = json_decode(&bytes, Some(content_type))?;
                Ok(GetWidgetResponse::Ok200(value))
            }
            ::http::StatusCode::NOT_FOUND => Ok(GetWidgetResponse::NotFound404),
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `HEAD` `/widgets/{id}`.
    /// Operation `headWidget`.
    pub async fn head_widget(&self, id: &str) -> Result<HeadWidgetResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/widgets/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self.http.request(::http::Method::HEAD, &url).send().await?;
        self.decode_head_widget(response).await
    }

    /// Shared decode tail for `head_widget` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_head_widget(
        &self,
        response: ::reqwest::Response,
    ) -> Result<HeadWidgetResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let e_tag = parse_required_header::<String>(&response, "etag")?;
                let content_length = parse_required_header::<i64>(&response, "content-length")?;
                Ok(HeadWidgetResponse::Ok200 {
                    e_tag,
                    content_length,
                })
            }
            ::http::StatusCode::NOT_FOUND => Ok(HeadWidgetResponse::NotFound404),
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/echo-note`.
    /// Operation `echoNote`.
    pub async fn echo_note(
        &self,
        body: Option<&Option<String>>,
    ) -> Result<EchoNoteResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/echo-note");
        let mut request = self.http.request(::http::Method::POST, &url);
        if let Some(body) = body {
            let payload = match serialize_json_limited(body, self.limits.structured_encode_bytes) {
                Ok(payload) => payload,
                Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
            };
            request = request
                .header(::http::header::CONTENT_TYPE, "application/json")
                .body(payload);
        }
        request = request.header(::http::header::ACCEPT, "application/json");
        let response = request.send().await?;
        self.decode_echo_note(response).await
    }

    /// Shared decode tail for `echo_note` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_echo_note(
        &self,
        response: ::reqwest::Response,
    ) -> Result<EchoNoteResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Option<String> = json_decode(&bytes, Some(content_type))?;
                Ok(EchoNoteResponse::Ok200(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/accounts`.
    /// Operation `createAccount`.
    pub async fn create_account(
        &self,
        body: &AccountWrite,
    ) -> Result<CreateAccountResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/accounts");
        let payload = match serialize_json_limited(body, self.limits.structured_encode_bytes) {
            Ok(payload) => payload,
            Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
        };
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "application/json")
            .header(::http::header::ACCEPT, "application/json")
            .body(payload)
            .send()
            .await?;
        self.decode_create_account(response).await
    }

    /// Shared decode tail for `create_account` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_create_account(
        &self,
        response: ::reqwest::Response,
    ) -> Result<CreateAccountResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: AccountRead = json_decode(&bytes, Some(content_type))?;
                Ok(CreateAccountResponse::Created201(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/audit/{id}`.
    /// Operation `listAuditEntries`.
    pub async fn list_audit_entries(
        &self,
        id: &str,
    ) -> Result<ListAuditEntriesResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/audit/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(::http::header::ACCEPT, "application/json")
            .send()
            .await?;
        self.decode_list_audit_entries(response).await
    }

    /// Shared decode tail for `list_audit_entries` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_list_audit_entries(
        &self,
        response: ::reqwest::Response,
    ) -> Result<ListAuditEntriesResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: AuditEntryRead = json_decode(&bytes, Some(content_type))?;
                Ok(ListAuditEntriesResponse::Ok200(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `PUT` `/synced`.
    /// Operation `syncRecord`.
    pub async fn sync_record(
        &self,
        body: &SyncedRecordWrite,
    ) -> Result<SyncRecordResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/synced");
        let payload = match serialize_json_limited(body, self.limits.structured_encode_bytes) {
            Ok(payload) => payload,
            Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
        };
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::PUT, &url)
            .header(::http::header::CONTENT_TYPE, "application/json")
            .header(::http::header::ACCEPT, "application/json")
            .body(payload)
            .send()
            .await?;
        self.decode_sync_record(response).await
    }

    /// Shared decode tail for `sync_record` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_sync_record(
        &self,
        response: ::reqwest::Response,
    ) -> Result<SyncRecordResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: SyncedRecordRead = json_decode(&bytes, Some(content_type))?;
                Ok(SyncRecordResponse::Ok200(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/pets`.
    /// Operation `createPet`.
    pub async fn create_pet(&self, body: &Pet) -> Result<CreatePetResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/pets");
        let payload = match serialize_json_limited(body, self.limits.structured_encode_bytes) {
            Ok(payload) => payload,
            Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
        };
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "application/json")
            .header(::http::header::ACCEPT, "application/json")
            .body(payload)
            .send()
            .await?;
        self.decode_create_pet(response).await
    }

    /// Shared decode tail for `create_pet` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_create_pet(
        &self,
        response: ::reqwest::Response,
    ) -> Result<CreatePetResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: FullWidget = json_decode(&bytes, Some(content_type))?;
                Ok(CreatePetResponse::Created201(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/records`.
    /// Operation `createRecord`.
    pub async fn create_record(
        &self,
        body: &MatrixRecord,
    ) -> Result<CreateRecordResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/records");
        let payload = match serialize_json_limited(body, self.limits.structured_encode_bytes) {
            Ok(payload) => payload,
            Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
        };
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "application/json")
            .header(::http::header::ACCEPT, "application/json")
            .body(payload)
            .send()
            .await?;
        self.decode_create_record(response).await
    }

    /// Shared decode tail for `create_record` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_create_record(
        &self,
        response: ::reqwest::Response,
    ) -> Result<CreateRecordResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: MatrixRecord = json_decode(&bytes, Some(content_type))?;
                Ok(CreateRecordResponse::Created201(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }
}

/// One predicate shared by every framing classifier (main spec §40): delegates to the support crate's hyper-aware READ-side premature-body-end classification.
fn premature_body_end(error: &(dyn ::std::error::Error + Send + Sync + 'static)) -> bool {
    ::openapi_support::transport_classify::is_premature_body_end(error)
}

/// Remaps ONE decoded item of the `jsonseq` stream (main spec §40): hyper READ-side premature body ends — the connection closed before the promised message completed — become `JsonSeqDecodeError::Truncated`; every other transport failure keeps flowing through as `JsonSeqDecodeError::Source` with its cause preserved.
fn remap_jsonseq_item<T>(item: Result<T, JsonSeqDecodeError>) -> Result<T, JsonSeqDecodeError> {
    match item {
        Ok(value) => Ok(value),
        Err(JsonSeqDecodeError::Source(source)) if premature_body_end(source.as_ref()) => {
            Err(JsonSeqDecodeError::Truncated)
        }
        other => other,
    }
}

/// Wraps one `jsonseq` decoder so transport failures are classified once at the adapter boundary (main spec §40 client-visible effect): truncation is never mistaken for clean end-of-stream or an opaque transport fault.
struct ClassifyJsonSeqPrematureEnds<S> {
    inner: ::std::pin::Pin<Box<S>>,
}

impl<S, T> ::futures_core::Stream for ClassifyJsonSeqPrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, JsonSeqDecodeError>>,
{
    type Item = Result<T, JsonSeqDecodeError>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        self.get_mut()
            .inner
            .as_mut()
            .poll_next(cx)
            .map(|option| option.map(remap_jsonseq_item::<T>))
    }
}

/// Classifies transport failures beneath one `jsonseq` decoder (main spec §40).
fn classify_jsonseq_premature_ends<S, T>(inner: S) -> ClassifyJsonSeqPrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, JsonSeqDecodeError>>,
{
    ClassifyJsonSeqPrematureEnds {
        inner: Box::pin(inner),
    }
}

/// Remaps ONE decoded item of the `ndjson` stream (main spec §40): hyper READ-side premature body ends — the connection closed before the promised message completed — become `NdjsonDecodeError::Truncated`; every other transport failure keeps flowing through as `NdjsonDecodeError::Source` with its cause preserved.
fn remap_ndjson_item<T>(item: Result<T, NdjsonDecodeError>) -> Result<T, NdjsonDecodeError> {
    match item {
        Ok(value) => Ok(value),
        Err(NdjsonDecodeError::Source(source)) if premature_body_end(source.as_ref()) => {
            Err(NdjsonDecodeError::Truncated)
        }
        other => other,
    }
}

/// Wraps one `ndjson` decoder so transport failures are classified once at the adapter boundary (main spec §40 client-visible effect): truncation is never mistaken for clean end-of-stream or an opaque transport fault.
struct ClassifyNdjsonPrematureEnds<S> {
    inner: ::std::pin::Pin<Box<S>>,
}

impl<S, T> ::futures_core::Stream for ClassifyNdjsonPrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, NdjsonDecodeError>>,
{
    type Item = Result<T, NdjsonDecodeError>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        self.get_mut()
            .inner
            .as_mut()
            .poll_next(cx)
            .map(|option| option.map(remap_ndjson_item::<T>))
    }
}

/// Classifies transport failures beneath one `ndjson` decoder (main spec §40).
fn classify_ndjson_premature_ends<S, T>(inner: S) -> ClassifyNdjsonPrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, NdjsonDecodeError>>,
{
    ClassifyNdjsonPrematureEnds {
        inner: Box::pin(inner),
    }
}

/// Remaps ONE decoded item of the `sse` stream (main spec §40): hyper READ-side premature body ends — the connection closed before the promised message completed — become `SseDecodeError::Truncated`; every other transport failure keeps flowing through as `SseDecodeError::Source` with its cause preserved.
fn remap_sse_item<T>(item: Result<T, SseDecodeError>) -> Result<T, SseDecodeError> {
    match item {
        Ok(value) => Ok(value),
        Err(SseDecodeError::Source(source)) if premature_body_end(source.as_ref()) => {
            Err(SseDecodeError::Truncated)
        }
        other => other,
    }
}

/// Wraps one `sse` decoder so transport failures are classified once at the adapter boundary (main spec §40 client-visible effect): truncation is never mistaken for clean end-of-stream or an opaque transport fault.
struct ClassifySsePrematureEnds<S> {
    inner: ::std::pin::Pin<Box<S>>,
}

impl<S, T> ::futures_core::Stream for ClassifySsePrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, SseDecodeError>>,
{
    type Item = Result<T, SseDecodeError>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        self.get_mut()
            .inner
            .as_mut()
            .poll_next(cx)
            .map(|option| option.map(remap_sse_item::<T>))
    }
}

/// Classifies transport failures beneath one `sse` decoder (main spec §40).
fn classify_sse_premature_ends<S, T>(inner: S) -> ClassifySsePrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, SseDecodeError>>,
{
    ClassifySsePrematureEnds {
        inner: Box::pin(inner),
    }
}

/// Polls the next item of a boxed erased item stream (main spec §6 request-direction streams).
fn poll_items<T>(
    items: &mut ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>,
) -> impl ::std::future::Future<Output = Option<T>> + '_ {
    ::std::future::poll_fn(|cx| ::futures_core::Stream::poll_next(items.as_mut(), cx))
}

/// Lazy mid-send encoder over one streamed record request body (§34.2): items encode one at a time under `limit`.
struct RequestItemEncoder<T> {
    items: ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>,
    head: Option<::bytes::Bytes>,
    limit: usize,
    encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,
    finished: bool,
}

impl<T: serde::Serialize> ::futures_core::Stream for RequestItemEncoder<T> {
    type Item = Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return ::core::task::Poll::Ready(None);
        }
        if let Some(head) = this.head.take() {
            return ::core::task::Poll::Ready(Some(Ok(head)));
        }
        match ::futures_core::Stream::poll_next(this.items.as_mut(), cx) {
            ::core::task::Poll::Pending => ::core::task::Poll::Pending,
            ::core::task::Poll::Ready(None) => {
                this.finished = true;
                ::core::task::Poll::Ready(None)
            }
            ::core::task::Poll::Ready(Some(item)) => match (this.encode)(&item, this.limit) {
                Ok(bytes) => ::core::task::Poll::Ready(Some(Ok(bytes))),
                Err(error) => {
                    this.finished = true;
                    ::core::task::Poll::Ready(Some(Err(error)))
                }
            },
        }
    }
}

/// Encodes the first item of one streamed record request body eagerly (§34.2: an oversized head returns `ClientError::BodyTooLarge` without sending anything) and hands the remaining items to the lazy mid-send encoder.
/// `encode` is the per-framing bounded item encoder; `limit` is `max_stream_record_bytes`.
#[allow(clippy::missing_errors_doc)]
async fn stream_request_encoder<T>(
    items: ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>,
    encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,
    limit: usize,
) -> Result<RequestItemEncoder<T>, ClientError> {
    let mut items = items;
    let mut head = None;
    if let Some(item) = poll_items(&mut items).await {
        match encode(&item, limit) {
            Ok(bytes) => head = Some(bytes),
            Err(_) => return Err(encode_overflow_error(limit)),
        }
    }
    Ok(RequestItemEncoder {
        items,
        head,
        limit,
        encode,
        finished: false,
    })
}

/// Attaches a declared media type to one multipart part (main spec §17). The literal was planned from the document's `encoding.contentType`; a value the MIME parser refuses is a malformed content type, never silently defaulted.
#[allow(clippy::missing_errors_doc)]
fn part_with_mime(part: Part, mime_literal: &str) -> Result<Part, ClientError> {
    part.mime_str(mime_literal).map_err(|_| {
        ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)
    })
}

/// Reads and parses the response `Content-Type` (§28 steps 1–2): duplicate headers are ambiguous decode errors (§28.1), a missing header yields `None`, malformed values surface as `MalformedContentType`.
fn parse_response_content_type(
    response: &::reqwest::Response,
) -> Result<Option<ParsedMediaType>, ClientError> {
    let values: Vec<&::http::HeaderValue> = response
        .headers()
        .get_all(::http::header::CONTENT_TYPE)
        .iter()
        .collect();
    if values.len() > 1 {
        return Err(ClientError::Decode {
            content_type: None,
            source: Box::new(DuplicateContentType),
        });
    }
    let Some(raw) = values.first() else {
        return Ok(None);
    };
    let text = raw.to_str().map_err(|_| {
        ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)
    })?;
    Ok(Some(::openapi_support::mediatype::parse_content_type(
        text,
    )?))
}

/// duplicate Content-Type headers are an ambiguous message (§28.1); generated code never picks one arbitrarily
#[derive(Debug)]
struct DuplicateContentType;

impl std::fmt::Display for DuplicateContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("duplicate Content-Type headers on one message")
    }
}

impl std::error::Error for DuplicateContentType {}

/// Builds the `mime::Mime` carried by [`ClientError`] fields.
#[allow(clippy::missing_errors_doc)]
fn mime_of(parsed: &ParsedMediaType) -> Result<::mime::Mime, ClientError> {
    let subtype = match &parsed.suffix {
        Some(suffix) => format!("{}+{}", parsed.subtype, suffix),
        None => parsed.subtype.clone(),
    };
    let text = format!("{}/{}", parsed.ty, subtype);
    text.parse().map_err(|_| {
        ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)
    })
}

/// §28.4 charset policy (D-impl-charset-rejection): textual media decode as UTF-8; any other declared charset is a decode error instead of replacement-character corruption.
#[allow(clippy::missing_errors_doc)]
fn ensure_utf8_charset(parsed: &ParsedMediaType) -> Result<(), ClientError> {
    if let Some((_, value)) = parsed.parameters.iter().find(|(name, _)| name == "charset") {
        let lowered = value.to_ascii_lowercase();
        if lowered != "utf-8" && lowered != "utf8" {
            return Err(ClientError::Decode {
                content_type: None,
                source: Box::new(UnsupportedCharset(value.clone())),
            });
        }
    }
    Ok(())
}

/// declared charset is outside the UTF-8 family (§28.4); generated clients surface this as `ClientError::Decode` (D-impl-charset-rejection)
#[derive(Debug)]
struct UnsupportedCharset(String);

impl std::fmt::Display for UnsupportedCharset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "charset `{}` is outside the UTF-8 family", self.0)
    }
}

impl std::error::Error for UnsupportedCharset {}

/// a documented JSON status arrived with an empty body; empty input is never decoded as a default value (§28.3)
#[derive(Debug)]
struct EmptyJsonBody;

impl std::fmt::Display for EmptyJsonBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("documented JSON status arrived with an empty body")
    }
}

impl std::error::Error for EmptyJsonBody {}

/// §28 dispatch ranking: Exact beats suffix family beats range match beats wildcard.
#[must_use]
fn negotiation_rank(matched: ::openapi_support::mediatype::EntryMatch) -> u8 {
    match matched {
        ::openapi_support::mediatype::EntryMatch::Exact => 0,
        ::openapi_support::mediatype::EntryMatch::SuffixFamily => 1,
        ::openapi_support::mediatype::EntryMatch::RangeMatch => 2,
        ::openapi_support::mediatype::EntryMatch::Wildcard => 3,
    }
}

/// Maps bounded JSON decode failures onto [`ClientError::Decode`] (§36); `content_type` is carried for diagnostics.
fn json_decode<T>(bytes: &[u8], content_type: Option<::mime::Mime>) -> Result<T, ClientError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| ClientError::Decode {
        content_type,
        source: Box::new(error),
    })
}

/// Client-side encode overflow (§34.2): returned BEFORE anything is sent.
#[must_use]
fn encode_overflow_error(limit: usize) -> ClientError {
    ClientError::BodyTooLarge {
        direction: BodyLimitDirection::Encode,
        limit,
    }
}

/// Typed documented response headers (main spec §15): required headers missing from the response are protocol errors (`MissingRequiredHeader`), values failing their Rust type are `InvalidHeader`; both surface BEFORE the body is consumed. A repeated documented header reads its first occurrence.
#[allow(clippy::missing_errors_doc)]
fn parse_required_header<T>(
    response: &::reqwest::Response,
    wire: &'static str,
) -> Result<T, ClientError>
where
    T: ::std::str::FromStr,
    T::Err: ::std::error::Error + Send + Sync + 'static,
{
    let name = ::http::HeaderName::from_static(wire);
    let Some(raw) = response.headers().get(&name) else {
        return Err(ClientError::MissingRequiredHeader { name });
    };
    parse_header_value(name, raw)
}

#[allow(clippy::missing_errors_doc)]
fn parse_optional_header<T>(
    response: &::reqwest::Response,
    wire: &'static str,
) -> Result<Option<T>, ClientError>
where
    T: ::std::str::FromStr,
    T::Err: ::std::error::Error + Send + Sync + 'static,
{
    let name = ::http::HeaderName::from_static(wire);
    match response.headers().get(&name) {
        Some(raw) => parse_header_value(name, raw).map(Some),
        None => Ok(None),
    }
}

/// Decodes one raw header value into its typed representation.
#[allow(clippy::missing_errors_doc)]
fn parse_header_value<T>(
    name: ::http::HeaderName,
    raw: &::http::HeaderValue,
) -> Result<T, ClientError>
where
    T: ::std::str::FromStr,
    T::Err: ::std::error::Error + Send + Sync + 'static,
{
    let text = raw.to_str().map_err(|_| ClientError::InvalidHeader {
        name: name.clone(),
        source: Box::new(NonUtf8HeaderValue),
    })?;
    text.parse().map_err(|source| ClientError::InvalidHeader {
        name,
        source: Box::new(source),
    })
}

/// a documented response header carried non-UTF-8 bytes; generated clients surface this as `ClientError::InvalidHeader`
#[derive(Debug)]
struct NonUtf8HeaderValue;

impl std::fmt::Display for NonUtf8HeaderValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("documented response header value is not valid UTF-8")
    }
}

impl std::error::Error for NonUtf8HeaderValue {}

/// Substitutes server variables with builder overrides or declared defaults, validating enum membership at build time (companion §8).
#[allow(clippy::missing_errors_doc)]
fn substitute_server_variables(
    url: &str,
    variables: &[(String, String, Option<Vec<String>>)],
    overrides: &::std::collections::BTreeMap<String, String>,
) -> Result<String, ClientError> {
    let mut resolved = url.to_owned();
    for (name, default, allowed) in variables {
        let value = if let Some(value) = overrides.get(name) {
            value.clone()
        } else {
            default.clone()
        };
        if let Some(allowed) = allowed {
            if !allowed.contains(&value) {
                return Err(ClientError::InvalidUrl(format!(
                    "server variable `{name}` value `{value}` is not one of {allowed:?}"
                )));
            }
        }
        let placeholder = format!("{{{name}}}");
        if !resolved.contains(&placeholder) {
            return Err(ClientError::InvalidUrl(format!(
                "server variable `{name}` has no placeholder in `{url}`"
            )));
        }
        resolved = resolved.replace(&placeholder, &value);
    }
    if resolved.contains('{') || resolved.contains('}') {
        return Err(ClientError::InvalidUrl(format!(
            "unresolved server variable placeholder in `{resolved}`"
        )));
    }
    Ok(resolved)
}

/// Absolute-URL gate for the resolved base (D-impl-relative-servers): scheme + `://` + non-empty remainder.
#[must_use]
fn is_absolute_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    !scheme.is_empty()
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
        && !rest.is_empty()
}
