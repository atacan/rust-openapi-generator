//! Axum server for the large-upload example
//! (`--generate server --types-path large_upload_models`).
//!
//! The committed artifact under `generated/` is the byte-exact output of
//! `openapi-to-rust openapi.yaml --generate server --types-path
//! large_upload_models`; it is include!d UNMODIFIED so its
//! `large_upload_models::models`/`::views` imports resolve against the
//! shared types crate.

pub mod server {
    include!("../generated/server.rs");
}

/// Demo-only memory instrumentation (sampled RSS + getrusage high-water
/// mark, progress printers), included ONCE from the example-root
/// `memmon/mod.rs` so both transports share it without a fourth crate or a
/// client↔server dependency.
#[path = "../../memmon/mod.rs"]
pub mod memmon;

/// Hand-written demo application ([`app::LargeUploadApp`] implementing the
/// generated [`crate::server::Api`] trait in two modes — chunk-wise disk
/// persistence or whole-body proxy forwarding) plus the router wiring shared
/// verbatim by the binary and the client crate's ignored smoke tests.
pub mod app;
