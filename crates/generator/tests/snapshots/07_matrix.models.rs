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
use std::collections::BTreeMap;

use openapi_support::optional::OptionalField;
use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub code: OptionalField<IntCode>,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub anything: OptionalField<MixedScalar>,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub strict: OptionalField<StrictRecord>,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub tags: OptionalField<TaggedBag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StringStatus {
    #[serde(rename = "draft")]
    Draft,
    #[serde(rename = "in_review")]
    InReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntCode {
    V1 = 1,
    V2 = 2,
    V4 = 4,
}

impl IntCode {
    /// Wire discriminants accepted by this enumeration.
    pub const VALUES: &'static [i64] = &[1, 2, 4];
}

impl serde::Serialize for IntCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Bare JSON numbers; derived unit variants would emit strings.
        serializer.serialize_i64(*self as i64)
    }
}

impl<'de> serde::Deserialize<'de> for IntCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <i64 as serde::Deserialize>::deserialize(deserializer)?;
        match raw {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            4 => Ok(Self::V4),
            other => Err(serde::de::Error::custom(format!(
                "unknown discriminant {other} for enum `IntCode`, expected one of [1, 2, 4]"
            ))),
        }
    }
}

/// Mixed-type enumeration (companion §4.3): typed variants for scalar constants plus a trailing `Other` catch-all matched last under `#[serde(untagged)]`; null and non-scalar constants fold into `Other`, whose identity Phase 2 validators enforce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MixedScalar {
    Text(String),
    V7(i64),
    True(bool),
    Other(serde_json::Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrictRecord {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaggedBag {
    pub label: String,
    #[serde(flatten)]
    pub additional: BTreeMap<String, String>,
}
