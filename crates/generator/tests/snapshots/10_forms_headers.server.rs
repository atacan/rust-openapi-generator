/// Axum server generated from the OpenAPI document (main spec §8 Output B).
///
/// Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form extractor is never used — routes self-decode after the §28 Content-Type dispatch), streaming raw payloads (§32), typed documented response headers (§15: IntoResponse converts stored domain values through the well-defined internal error path of §48, firing the encode hook and emitting the fixed empty 500 on failure), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{CacheEntryForm, CreateSessionForm, ProblemDetails, Session};
use ::axum::response::IntoResponse;
use ::openapi_support::collect::{collect_body_limited, CollectLimitedError};
use ::openapi_support::content_coding::ensure_identity_content_coding;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::form::decode_form_limited;
use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::{
    is_wildcard_incoming, match_entry, parse_content_type, EntryMatch, ParsedMediaType,
};
use ::openapi_support::params::{decode_path_segment, ParamSpec, ParamStyle, ParamValue};
use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};
use ::std::collections::HashMap;

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

/// Payload for status 404 of `get_session`: typed payload (main spec §15 Output B): required documented headers as plain fields, optional ones as `Option<T>`, then the body stored as a domain value for the bounded encoder.
#[derive(Debug)]
pub struct GetSession404 {
    /// Documented response header `X-Request-Id` (required).
    pub x_request_id: String,
    pub body: ProblemDetails,
}

/// Documented outcomes for `get_session` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetSessionResponse {
    /// HTTP 200 Ok.
    Ok200(Session),
    /// HTTP 404 NotFound.
    NotFound404(GetSession404),
}

/// Bounded encoder for [`GetSessionResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl GetSessionResponse {
    /// Encodes the documented outcome with the configured limits.
    #[allow(clippy::vec_init_then_push)]
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
                "getSession",
                "Ok200",
            ),
            Self::NotFound404(wrapper) => {
                let mut typed_headers = Vec::<(&'static str, String)>::new();
                typed_headers.push(("x-request-id", wrapper.x_request_id.clone()));
                let encoded = encode_json_limited(
                    ::http::StatusCode::NOT_FOUND,
                    "application/problem+json",
                    &wrapper.body,
                    limits,
                    hook,
                    "getSession",
                    "NotFound404",
                );
                write_typed_headers(encoded, hook, "getSession", "NotFound404", &typed_headers)
            }
        }
    }
}

/// Checked payload constructors validating every convertible documented header eagerly (main spec §15/§48).
/// Checked constructor for [`GetSession404`] (§48).
impl GetSession404 {
    pub fn new(
        x_request_id: String,
        body: ProblemDetails,
    ) -> Result<Self, ::openapi_support::response_headers::InvalidResponseHeader> {
        ::openapi_support::response_headers::checked_value("x-request-id", &x_request_id)?;
        Ok(Self { x_request_id, body })
    }
}

impl ::axum::response::IntoResponse for GetSessionResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `put_cache_entry` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutCacheEntryResponse {
    /// HTTP 204 NoContent.
    NoContent204,
}

/// Bounded encoder for [`PutCacheEntryResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PutCacheEntryResponse {
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

impl ::axum::response::IntoResponse for PutCacheEntryResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `POST` `/sessions`.
    /// Operation `createSession`.
    async fn create_session(&self, body: CreateSessionForm) -> CreateSessionResponse;

    /// `GET` `/sessions/{id}`.
    /// Operation `getSession`.
    async fn get_session(&self, id: String) -> GetSessionResponse;

    /// `PUT` `/cache/{key}`.
    /// Operation `putCacheEntry`.
    async fn put_cache_entry(&self, key: String, body: CacheEntryForm) -> PutCacheEntryResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
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

/// Route handler for `GET` `/sessions/{id}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_get_session(
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
    let response = api.get_session(id).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `PUT` `/cache/{key}` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_put_cache_entry(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    ::axum::extract::Path(__path): ::axum::extract::Path<HashMap<String, String>>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let spec = ParamSpec::new("key", ParamStyle::Simple, false, false);
    let raw_segment = match __path.get("key") {
        Some(segment) => segment.as_str(),
        None => {
            return Err(invalid_parameter("missing path parameter `key`"));
        }
    };
    let decoded = match decode_path_segment(&spec, raw_segment) {
        Ok(value) => value,
        Err(_) => {
            return Err(invalid_parameter("path parameter `key` is malformed"));
        }
    };
    let key: String = expect_text(decoded, "key")?;
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
                let value: CacheEntryForm =
                    decode_form_body(&bytes, limits.structured_request_bytes)?;
                value
            }
            RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
        };
    let api = __state.api.as_ref();
    let response = api.put_cache_entry(key, request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Builds the Axum router serving every documented operation (main spec §38): buffered-body routes install `DefaultBodyLimit` at `structured_request_bytes`; streaming-body routes remain exempt because nothing aggregates them.
pub fn router(
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
) -> ::axum::Router {
    let state = ServerState {
        api,
        limits,
        encode_overflow_hook,
    };
    ::axum::Router::new()
        .route(
            "/sessions",
            ::axum::routing::post(route_create_session).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route("/sessions/{id}", ::axum::routing::get(route_get_session))
        .route(
            "/cache/{key}",
            ::axum::routing::put(route_put_cache_entry).layer(
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
