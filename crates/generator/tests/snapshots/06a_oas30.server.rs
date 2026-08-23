/// Axum server generated from the OpenAPI document (main spec §8 Output B).
///
/// Mode A traits (§37), bounded JSON/text bodies (§34), streaming raw payloads (§32), pre-handler protocol rejections outside the documented enums (§39), identity-only inbound content coding (§30.4), and the §28 Content-Type dispatch state machine. The source document declares OpenAPI 3.0.3.
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::LegacyEnvelope;
use ::axum::response::IntoResponse;
use ::openapi_support::encode::serialize_json_limited;
use ::openapi_support::hooks::{EncodeOverflowHook, NoOpEncodeOverflowHook};
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::rejection::ProtocolRejection;

/// Documented outcomes for `get_legacy` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum GetLegacyResponse {
    /// HTTP 200 Ok.
    Ok200(LegacyEnvelope),
}

/// Bounded encoder for [`GetLegacyResponse`] (main spec §8 Output B, §41): JSON/text serialize under `structured_encode_bytes`; overflow discards partial output, fires the hook, and emits a fixed empty 500 (§34.1). Range/default statuses validate their carried status (§48).
impl GetLegacyResponse {
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
                "getLegacy",
                "Ok200",
            ),
        }
    }
}

impl ::axum::response::IntoResponse for GetLegacyResponse {
    fn into_response(self) -> ::axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default(), &NoOpEncodeOverflowHook)
    }
}

/// Application contract implemented by the service (main spec §37 Mode A): implementations translate internal failures into documented variants.
#[::async_trait::async_trait]
pub trait Api: Send + Sync + 'static {
    /// `GET` `/legacy`.
    /// Operation `getLegacy`.
    async fn get_legacy(&self) -> GetLegacyResponse;
}

/// Shared state threaded through every generated handler.
#[derive(Clone)]
struct ServerState {
    api: ::std::sync::Arc<dyn Api>,
    limits: BodyLimits,
    encode_overflow_hook: ::std::sync::Arc<dyn EncodeOverflowHook>,
}

/// Route handler for `GET` `/legacy` (main spec §38): identity-only content coding, parameter decoding, the §28 Content-Type state machine, and bounded collection all run before the application observes the request; every failure returns a `ProtocolRejection` outside the documented enum (§39 rule 1).
async fn route_get_legacy(
    ::axum::extract::State(__state): ::axum::extract::State<ServerState>,
) -> Result<::axum::response::Response, ProtocolRejection> {
    let limits = __state.limits;
    let hook = __state.encode_overflow_hook.as_ref();
    let api = __state.api.as_ref();
    let response = api.get_legacy().await;
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
        .route("/legacy", ::axum::routing::get(route_get_legacy))
        .with_state(state)
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
