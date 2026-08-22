/// Shared schema models generated from the OpenAPI document (main spec §2.6): one
/// module reused by both client and server operation codecs.
///
/// Every named `components/schemas` entry appears below in document declaration
/// order; nested anonymous objects and enumerations become generated definitions
/// emitted before their parents (`<Parent><FieldPascal>` plus numeric collision
/// suffixes, companion §10).
///
/// Property presence/nullability follows companion §2.1 cell-for-cell; bucket-2
/// validation constraints ride as documentation until Phase 2 runtime enforcement
/// (DECISIONS.md D-impl-runtime-validation-timing). This file is generated
/// deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use openapi_support::optional::OptionalField;
use serde::{Deserialize, Serialize};

/// Nullable: instances may be JSON `null`; reference sites wrap this type in `Option<T>`.
pub type NullableString = String;

pub type PositiveInt = i64;

/// Warning: `format: binary` marks a raw payload; Binary media classes stream bytes and never reach shared models in Phase 1 (main spec §5.3), so this is modeled as `String`.
pub type LegacyBytes = String;

/// Warning: `format: binary` marks a raw payload; Binary media classes stream bytes and never reach shared models in Phase 1 (main spec §5.3), so this is modeled as `String`.
pub type LegacyFileType = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyEnvelope {
    pub name: String,
    #[serde(default)]
    pub note: Option<String>,
    /// Warning: `format: binary` marks a raw payload; Binary media classes stream bytes and never reach shared models in Phase 1 (main spec §5.3), so this is modeled as `String`.
    /// Constraints (runtime enforcement starts in Phase 2, DECISIONS.md D-impl-runtime-validation-timing): format `binary`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub payload: OptionalField<String>,
    /// Constraints (runtime enforcement starts in Phase 2, DECISIONS.md D-impl-runtime-validation-timing): exclusiveMinimum > 1.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub count: OptionalField<i64>,
}
