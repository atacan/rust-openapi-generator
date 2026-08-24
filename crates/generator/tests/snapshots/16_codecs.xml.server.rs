/// Axum server generated from the OpenAPI document (main spec §8 Output B).
///
/// Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form extractor is never used — routes self-decode after the §28 Content-Type dispatch), streaming raw payloads (§32), typed documented response headers (§15: IntoResponse converts stored domain values through the well-defined internal error path of §48, firing the encode hook and emitting the fixed empty 500 on failure), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{JsonPing, ProblemDetails, XmlDocument};
use ::axum::response::IntoResponse;
use ::openapi_support::collect::{collect_body_limited, CollectLimitedError};
use ::openapi_support::content_coding::ensure_identity_content_coding;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::{
    is_wildcard_incoming, match_entry, parse_content_type, EntryMatch, ParsedMediaType,
};
use ::openapi_support::peek::{detect_body_presence, BodyPresence};
use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};
use ::quick_xml::de::from_reader;

/// Documented outcomes for `create_xml_document` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum CreateXmlDocumentResponse {
    /// HTTP 201 Created.
    Created201(XmlDocument),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Bounded encoder for [`CreateXmlDocumentResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl CreateXmlDocumentResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Created201(value) => xml_encode_limited(
                ::http::StatusCode::CREATED,
                "application/xml",
                &value,
                limits,
                hook,
                "createXmlDocument",
                "Created201",
            ),
            Self::BadRequest400(value) => encode_json_limited(
                ::http::StatusCode::BAD_REQUEST,
                "application/problem+json",
                &value,
                limits,
                hook,
                "createXmlDocument",
                "BadRequest400",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for CreateXmlDocumentResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Payload for status 200 of `put_cbor_state` (main spec §32): the body streams verbatim; typed documented-header fields ride beside it where documented (main spec §15/§32).
#[derive(Debug)]
pub struct PutCborState200 {
    pub body: ::axum::body::Body,
}

/// Documented outcomes for `put_cbor_state` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PutCborStateResponse {
    /// HTTP 200 Ok.
    Ok200(PutCborState200),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Bounded encoder for [`PutCborStateResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PutCborStateResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(wrapper) => {
                stream_response(::http::StatusCode::OK, "application/cbor", wrapper.body)
            }
            Self::BadRequest400(value) => encode_json_limited(
                ::http::StatusCode::BAD_REQUEST,
                "application/problem+json",
                &value,
                limits,
                hook,
                "putCborState",
                "BadRequest400",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for PutCborStateResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Payload for status 200 of `post_msg_pack_event` (main spec §32): the body streams verbatim; typed documented-header fields ride beside it where documented (main spec §15/§32).
#[derive(Debug)]
pub struct PostMsgPackEvent200 {
    pub body: ::axum::body::Body,
}

/// Documented outcomes for `post_msg_pack_event` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PostMsgPackEventResponse {
    /// HTTP 200 Ok.
    Ok200(PostMsgPackEvent200),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Bounded encoder for [`PostMsgPackEventResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl PostMsgPackEventResponse {
    /// Encodes the documented outcome with the configured limits.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
        hook: &dyn EncodeOverflowHook,
    ) -> ::axum::response::Response {
        match self {
            Self::Ok200(wrapper) => {
                stream_response(::http::StatusCode::OK, "application/msgpack", wrapper.body)
            }
            Self::BadRequest400(value) => encode_json_limited(
                ::http::StatusCode::BAD_REQUEST,
                "application/problem+json",
                &value,
                limits,
                hook,
                "postMsgPackEvent",
                "BadRequest400",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for PostMsgPackEventResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Documented outcomes for `echo_json` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum EchoJsonResponse {
    /// HTTP 200 Ok.
    Ok200(JsonPing),
    /// HTTP 400 BadRequest.
    BadRequest400(ProblemDetails),
}

/// Bounded encoder for [`EchoJsonResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl EchoJsonResponse {
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
                "echoJson",
                "Ok200",
            ),
            Self::BadRequest400(value) => encode_json_limited(
                ::http::StatusCode::BAD_REQUEST,
                "application/problem+json",
                &value,
                limits,
                hook,
                "echoJson",
                "BadRequest400",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for EchoJsonResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `POST` `/xml/documents`.
    /// Operation `createXmlDocument`.
    async fn create_xml_document(&self, body: XmlDocument) -> CreateXmlDocumentResponse;

    /// `PUT` `/cbor/state`.
    /// Operation `putCborState`.
    async fn put_cbor_state(&self, body: ::axum::body::Body) -> PutCborStateResponse;

    /// `POST` `/msgpack/events`.
    /// Operation `postMsgPackEvent`.
    async fn post_msg_pack_event(&self, body: ::axum::body::Body) -> PostMsgPackEventResponse;

    /// `POST` `/json/echo`.
    /// Operation `echoJson`.
    async fn echo_json(&self, body: JsonPing) -> EchoJsonResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
}

/// Route handler for `POST` `/xml/documents` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_create_xml_document(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/xml"]) {
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
            let bytes = body_bytes(body, limits.structured_request_bytes).await?;
            if bytes.is_empty() {
                return Err(malformed_body("documented request body arrived empty"));
            }
            let value: XmlDocument = xml_decode_body(&bytes)?;
            require_valid_request("body", value.validate_request())?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.create_xml_document(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `PUT` `/cbor/state` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_put_cbor_state(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/cbor"]) {
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
    let response = api.put_cbor_state(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/msgpack/events` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_post_msg_pack_event(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
    __headers: ::http::HeaderMap,
    body: ::axum::body::Body,
) -> Result<::axum::response::Response, ProtocolRejection> {
    ensure_identity_content_coding(&__headers)?;
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let parsed = parse_single_content_type(&__headers)?;
    let request_body = match classify_request_entry(parsed.as_ref(), &["application/msgpack"]) {
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
    let response = api.post_msg_pack_event(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Route handler for `POST` `/json/echo` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_echo_json(
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
            let value: JsonPing = decode_json_body(&bytes)?;
            value
        }
        RequestEntryMatch::Entry(_) => unreachable!("request entry index out of range"),
    };
    let api = __state.api.as_ref();
    let response = api.echo_json(request_body).await;
    Ok(response.into_response_with_limits(&limits, hook))
}

/// Builds the Axum router serving every documented operation (main spec §38): buffered-body routes install `DefaultBodyLimit` at `structured_request_bytes`; streaming-body routes remain exempt because nothing aggregates them.
pub fn router(
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
    _stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
) -> ::axum::Router {
    let state = ServerState {
        api,
        limits,
        encode_overflow_hook,
    };
    ::axum::Router::new()
        .route(
            "/xml/documents",
            ::axum::routing::post(route_create_xml_document).layer(
                ::axum::extract::DefaultBodyLimit::max(limits.structured_request_bytes),
            ),
        )
        .route("/cbor/state", ::axum::routing::put(route_put_cbor_state))
        .route(
            "/msgpack/events",
            ::axum::routing::post(route_post_msg_pack_event),
        )
        .route(
            "/json/echo",
            ::axum::routing::post(route_echo_json).layer(::axum::extract::DefaultBodyLimit::max(
                limits.structured_request_bytes,
            )),
        )
        .with_state(state)
}

/// quick-xml serializes through `std::fmt::Write`; this sink forwards the UTF-8 text into the byte-counting writer unchanged.
struct XmlFmtSink<'a, W>(&'a mut W);

impl<W: ::std::io::Write> ::std::fmt::Write for XmlFmtSink<'_, W> {
    fn write_str(&mut self, text: &str) -> ::std::fmt::Result {
        self.0
            .write_all(text.as_bytes())
            .map_err(|_| ::std::fmt::Error)
    }
}

/// Typed `xml` request-body decode from ALREADY-bounded bytes (main spec §45, D-impl-codec-plugins): the §28 Content-Type gate and the `structured_request_bytes` collection ran BEFORE this parse.
/// The schema/data distinction is not portable across codecs, so ALL decode failures — including missing-required-style errors — map onto MalformedBody 400 (documented deviation from §39 row 6).
#[allow(clippy::missing_errors_doc)]
fn xml_decode_body<T>(bytes: &[u8]) -> Result<T, ProtocolRejection>
where
    T: serde::de::DeserializeOwned,
{
    from_reader(bytes).map_err(|_| malformed_body("malformed body"))
}

/// Bounded `xml` response encoding (main spec §45/§34/§41); the literal keeps distinct types such as application/xml separate from text/xml.
/// Bytes stream through the fail-fast counting writer: overflow discards partial output, fires the hook, and emits the fixed empty 500 (§34.1).
fn xml_encode_limited<T>(
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
    let encoded = ::openapi_support::encode::serialize_with_writer_limited(budget, |writer| {
        let mut sink = XmlFmtSink(writer);
        let serializer = ::quick_xml::se::Serializer::with_root(&mut sink, Some("root"))
            .map_err(::std::io::Error::other)?;
        ::serde::Serialize::serialize(value, serializer)
            .map(|_verdict| ())
            .map_err(::std::io::Error::other)
    });
    match encoded {
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
