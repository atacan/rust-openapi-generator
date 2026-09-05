//! Axum server generated from the OpenAPI document (main spec §8 Output B).
//!
//! Mode A traits (§37), bounded JSON/form bodies (§34; axum's Form extractor is never used — routes self-decode after the §28 Content-Type dispatch), streaming raw payloads (§32), typed documented response headers (§15: IntoResponse converts stored domain values through the well-defined internal error path of §48, firing the encode hook and emitting the fixed empty 500 on failure), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
//! Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::Widget;
use ::axum::response::IntoResponse;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::params::{decode_path_segment, ParamSpec, ParamStyle, ParamValue};
use ::openapi_support::rejection::{ProtocolRejection, RejectionKind};
use ::std::collections::HashMap;

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

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `GET` `/widgets/{id}`.
    /// Operation `getWidget`.
    async fn get_widget(&self, id: String) -> GetWidgetResponse;

    /// `HEAD` `/widgets/{id}`.
    /// Operation `headWidget`.
    async fn head_widget(&self, id: String) -> HeadWidgetResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
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
        .route("/widgets/{id}", ::axum::routing::get(route_get_widget))
        .route("/widgets/{id}", ::axum::routing::head(route_head_widget))
        .with_state(state)
}

/// Canonical §39 mapping row 1: invalid or missing required  path/query/header parameter → 400.
fn invalid_parameter(detail: impl Into<::std::borrow::Cow<'static, str>>) -> ProtocolRejection {
    ProtocolRejection::new(RejectionKind::InvalidParameter).with_detail(detail)
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
