//! OpenAPI to Rust code generator.
//!
//! Current scope: document loading — diagnostics, the version-agnostic IR,
//! and the `$ref` resolution engine ([`parse::load_document`]). Composition
//! merging, naming, and code generation are later work packages.

pub mod diagnostics;
pub mod ir;
pub mod parse;
