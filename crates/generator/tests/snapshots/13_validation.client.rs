//! Reqwest client generated from the OpenAPI document (main spec §8 Output A).
//!
//! Bounded JSON/form bodies (§34), streaming raw payloads (§32), exhaustive documented-status enums (§2.4), typed documented response headers (§15), redirects off by default (§30.1), and the authoritative `ClientError` (§36). Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
//!
//! Servers (companion §8): operation-level `servers` override path-level, path-level overrides root-level, and within each effective array the first entry is that operation's default base. Every DISTINCT effective default URL becomes its own stored base: `base_url` is the primary (the first operation's first effective server); further bases live in `base_url_<key>` fields whose keys are documented under `ClientBuilder::secondary_base_url`. Recorded decision: an explicit `base_url` replaces ONLY the primary base; each other base needs its own `secondary_base_url` override, so a relative secondary still requires an absolute value there (D-impl-relative-servers).
//! Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{FeedbackForm, NoteReceipt, RegisteredSlug, Ticket};
use ::openapi_support::client_error::{BodyLimitDirection, ClientError};
use ::openapi_support::collect::collect_reqwest_limited;
use ::openapi_support::encode::serialize_form_limited;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::ParsedMediaType;
use ::reqwest::multipart::{Form, Part};

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

/// Documented outcomes for `create_ticket` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateTicketResponse {
    /// HTTP 201 Created.
    Created201(Ticket),
}

/// Documented outcomes for `register_slug` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum RegisterSlugResponse {
    /// HTTP 201 Created.
    Created201(RegisteredSlug),
}

/// Documented outcomes for `post_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostNoteResponse {
    /// HTTP 201 Created.
    Created201(NoteReceipt),
}

/// Documented outcomes for `send_feedback` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum SendFeedbackResponse {
    /// HTTP 204 NoContent.
    NoContent204,
}

/// Multipart input for `upload_attachment` (main spec §17 Output A): scalar/JSON parts are owned values; binary parts stay streaming (`::reqwest::Body`, never buffered by generated code).
#[derive(Debug)]
pub struct UploadAttachmentRequest {
    /// owned textual part `title`.
    pub title: String,
    /// owned textual part `kind`.
    pub kind: String,
    /// streaming binary part `attachment`.
    pub attachment: ::reqwest::Body,
    /// Upload filename reported for part `attachment`, when set.
    pub attachment_name: Option<String>,
    /// Content type for part `attachment`, when set.
    pub attachment_content_type: Option<::mime::Mime>,
}

impl UploadAttachmentRequest {
    /// Opens `path` as the streaming payload of part `attachment` (main spec §17): bytes flow through tokio-util's ReaderStream without whole-file buffering; other binary parts start empty.
    /// Errors propagate `std::io::Error` from opening the file.
    #[allow(clippy::missing_errors_doc)]
    pub async fn from_attachment(
        title: String,
        kind: String,
        path: impl AsRef<::std::path::Path>,
    ) -> Result<Self, ::std::io::Error> {
        let file = ::tokio::fs::File::open(path.as_ref()).await?;
        let stream = ::tokio_util::io::ReaderStream::new(file);
        Ok(Self {
            title,
            kind,
            attachment: ::reqwest::Body::wrap_stream(stream),
            attachment_name: path
                .as_ref()
                .file_name()
                .map(|value| value.to_string_lossy().into_owned()),
            attachment_content_type: None,
        })
    }
}

/// Documented outcomes for `upload_attachment` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum UploadAttachmentResponse {
    /// HTTP 201 Created.
    Created201(serde_json::Map<String, serde_json::Value>),
}

impl Client {
    /// `POST` `/tickets`.
    /// Operation `createTicket`.
    pub async fn create_ticket(&self, body: &Ticket) -> Result<CreateTicketResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/tickets");
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
        self.decode_create_ticket(response).await
    }

    /// Shared decode tail for `create_ticket` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_create_ticket(
        &self,
        response: ::reqwest::Response,
    ) -> Result<CreateTicketResponse, ClientError> {
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
                let value: Ticket = json_decode(&bytes, Some(content_type))?;
                Ok(CreateTicketResponse::Created201(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/slugs`.
    /// Operation `registerSlug`.
    pub async fn register_slug(&self, body: &String) -> Result<RegisterSlugResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/slugs");
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
        self.decode_register_slug(response).await
    }

    /// Shared decode tail for `register_slug` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_register_slug(
        &self,
        response: ::reqwest::Response,
    ) -> Result<RegisterSlugResponse, ClientError> {
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
                let value: RegisteredSlug = json_decode(&bytes, Some(content_type))?;
                Ok(RegisterSlugResponse::Created201(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/notes`.
    /// Operation `postNote`.
    pub async fn post_note(&self, body: &str) -> Result<PostNoteResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/notes");
        if body.len() > self.limits.structured_encode_bytes {
            return Err(encode_overflow_error(self.limits.structured_encode_bytes));
        }
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "text/plain")
            .header(::http::header::ACCEPT, "application/json")
            .body(body.to_owned())
            .send()
            .await?;
        self.decode_post_note(response).await
    }

    /// Shared decode tail for `post_note` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_post_note(
        &self,
        response: ::reqwest::Response,
    ) -> Result<PostNoteResponse, ClientError> {
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
                let value: NoteReceipt = json_decode(&bytes, Some(content_type))?;
                Ok(PostNoteResponse::Created201(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/feedback`.
    /// Operation `sendFeedback`.
    pub async fn send_feedback(
        &self,
        body: &FeedbackForm,
    ) -> Result<SendFeedbackResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/feedback");
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
            .body(payload)
            .send()
            .await?;
        self.decode_send_feedback(response).await
    }

    /// Shared decode tail for `send_feedback` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_send_feedback(
        &self,
        response: ::reqwest::Response,
    ) -> Result<SendFeedbackResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::NO_CONTENT => Ok(SendFeedbackResponse::NoContent204),
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/uploads`.
    /// Operation `uploadAttachment`.
    pub async fn upload_attachment(
        &self,
        body: UploadAttachmentRequest,
    ) -> Result<UploadAttachmentResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/uploads");
        let mut request = self.http.request(::http::Method::POST, &url);
        let mut form = Form::new();
        form = form.part("title", Part::text(body.title.clone()));
        form = form.part("kind", Part::text(body.kind.clone()));
        form = form.part("attachment", {
            let mut part = Part::stream(body.attachment);
            if let Some(value) = body.attachment_name {
                part = part.file_name(value.clone());
            }
            if let Some(value) = body.attachment_content_type {
                part = part_with_mime(part, value.as_ref())?;
            }
            part
        });
        request = request.multipart(form);
        request = request.header(::http::header::ACCEPT, "application/json");
        let response = request.send().await?;
        self.decode_upload_attachment(response).await
    }

    /// Shared decode tail for `upload_attachment` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    /// Called by `upload_attachment` and its §31 `upload_attachment_replaying` twin so both share one classification path.
    #[allow(clippy::unused_async)]
    async fn decode_upload_attachment(
        &self,
        response: ::reqwest::Response,
    ) -> Result<UploadAttachmentResponse, ClientError> {
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
                let value: serde_json::Map<String, serde_json::Value> =
                    json_decode(&bytes, Some(content_type))?;
                Ok(UploadAttachmentResponse::Created201(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/uploads` with explicit-factory retries (§31/D-impl-retry).
    /// Operation `uploadAttachment`.
    /// Idempotency is the caller's responsibility: PUT-style operations are natural fits; retrying POST may duplicate effects.
    /// Every attempt rebuilds the ENTIRE multipart form through `body_factory`, re-encoding scalar/JSON parts through the bounded serializers each time.
    /// Only PRE-response transport failures classified by `openapi_support::retry::is_retryable_transport` are retried — once response headers arrive the outcome is final; factory errors abort without retry.
    pub async fn upload_attachment_replaying<F, Fut>(
        &self,
        body_factory: F,
        policy: ::openapi_support::retry::RetryPolicy,
    ) -> Result<UploadAttachmentResponse, ClientError>
    where
        F: Fn() -> Fut,
        Fut: ::std::future::Future<Output = Result<UploadAttachmentRequest, ClientError>>,
    {
        let mut url = self.base_url.clone();
        url.push_str("/uploads");
        let budget = policy.max_attempts.max(1);
        let mut failed = 0_u32;
        loop {
            let body = (body_factory)().await?;
            let mut request = self.http.request(::http::Method::POST, &url);
            let mut form = Form::new();
            form = form.part("title", Part::text(body.title.clone()));
            form = form.part("kind", Part::text(body.kind.clone()));
            form = form.part("attachment", {
                let mut part = Part::stream(body.attachment);
                if let Some(value) = body.attachment_name {
                    part = part.file_name(value.clone());
                }
                if let Some(value) = body.attachment_content_type {
                    part = part_with_mime(part, value.as_ref())?;
                }
                part
            });
            request = request.multipart(form);
            request = request.header(::http::header::ACCEPT, "application/json");
            let response = request.send().await;
            match response {
                Ok(response) => return self.decode_upload_attachment(response).await,
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
