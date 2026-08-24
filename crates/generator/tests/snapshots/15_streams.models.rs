/// Shared schema models generated from the OpenAPI document (main spec §2.6): one
/// module reused by both client and server operation codecs.
///
/// Every named `components/schemas` entry appears below in document declaration
/// order; nested anonymous objects and enumerations become generated definitions
/// emitted before their parents (`<Parent><FieldPascal>` plus numeric collision
/// suffixes, companion §10).
///
/// Property presence/nullability follows companion §2.1 cell-for-cell; bucket-2
/// validation constraints ride as documentation and as emitted `validate_request`
/// methods (companion §9; D-impl-runtime-validation-timing Phase 2 half). This file
/// is generated deterministically byte-for-byte (main spec §50 test 39); do not edit
/// by hand.
use openapi_support::optional::OptionalField;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub value: OptionalField<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `int64`.
    pub seq: i64,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub payload: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub meta: String,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventPayload {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `int32`.
    pub code: i32,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub message: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub value: OptionalField<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ack {
    pub accepted: bool,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `int32`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub received: OptionalField<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProblemDetails {
    pub title: String,
}
