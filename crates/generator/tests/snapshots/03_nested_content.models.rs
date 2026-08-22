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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub id: String,
    pub name: String,
    /// Constraints (runtime enforcement starts in Phase 2, DECISIONS.md D-impl-runtime-validation-timing): format `int64`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub size: OptionalField<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProblemDetails {
    pub title: String,
}
