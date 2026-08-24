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

/// Nullable: instances may be JSON `null`; reference sites wrap this type in `Option<T>`.
pub type NullableString31 = String;

/// Nullable: instances may be JSON `null`; reference sites wrap this type in `Option<T>`.
pub type NullableInt31 = i64;

/// Raw/value fallback: multiple non-null entries in a 3.1 `type` array; represented as `serde_json::Value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MixedTypes31Fallback(pub serde_json::Value);

pub type Coordinate = (String, i64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModernEnvelope {
    #[serde(
        deserialize_with = "openapi_support::optional::presence::deserialize_required_nullable"
    )]
    pub label: Option<NullableString31>,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub position: OptionalField<Coordinate>,
}
