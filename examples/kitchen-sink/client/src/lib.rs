//! Reqwest client for the kitchen-sink example
//! (`--generate client --types-path kitchen_sink_models`).
//!
//! The committed artifact under `generated/` is the byte-exact output of
//! `openapi-to-rust openapi.yaml --generate client --types-path
//! kitchen_sink_models`; it is wired in UNMODIFIED as a file module (its
//! `//!` header is only legal in that position — never under `include!`) so
//! its `kitchen_sink_models::models`/`::views` imports resolve against the
//! shared types crate. Compiling this crate never compiles the Axum server
//! stack.

#[path = "../generated/client.rs"]
pub mod client;

/// Hand-written full-operation sweep driver ([`sweep::run_sweep`]) plus the
/// generated-client constructor shared verbatim by the binary and the
/// ignored smoke test.
pub mod sweep;
