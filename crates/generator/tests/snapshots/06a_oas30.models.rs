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
pub type NullableString = String;

pub type PositiveInt = i64;

/// Server-side request validation (companion §9) for the constrained scalar alias `PositiveInt`: bucket-2 constraints enforced on server requests; client encoding stays lenient.
pub fn validate_positive_int_request(
    value: &i64,
) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
    ::openapi_support::validation::validate_number(*value as f64, Some((5.0, true)), None, None)?;
    Ok(())
}

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
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `binary`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub payload: OptionalField<String>,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): exclusiveMinimum > 1.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub count: OptionalField<i64>,
}

impl LegacyEnvelope {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        if let OptionalField::Present(value) = &self.count {
            ::openapi_support::validation::validate_number(
                *value as f64,
                Some((1.0, true)),
                None,
                None,
            )
            .map_err(|error| error.at_field("count"))?;
        }
        Ok(())
    }
}
