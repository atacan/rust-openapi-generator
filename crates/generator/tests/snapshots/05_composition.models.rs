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
pub struct BaseWidget {
    pub id: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub name: OptionalField<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timestamps {
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub name: OptionalField<String>,
    /// Constraints (runtime enforcement starts in Phase 2, DECISIONS.md D-impl-runtime-validation-timing): format `date-time`.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullWidget {
    pub id: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub name: OptionalField<String>,
    /// Constraints (runtime enforcement starts in Phase 2, DECISIONS.md D-impl-runtime-validation-timing): format `date-time`.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

/// Constraints (runtime enforcement starts in Phase 2, DECISIONS.md D-impl-runtime-validation-timing): pattern `^[a-z]+$`; minLength >= 3.
pub type Slug = String;

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
