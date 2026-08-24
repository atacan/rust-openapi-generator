/// Axum server generated from the OpenAPI document (main spec §8 Output B).
///
/// Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form extractor is never used — routes self-decode after the §28 Content-Type dispatch), streaming raw payloads (§32), typed documented response headers (§15: IntoResponse converts stored domain values through the well-defined internal error path of §48, firing the encode hook and emitting the fixed empty 500 on failure), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{Ack, Event, EventPayload, Metric, ProblemDetails, Record};
use ::axum::response::IntoResponse;
use ::openapi_support::content_coding::ensure_identity_content_coding;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use ::openapi_support::jsonseq::decode_jsonseq;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::{
    is_wildcard_incoming, match_entry, parse_content_type, EntryMatch, ParsedMediaType,
};
use ::openapi_support::ndjson::encode_ndjson_item;
use ::openapi_support::peek::{detect_body_presence, BodyPresence};
use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};
use ::openapi_support::sse::encode_sse_event;
use ::openapi_support::stream_errors::JsonSeqDecodeError;
use ::openapi_support::stream_errors::ServerStreamError;

/// Boxed erased item stream shared by every generated streaming                  wrapper (§18–§20): producers box their producer stream into                  this type.
#[doc(hidden)]
pub type ErasedItems<T> =
    ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>;

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

/// Erased `sse` item stream for status 200 of `stream_envelope_events` (main spec §18 Output B / §40): failures after commit ride `ServerStreamError` — no fabricated statuses.
pub type StreamEnvelopeEvents200Stream = ErasedItems<Result<EventPayload, ServerStreamError>>;

/// Documented outcomes for `stream_envelope_events` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
pub enum StreamEnvelopeEventsResponse {
    /// HTTP 200 Ok.
    Ok200(StreamEnvelopeEvents200Stream),
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Bounded encoder for [`StreamEnvelopeEventsResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl StreamEnvelopeEventsResponse {
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
                    "streamEnvelopeEvents",
                    encode_sse_event::<EventPayload>,
                ));
                encoded
            }
            Self::NotFound404(value) => encode_json_limited(
                ::http::StatusCode::NOT_FOUND,
                "application/problem+json",
                &value,
                limits,
                hook,
                "streamEnvelopeEvents",
                "NotFound404",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for StreamEnvelopeEventsResponse {
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

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `GET` `/records/export`.
    /// Operation `exportRecords`.
    async fn export_records(&self) -> ExportRecordsResponse;

    /// `GET` `/events`.
    /// Operation `streamEvents`.
    async fn stream_events(&self) -> StreamEventsResponse;

    /// `GET` `/envelope-events`.
    /// Operation `streamEnvelopeEvents`.
    async fn stream_envelope_events(&self) -> StreamEnvelopeEventsResponse;

    /// `POST` `/metrics`.
    /// Operation `pushMetrics`.
    async fn push_metrics(&self, body: PushMetricsJsonSeqInput) -> PushMetricsResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
    stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,
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

/// Route handler for `GET` `/envelope-events` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_stream_envelope_events(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let api = __state.api.as_ref();
    let response = api.stream_envelope_events().await;
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
            "/records/export",
            ::axum::routing::get(route_export_records),
        )
        .route("/events", ::axum::routing::get(route_stream_events))
        .route(
            "/envelope-events",
            ::axum::routing::get(route_stream_envelope_events),
        )
        .route("/metrics", ::axum::routing::post(route_push_metrics))
        .with_state(state)
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
