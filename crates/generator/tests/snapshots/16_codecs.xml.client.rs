//! Reqwest client generated from the OpenAPI document (main spec §8 Output A).
//!
//! Bounded JSON/form bodies (§34), streaming raw payloads (§32), exhaustive documented-status enums (§2.4), typed documented response headers (§15), redirects off by default (§30.1), and the authoritative `ClientError` (§36). Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
//!
//! Servers (companion §8): operation-level `servers` override path-level, path-level overrides root-level, and within each effective array the first entry is that operation's default base. Every DISTINCT effective default URL becomes its own stored base: `base_url` is the primary (the first operation's first effective server); further bases live in `base_url_<key>` fields whose keys are documented under `ClientBuilder::secondary_base_url`. Recorded decision: an explicit `base_url` replaces ONLY the primary base; each other base needs its own `secondary_base_url` override, so a relative secondary still requires an absolute value there (D-impl-relative-servers).
//! Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{JsonPing, ProblemDetails, XmlDocument};
use ::openapi_support::client_error::{BodyLimitDirection, ClientError};
use ::openapi_support::collect::collect_reqwest_limited;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::ParsedMediaType;
use ::quick_xml::de::from_reader;

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

/// Documented outcomes for `create_xml_document` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateXmlDocumentResponse {
    /// HTTP 201 Created.
    Created201(XmlDocument),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Streaming payload for status 200 of `put_cbor_state` (main spec §32): owns the response.
#[derive(Debug)]
pub struct PutCborState200 {
    pub response: ::reqwest::Response,
}

impl PutCborState200 {
    /// Consumes the wrapper into the raw chunk stream (main spec §32).
    pub fn into_bytes_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = ::reqwest::Result<::bytes::Bytes>> {
        self.response.bytes_stream()
    }
}

/// Documented outcomes for `put_cbor_state` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutCborStateResponse {
    /// HTTP 200 Ok.
    Ok200(PutCborState200),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Streaming payload for status 200 of `post_msg_pack_event` (main spec §32): owns the response.
#[derive(Debug)]
pub struct PostMsgPackEvent200 {
    pub response: ::reqwest::Response,
}

impl PostMsgPackEvent200 {
    /// Consumes the wrapper into the raw chunk stream (main spec §32).
    pub fn into_bytes_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = ::reqwest::Result<::bytes::Bytes>> {
        self.response.bytes_stream()
    }
}

/// Documented outcomes for `post_msg_pack_event` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostMsgPackEventResponse {
    /// HTTP 200 Ok.
    Ok200(PostMsgPackEvent200),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Documented outcomes for `echo_json` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum EchoJsonResponse {
    /// HTTP 200 Ok.
    Ok200(JsonPing),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

impl Client {
    /// `POST` `/xml/documents`.
    /// Operation `createXmlDocument`.
    pub async fn create_xml_document(
        &self,
        body: &XmlDocument,
    ) -> Result<CreateXmlDocumentResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/xml/documents");
        let payload = match xml_encode_limited(body, self.limits.structured_encode_bytes) {
            Ok(payload) => payload,
            Err(_) => return Err(encode_overflow_error(self.limits.structured_encode_bytes)),
        };
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "application/xml")
            .header(
                ::http::header::ACCEPT,
                "application/xml, application/problem+json",
            )
            .body(payload)
            .send()
            .await?;
        self.decode_create_xml_document(response).await
    }

    /// Shared decode tail for `create_xml_document` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_create_xml_document(
        &self,
        response: ::reqwest::Response,
    ) -> Result<CreateXmlDocumentResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::CREATED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/xml".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                let value: XmlDocument = xml_decode_typed(&bytes, Some(content_type))?;
                Ok(CreateXmlDocumentResponse::Created201(value))
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
                Ok(CreateXmlDocumentResponse::BadRequest400(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `PUT` `/cbor/state`.
    /// Operation `putCborState`.
    pub async fn put_cbor_state(
        &self,
        body: ::reqwest::Body,
    ) -> Result<PutCborStateResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/cbor/state");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::PUT, &url)
            .header(::http::header::CONTENT_TYPE, "application/cbor")
            .header(
                ::http::header::ACCEPT,
                "application/cbor, application/problem+json",
            )
            .body(body)
            .send()
            .await?;
        self.decode_put_cbor_state(response).await
    }

    /// Shared decode tail for `put_cbor_state` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    /// Called by `put_cbor_state` and its §31 `put_cbor_state_replaying` twin so both share one classification path.
    #[allow(clippy::unused_async)]
    async fn decode_put_cbor_state(
        &self,
        response: ::reqwest::Response,
    ) -> Result<PutCborStateResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => Ok(PutCborStateResponse::Ok200(PutCborState200 { response })),
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
                Ok(PutCborStateResponse::BadRequest400(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `PUT` `/cbor/state` with explicit-factory retries (§31/D-impl-retry).
    /// Operation `putCborState`.
    /// Idempotency is the caller's responsibility: PUT-style operations are natural fits; retrying POST may duplicate effects.
    /// Every attempt rebuilds the streaming body through `body_factory`; multipart-free raw payloads are never buffered for replay.
    /// Only PRE-response transport failures classified by `openapi_support::retry::is_retryable_transport` are retried — once response headers arrive the outcome is final; factory errors abort without retry.
    pub async fn put_cbor_state_replaying<F, Fut>(
        &self,
        body_factory: F,
        policy: ::openapi_support::retry::RetryPolicy,
    ) -> Result<PutCborStateResponse, ClientError>
    where
        F: Fn() -> Fut,
        Fut: ::std::future::Future<Output = Result<::reqwest::Body, ClientError>>,
    {
        let mut url = self.base_url.clone();
        url.push_str("/cbor/state");
        let budget = policy.max_attempts.max(1);
        let mut failed = 0_u32;
        loop {
            let body = (body_factory)().await?;
            let mut request = self.http.request(::http::Method::PUT, &url);
            request = request
                .header(::http::header::CONTENT_TYPE, "application/cbor")
                .body(body);
            request = request.header(
                ::http::header::ACCEPT,
                "application/cbor, application/problem+json",
            );
            let response = request.send().await;
            match response {
                Ok(response) => return self.decode_put_cbor_state(response).await,
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

    /// `POST` `/msgpack/events`.
    /// Operation `postMsgPackEvent`.
    pub async fn post_msg_pack_event(
        &self,
        body: ::reqwest::Body,
    ) -> Result<PostMsgPackEventResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/msgpack/events");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "application/msgpack")
            .header(
                ::http::header::ACCEPT,
                "application/msgpack, application/problem+json",
            )
            .body(body)
            .send()
            .await?;
        self.decode_post_msg_pack_event(response).await
    }

    /// Shared decode tail for `post_msg_pack_event` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    /// Called by `post_msg_pack_event` and its §31 `post_msg_pack_event_replaying` twin so both share one classification path.
    #[allow(clippy::unused_async)]
    async fn decode_post_msg_pack_event(
        &self,
        response: ::reqwest::Response,
    ) -> Result<PostMsgPackEventResponse, ClientError> {
        match response.status() {
            ::http::StatusCode::OK => Ok(PostMsgPackEventResponse::Ok200(PostMsgPackEvent200 {
                response,
            })),
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
                Ok(PostMsgPackEventResponse::BadRequest400(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/msgpack/events` with explicit-factory retries (§31/D-impl-retry).
    /// Operation `postMsgPackEvent`.
    /// Idempotency is the caller's responsibility: PUT-style operations are natural fits; retrying POST may duplicate effects.
    /// Every attempt rebuilds the streaming body through `body_factory`; multipart-free raw payloads are never buffered for replay.
    /// Only PRE-response transport failures classified by `openapi_support::retry::is_retryable_transport` are retried — once response headers arrive the outcome is final; factory errors abort without retry.
    pub async fn post_msg_pack_event_replaying<F, Fut>(
        &self,
        body_factory: F,
        policy: ::openapi_support::retry::RetryPolicy,
    ) -> Result<PostMsgPackEventResponse, ClientError>
    where
        F: Fn() -> Fut,
        Fut: ::std::future::Future<Output = Result<::reqwest::Body, ClientError>>,
    {
        let mut url = self.base_url.clone();
        url.push_str("/msgpack/events");
        let budget = policy.max_attempts.max(1);
        let mut failed = 0_u32;
        loop {
            let body = (body_factory)().await?;
            let mut request = self.http.request(::http::Method::POST, &url);
            request = request
                .header(::http::header::CONTENT_TYPE, "application/msgpack")
                .body(body);
            request = request.header(
                ::http::header::ACCEPT,
                "application/msgpack, application/problem+json",
            );
            let response = request.send().await;
            match response {
                Ok(response) => return self.decode_post_msg_pack_event(response).await,
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

    /// `POST` `/json/echo`.
    /// Operation `echoJson`.
    pub async fn echo_json(&self, body: &JsonPing) -> Result<EchoJsonResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/json/echo");
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
        self.decode_echo_json(response).await
    }

    /// Shared decode tail for `echo_json` (main spec §23–§28): classifies the received response into its exhaustive documented-status enum.
    #[allow(clippy::unused_async)]
    async fn decode_echo_json(
        &self,
        response: ::reqwest::Response,
    ) -> Result<EchoJsonResponse, ClientError> {
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
                let value: JsonPing = json_decode(&bytes, Some(content_type))?;
                Ok(EchoJsonResponse::Ok200(value))
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
                Ok(EchoJsonResponse::BadRequest400(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }
}

struct XmlFmtSink<'a, W>(&'a mut W);

impl<W: ::std::io::Write> ::std::fmt::Write for XmlFmtSink<'_, W> {
    fn write_str(&mut self, text: &str) -> ::std::fmt::Result {
        self.0
            .write_all(text.as_bytes())
            .map_err(|_| ::std::fmt::Error)
    }
}

/// Bounded `xml` request serialization (main spec §45/§34): bytes stream through the fail-fast counting writer, so an oversized document returns `BodyTooLarge` before any wire traffic and no partial output escapes.
/// requires the `serialize` feature of quick-xml.
#[allow(clippy::let_and_return)]
fn xml_encode_limited<T>(
    value: &T,
    limit: usize,
) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>
where
    T: serde::Serialize,
{
    let encoded = ::openapi_support::encode::serialize_with_writer_limited(limit, |writer| {
        let mut sink = XmlFmtSink(writer);
        let serializer = ::quick_xml::se::Serializer::with_root(&mut sink, Some("root"))
            .map_err(::std::io::Error::other)?;
        ::serde::Serialize::serialize(value, serializer)
            .map(|_verdict| ())
            .map_err(::std::io::Error::other)
    });
    encoded
}

/// Typed `xml` response decode from ALREADY-bounded bytes (main spec §45): any codec failure maps onto `ClientError::Decode` with the negotiated content type.
/// The schema/data distinction is not portable across codecs; all decode failures surface as decode errors.
/// requires the `serialize` feature of quick-xml.
#[allow(clippy::missing_errors_doc)]
fn xml_decode_typed<T>(bytes: &[u8], content_type: Option<::mime::Mime>) -> Result<T, ClientError>
where
    T: serde::de::DeserializeOwned,
{
    from_reader(bytes).map_err(|error| ClientError::Decode {
        content_type: content_type.clone(),
        source: Box::new(error),
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
