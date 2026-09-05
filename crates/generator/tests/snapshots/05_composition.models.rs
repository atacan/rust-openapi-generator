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

/// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): pattern `^[a-z]+$`; minLength >= 3.
pub type Slug = String;

/// Server-side request validation (companion §9) for the constrained scalar alias `Slug`: bucket-2 constraints enforced on server requests; client encoding stays lenient.
pub fn validate_slug_request(
    value: &str,
) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
    ::openapi_support::validation::validate_string(
        value,
        &::openapi_support::validation::StringConstraints {
            pattern: Some("^[a-z]+$"),
            min_length: Some(3),
            max_length: None,
        },
    )?;
    Ok(())
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
pub struct Card {
    #[serde(rename = "cardNumber")]
    pub card_number: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cash {
    pub amount: i64,
}

/// Raw/value fallback carrying retained validation metadata (companion §4.2, DECISIONS.md D-impl-oneoffallback): mutual exclusivity of the anyOf branches could not be proven statically and choose-one enums are forbidden without proof (unproven-any-of); exactly-one semantics stay exact at the JSON level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaymentFallback(pub serde_json::Value);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeNode {
    pub value: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub left: OptionalField<Box<TreeNode>>,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub right: OptionalField<Box<TreeNode>>,
}
