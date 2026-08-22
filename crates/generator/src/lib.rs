//! OpenAPI to Rust code generator.
//!
//! Current scope: document loading ([`parse::load_document`]) into the
//! version-agnostic IR plus the normalization layer ([`normalize`]) that
//! resolves composition keywords (companion §4), applies server precedence
//! (companion §8) and parameter merging (companion §6), assigns deterministic
//! Rust names (companion §10), and renders reproducible debug dumps
//! (main spec §50). Code generation itself is a later work package.

pub mod diagnostics;
pub mod ir;
pub mod normalize;
pub mod parse;

pub use normalize::{normalize, normalize_with_config, NormalizeConfig};
