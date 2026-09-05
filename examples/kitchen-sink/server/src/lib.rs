//! Axum server for the kitchen-sink example
//! (`--generate server --types-path kitchen_sink_models`).
//!
//! The committed artifact under `generated/` is the byte-exact output of
//! `openapi-to-rust openapi.yaml --generate server --types-path
//! kitchen_sink_models`; it is wired in UNMODIFIED as a file module (its
//! `//!` header is only legal in that position — never under `include!`) so
//! its `kitchen_sink_models::models`/`::views` imports resolve against the
//! shared types crate. Compiling this crate never compiles the Reqwest
//! client stack.

#[path = "../generated/server.rs"]
pub mod server;

/// Hand-written demo application ([`app::KitchenSinkApp`] implementing every
/// documented operation over in-memory data plus temp-dir binary storage)
/// and the router wiring shared verbatim by the binary and the smoke test.
pub mod app;
