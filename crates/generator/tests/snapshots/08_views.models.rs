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
pub struct Account {
    pub id: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub note: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "draftNote")]
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub draft_note: OptionalField<String>,
    #[serde(default)]
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncedRecord {
    pub id: String,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): minLength >= 3.
    pub label: String,
    #[serde(rename = "secretToken")]
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub secret_token: OptionalField<String>,
    #[serde(rename = "reviewedBy")]
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub reviewed_by: OptionalField<String>,
}

impl SyncedRecord {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_string(
            &self.label,
            &::openapi_support::validation::StringConstraints {
                pattern: None,
                min_length: Some(3),
                max_length: None,
            },
        )
        .map_err(|error| error.at_field("label"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlainNote {
    pub title: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub body: OptionalField<String>,
}
