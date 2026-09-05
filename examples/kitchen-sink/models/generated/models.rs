//! Shared schema models generated from the OpenAPI document (main spec §2.6): one
//! module reused by both client and server operation codecs.
//!
//! Every named `components/schemas` entry appears below in document declaration
//! order; nested anonymous objects and enumerations become generated definitions
//! emitted before their parents (`<Parent><FieldPascal>` plus numeric collision
//! suffixes, companion §10).
//!
//! Property presence/nullability follows companion §2.1 cell-for-cell; bucket-2
//! validation constraints ride as documentation and as emitted `validate_request`
//! methods (companion §9; D-impl-runtime-validation-timing Phase 2 half). This file
//! is generated deterministically byte-for-byte (main spec §50 test 39); do not edit
//! by hand.
use openapi_support::optional::OptionalField;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProblemDetails {
    pub title: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub detail: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateWidget {
    pub name: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub description: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub description: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuccessEnvelope {
    pub data: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateSessionForm {
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub remember_me: OptionalField<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub token: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub title: String,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `int32`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub pages: OptionalField<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
}

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
pub struct ThumbnailMetadata {
    pub id: String,
    pub name: String,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `int64`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub size: OptionalField<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VendorDocument {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): minLength >= 2.
    pub title: String,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `int32`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub revision: OptionalField<i32>,
}

impl VendorDocument {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_string(
            &self.title,
            &::openapi_support::validation::StringConstraints {
                pattern: None,
                min_length: Some(2),
                max_length: None,
            },
        )
        .map_err(|error| error.at_field("title"))?;
        Ok(())
    }
}

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
pub struct BaseWidget {
    pub id: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub name: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timestamps {
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub name: OptionalField<String>,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `date-time`.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl Timestamps {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_format_string(&self.created_at, "date-time")
            .map_err(|error| error.at_field("created_at"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullWidget {
    pub id: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub name: OptionalField<String>,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `date-time`.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl FullWidget {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_format_string(&self.created_at, "date-time")
            .map_err(|error| error.at_field("created_at"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DogKind {
    #[serde(rename = "dog")]
    Dog,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dog {
    pub kind: DogKind,
    pub bark: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CatKind {
    #[serde(rename = "cat")]
    Cat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cat {
    pub kind: CatKind,
    pub meow: i64,
}

/// Proven mutually exclusive branches (companion §4.2): exclusivity was proven statically, so derive-based untagged decoding preserves exactly-one validation.
/// Discriminator (routing hint only; inspect-select-validate decode arrives in a later package): property `kind`.
/// Mapping: dog -> Dog, cat -> Cat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Pet {
    Dog(Dog),
    Cat(Cat),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixRecord {
    pub req_plain: String,
    #[serde(
        deserialize_with = "openapi_support::optional::presence::deserialize_required_nullable"
    )]
    pub req_nullable: Option<String>,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `int32`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub opt_plain: OptionalField<i32>,
    #[serde(default)]
    pub opt_nullable: Option<i64>,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub status: OptionalField<StringStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StringStatus {
    #[serde(rename = "draft")]
    Draft,
    #[serde(rename = "in_review")]
    InReview,
}
