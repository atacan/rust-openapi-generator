//! Shared OpenAPI schema surface for the kitchen-sink example
//! (`--generate types`, main spec §3): every schema type has exactly ONE
//! Rust identity, consumed by both transport crates.
//!
//! The two committed artifacts under `generated/` are the byte-exact output
//! of `openapi-to-rust openapi.yaml --generate types`; they are include!d
//! UNMODIFIED into sibling modules so `views`' `super::models` references
//! resolve inside this crate. `tests/determinism.rs` re-runs the pipeline
//! and proves all three example crates' committed artifacts stay
//! byte-stable (main spec §50 test 39).
//!
//! NOTE: `include!` paths resolve relative to THIS file's directory
//! (`src/`), so `../generated/…` reaches the crate-root `generated/`
//! directory.

pub mod models {
    include!("../generated/models.rs");
}
pub mod views {
    include!("../generated/views.rs");
}
