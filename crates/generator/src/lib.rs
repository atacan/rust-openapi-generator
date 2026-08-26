//! OpenAPI to Rust code generator.
//!
//! Current scope: document loading ([`parse::load_document`]) into the
//! version-agnostic IR, the normalization layer ([`normalize`]) that resolves
//! composition keywords (companion §4), applies server precedence
//! (companion §8) and parameter merging (companion §6), assigns deterministic
//! Rust names (companion §10), renders reproducible debug dumps (main spec
//! §50), and emits the source artifacts — shared schema models
//! ([`codegen::models::generate_models`], main spec §2.6) plus their
//! directional views, the Reqwest client, and the Axum server interface —
//! with an explicit shared-types location
//! ([`codegen::config::TypesLocation`], D-impl-selective-artifacts) so
//! split workspace layouts can point client/server at an externally
//! generated types crate.

pub mod codegen;
pub mod diagnostics;
pub mod ir;
pub mod normalize;
pub mod parse;

pub use normalize::{normalize, normalize_with_config, NormalizeConfig};
