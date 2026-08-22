//! OpenAPI to Rust code generator.
//!
//! Current scope: document loading ([`parse::load_document`]) into the
//! version-agnostic IR, the normalization layer ([`normalize`]) that resolves
//! composition keywords (companion §4), applies server precedence
//! (companion §8) and parameter merging (companion §6), assigns deterministic
//! Rust names (companion §10), renders reproducible debug dumps (main spec
//! §50), and emits shared schema models ([`codegen::models::generate_models`],
//! main spec §2.6). Client/server operation codegen is a later work package.

pub mod codegen;
pub mod diagnostics;
pub mod ir;
pub mod normalize;
pub mod parse;

pub use normalize::{normalize, normalize_with_config, NormalizeConfig};
