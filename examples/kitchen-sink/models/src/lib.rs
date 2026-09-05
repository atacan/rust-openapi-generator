//! Shared OpenAPI schema surface for the kitchen-sink example
//! (`--generate types`, main spec §3): every schema type has exactly ONE
//! Rust identity, consumed by both transport crates.
//!
//! The two committed artifacts under `generated/` are the byte-exact output
//! of `openapi-to-rust openapi.yaml --generate types`; they are wired in
//! UNMODIFIED as file modules (their `//!` headers are only legal in that
//! position — never under `include!`) so `views`' `super::models`
//! references resolve inside this crate. `tests/determinism.rs` re-runs the
//! pipeline and proves all three example crates' committed artifacts stay
//! byte-stable (main spec §50 test 39).
//!
//! NOTE: `#[path]` resolves relative to THIS file's directory (`src/`), so
//! `../generated/…` reaches the crate-root `generated/` directory.

#[path = "../generated/models.rs"]
pub mod models;
#[path = "../generated/views.rs"]
pub mod views;
