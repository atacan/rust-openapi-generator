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
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProblemDetails {
    pub title: String,
}
