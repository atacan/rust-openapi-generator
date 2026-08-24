/// Reqwest client generated from the OpenAPI document (main spec §8 Output A).
///
/// Bounded JSON/form bodies (§34), streaming raw payloads (§32), exhaustive documented-status enums (§2.4), typed documented response headers (§15), redirects off by default (§30.1), and the authoritative `ClientError` (§36). Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
///
/// Servers (companion §8): operation-level `servers` override path-level, path-level overrides root-level, and within each effective array the first entry is that operation's default base. Every DISTINCT effective default URL becomes its own stored base: `base_url` is the primary (the first operation's first effective server); further bases live in `base_url_<key>` fields whose keys are documented under `ClientBuilder::secondary_base_url`. Recorded decision: an explicit `base_url` replaces ONLY the primary base; each other base needs its own `secondary_base_url` override, so a relative secondary still requires an absolute value there (D-impl-relative-servers).
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{LegacyError, Payload, ProblemDetails, Report};
use ::openapi_support::client_error::{BodyLimitDirection, ClientError};
use ::openapi_support::collect::collect_reqwest_limited;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::{match_entry, ParsedMediaType};
use ::openapi_support::params::{encode_path, ParamSpec, ParamStyle, ParamValue};

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

/// Documented representations for status 200 of `get_report` (main spec §11): the client negotiates via Content-Type (§28).
#[derive(Debug)]
pub enum GetReport200Content {
    Json(Report),
    OctetStream(::reqwest::Response),
}

/// Documented representations for status 400 of `get_report` (main spec §11): the client negotiates via Content-Type (§28).
#[derive(Debug)]
pub enum GetReport400Content {
    ProblemJson(ProblemDetails),
    Json(LegacyError),
    TextPlain(String),
}

/// Documented outcomes for `get_report` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetReportResponse {
    /// HTTP 200 Ok.
    Ok200(GetReport200Content),
    /// HTTP 400 BadRequest.
    BadRequest400(GetReport400Content),
}

/// Request payloads for `post_mirror` (main spec §12/§43): owning variants (D-§51.3); streaming variants attach `reqwest::Body` or a boxed item-stream verbatim.
#[derive(Debug)]
pub enum PostMirrorRequestBody {
    Json(Payload),
    Any(::reqwest::Body),
}

/// Documented outcomes for `post_mirror` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostMirrorResponse {
    /// HTTP 202 Accepted.
    Accepted202,
}

/// Streaming payload for status 200 of `get_raw_text` (main spec §32): owns the response.
#[derive(Debug)]
pub struct GetRawText200 {
    pub response: ::reqwest::Response,
}

impl GetRawText200 {
    /// Consumes the wrapper into the raw chunk stream (main spec §32).
    pub fn into_bytes_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = ::reqwest::Result<::bytes::Bytes>> {
        self.response.bytes_stream()
    }
}

/// Documented outcomes for `get_raw_text` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetRawTextResponse {
    /// HTTP 200 Ok.
    Ok200(GetRawText200),
}

/// Documented representations for status 200 of `get_either` (main spec §11): the client negotiates via Content-Type (§28).
#[derive(Debug)]
pub enum GetEither200Content {
    Json(Payload),
    Any(::reqwest::Response),
}

/// Documented outcomes for `get_either` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetEitherResponse {
    /// HTTP 200 Ok.
    Ok200(GetEither200Content),
}

/// Documented outcomes for `put_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutNoteResponse {
    /// HTTP 204 NoContent.
    NoContent204,
}

/// Documented outcomes for `post_stream_note` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostStreamNoteResponse {
    /// HTTP 204 NoContent.
    NoContent204,
}

impl Client {
    /// `GET` `/reports/{id}`.
    /// Operation `getReport`.
    pub async fn get_report(&self, id: &str) -> Result<GetReportResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/reports/");
        let spec = ParamSpec::new("id", ParamStyle::Simple, false, false);
        let value = ParamValue::Text(id.to_owned());
        url.push_str(&encode_path(&spec, &value));
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "application/json, application/octet-stream, application/problem+json, text/plain",
            )
            .send()
            .await?;
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec![
                            "application/json".to_owned(),
                            "application/octet-stream".to_owned(),
                        ],
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
                if let Some(rank) = match_entry(&parsed, "application/octet-stream") {
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
                        let value: Report = json_decode(&bytes, Some(content_type))?;
                        let payload = GetReport200Content::Json(value);
                        Ok(GetReportResponse::Ok200(payload))
                    }
                    Some(1) => {
                        let payload = GetReport200Content::OctetStream(response);
                        Ok(GetReportResponse::Ok200(payload))
                    }
                    _ => Err(ClientError::UnexpectedContentType {
                        expected: vec![
                            "application/json".to_owned(),
                            "application/octet-stream".to_owned(),
                        ],
                        actual: Some(mime_of(&parsed)?),
                    }),
                }
            }
            ::http::StatusCode::BAD_REQUEST => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec![
                            "application/problem+json".to_owned(),
                            "application/json".to_owned(),
                            "text/plain".to_owned(),
                        ],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                let mut best_rank: Option<u8> = None;
                let mut best_index: usize = 0;
                if let Some(rank) = match_entry(&parsed, "application/problem+json") {
                    let rank = negotiation_rank(rank);
                    if best_rank.is_none_or(|seen| rank < seen) {
                        best_rank = Some(rank);
                        best_index = 0;
                    }
                }
                if let Some(rank) = match_entry(&parsed, "application/json") {
                    let rank = negotiation_rank(rank);
                    if best_rank.is_none_or(|seen| rank < seen) {
                        best_rank = Some(rank);
                        best_index = 1;
                    }
                }
                if let Some(rank) = match_entry(&parsed, "text/plain") {
                    let rank = negotiation_rank(rank);
                    if best_rank.is_none_or(|seen| rank < seen) {
                        best_rank = Some(rank);
                        best_index = 2;
                    }
                }
                let selected = best_rank.is_some().then_some(best_index);
                match selected {
                    Some(0) => {
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
                        let payload = GetReport400Content::ProblemJson(value);
                        Ok(GetReportResponse::BadRequest400(payload))
                    }
                    Some(1) => {
                        ensure_utf8_charset(&parsed)?;
                        let limit = self.limits.error_response_bytes;
                        let bytes = collect_reqwest_limited(response, limit).await?;
                        if bytes.is_empty() {
                            return Err(ClientError::Decode {
                                content_type: Some(content_type),
                                source: Box::new(EmptyJsonBody),
                            });
                        }
                        let value: LegacyError = json_decode(&bytes, Some(content_type))?;
                        let payload = GetReport400Content::Json(value);
                        Ok(GetReportResponse::BadRequest400(payload))
                    }
                    Some(2) => {
                        ensure_utf8_charset(&parsed)?;
                        let limit = self.limits.error_response_bytes;
                        let bytes = collect_reqwest_limited(response, limit).await?;
                        let value = text_decode(bytes, Some(content_type))?;
                        let payload = GetReport400Content::TextPlain(value);
                        Ok(GetReportResponse::BadRequest400(payload))
                    }
                    _ => Err(ClientError::UnexpectedContentType {
                        expected: vec![
                            "application/problem+json".to_owned(),
                            "application/json".to_owned(),
                            "text/plain".to_owned(),
                        ],
                        actual: Some(mime_of(&parsed)?),
                    }),
                }
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/mirror`.
    /// Operation `postMirror`.
    pub async fn post_mirror(
        &self,
        body: PostMirrorRequestBody,
    ) -> Result<PostMirrorResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/mirror");
        let mut request = self.http.request(::http::Method::POST, &url);
        request = match body {
            PostMirrorRequestBody::Json(value) => {
                let payload =
                    match serialize_json_limited(&value, self.limits.structured_encode_bytes) {
                        Ok(payload) => payload,
                        Err(_) => {
                            return Err(encode_overflow_error(self.limits.structured_encode_bytes));
                        }
                    };
                request
                    .header(::http::header::CONTENT_TYPE, "application/json")
                    .body(payload)
            }
            PostMirrorRequestBody::Any(body) => request
                .header(::http::header::CONTENT_TYPE, "*/*")
                .body(body),
        };
        let response = request.send().await?;
        match response.status() {
            ::http::StatusCode::ACCEPTED => Ok(PostMirrorResponse::Accepted202),
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/raw-text`.
    /// Operation `getRawText`.
    pub async fn get_raw_text(&self) -> Result<GetRawTextResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/raw-text");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(::http::header::ACCEPT, "text/*")
            .send()
            .await?;
        match response.status() {
            ::http::StatusCode::OK => Ok(GetRawTextResponse::Ok200(GetRawText200 { response })),
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/either`.
    /// Operation `getEither`.
    pub async fn get_either(&self) -> Result<GetEitherResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/either");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(::http::header::ACCEPT, "application/json, */*")
            .send()
            .await?;
        match response.status() {
            ::http::StatusCode::OK => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned(), "*/*".to_owned()],
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
                if let Some(rank) = match_entry(&parsed, "*/*") {
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
                        let value: Payload = json_decode(&bytes, Some(content_type))?;
                        let payload = GetEither200Content::Json(value);
                        Ok(GetEitherResponse::Ok200(payload))
                    }
                    Some(1) => {
                        let payload = GetEither200Content::Any(response);
                        Ok(GetEitherResponse::Ok200(payload))
                    }
                    _ => Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned(), "*/*".to_owned()],
                        actual: Some(mime_of(&parsed)?),
                    }),
                }
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
        match response.status() {
            ::http::StatusCode::NO_CONTENT => Ok(PutNoteResponse::NoContent204),
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/stream-notes`.
    /// Operation `postStreamNote`.
    pub async fn post_stream_note(
        &self,
        body: ::reqwest::Body,
    ) -> Result<PostStreamNoteResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/stream-notes");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::POST, &url)
            .header(::http::header::CONTENT_TYPE, "text/plain")
            .body(body)
            .send()
            .await?;
        match response.status() {
            ::http::StatusCode::NO_CONTENT => Ok(PostStreamNoteResponse::NoContent204),
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

/// UTF-8 validation for bounded plain-text bodies (§28.4): invalid bytes are decode errors, never replacement characters.
#[allow(clippy::missing_errors_doc)]
fn text_decode(
    bytes: ::bytes::Bytes,
    content_type: Option<::mime::Mime>,
) -> Result<String, ClientError> {
    ::std::str::from_utf8(&bytes)
        .map(|text| text.to_owned())
        .map_err(|error| ClientError::Decode {
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
