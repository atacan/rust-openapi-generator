/// Reqwest client generated from the OpenAPI document (main spec §8 Output A).
///
/// Bounded JSON/form bodies (§34), streaming raw payloads (§32), exhaustive documented-status enums (§2.4), typed documented response headers (§15), redirects off by default (§30.1), and the authoritative `ClientError` (§36). Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
///
/// Servers (companion §8): operation-level `servers` override path-level, path-level overrides root-level, and within each effective array the first entry is that operation's default base. Every DISTINCT effective default URL becomes its own stored base: `base_url` is the primary (the first operation's first effective server); further bases live in `base_url_<key>` fields whose keys are documented under `ClientBuilder::secondary_base_url`. Recorded decision: an explicit `base_url` replaces ONLY the primary base; each other base needs its own `secondary_base_url` override, so a relative secondary still requires an absolute value there (D-impl-relative-servers).
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::ProblemDetails;
use ::openapi_support::client_error::ClientError;
use ::openapi_support::collect::collect_reqwest_limited;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::ParsedMediaType;
use ::openapi_support::params::{encode_path, ParamSpec, ParamStyle, ParamValue};

/// Client carrying one resolved base per distinct effective default server (companion §8): `base_url` is the PRIMARY base (the first operation's first effective server); every further distinct URL gets its own `base_url_<key>` field, and each generated method sends through its operation's own base.
#[derive(Clone)]
pub struct Client {
    http: ::reqwest::Client,
    base_url: String,
    limits: BodyLimits,
    base_url_storage: String,
}

/// Builder for `Client` (main spec §30.1): redirects disabled unless opted in through `follow_redirects`; relative default servers require explicit overrides (D-impl-relative-servers). Recorded decision (companion §8): an explicit `base_url` replaces ONLY the primary base; every additional base is overridden per key through `secondary_base_url`.
pub struct ClientBuilder {
    http: ::reqwest::ClientBuilder,
    base_url: Option<String>,
    limits: BodyLimits,
    default_server_url: String,
    default_server_variables: Vec<(String, String, Option<Vec<String>>)>,
    secondary_base_urls: ::std::collections::BTreeMap<String, String>,
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
        let default_server_url = "https://{region}.api.example.com/v1".to_owned();
        let default_server_variables = vec![server_variable(
            "region",
            "us-east",
            &["us-east", "eu-west"],
        )];
        Self {
            http: ::reqwest::Client::builder().redirect(::reqwest::redirect::Policy::none()),
            base_url: None,
            limits: BodyLimits::process_default(),
            default_server_url,
            default_server_variables,
            secondary_base_urls: ::std::collections::BTreeMap::new(),
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

    /// Overrides ONE secondary base URL by its documented key (companion §8: every distinct effective default server generates its own base).
    /// An explicit `base_url` never affects these bases — it replaces only the primary (recorded decision); a relative secondary URL therefore REQUIRES an absolute value here (D-impl-relative-servers).
    /// Keys are deterministic snake_case derivations of each server URL; declared keys for this client:
    /// - `storage`: `/storage`
    pub fn secondary_base_url(mut self, key: &str, value: impl Into<String>) -> Self {
        self.secondary_base_urls
            .insert(key.to_owned(), value.into());
        self
    }

    /// Server variable `{region}` (declared default `us-east`; allowed values: us-east, eu-west)
    /// One builder method per variable name controls EVERY base that declares it (companion §8); enum validation against the declared allowed values happens at `build` time.
    pub fn region(mut self, value: impl Into<String>) -> Self {
        self.server_variables
            .insert("region".to_owned(), value.into());
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
        let storage_override = self.secondary_base_urls.get("storage").cloned();
        let url_storage = match storage_override {
            Some(explicit) => explicit,
            None => substitute_server_variables("/storage", &[], &self.server_variables)?,
        };
        let trimmed_storage = url_storage.trim_end_matches('/');
        if !is_absolute_url(trimmed_storage) {
            return Err(ClientError::InvalidUrl(format!(
                "secondary base `storage` URL `{trimmed_storage}` is not absolute; \
                 call `secondary_base_url` with an absolute value"
            )));
        }
        let http = self.http.build().map_err(ClientError::Transport)?;
        Ok(Client {
            http,
            base_url: trimmed.to_owned(),
            limits: self.limits,
            base_url_storage: trimmed_storage.to_owned(),
        })
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

impl Client {
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
        let mut url = self.base_url_storage.clone();
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

/// Typed documented response headers (main spec §15): required headers missing from the response are protocol errors (`MissingRequiredHeader`), values failing their Rust type are `InvalidHeader`; both surface BEFORE the body is consumed. A repeated documented header reads its first occurrence.
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

/// One declared server variable in builder-ready form.
#[must_use]
fn server_variable(
    name: &str,
    default: &str,
    allowed: &[&str],
) -> (String, String, Option<Vec<String>>) {
    let allowed = if allowed.is_empty() {
        None
    } else {
        Some(allowed.iter().map(|value| (*value).to_owned()).collect())
    };
    (name.to_owned(), default.to_owned(), allowed)
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
