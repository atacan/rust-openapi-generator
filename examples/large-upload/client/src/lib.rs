//! Reqwest client for the large-upload example
//! (`--generate client --types-path large_upload_models`).
//!
//! The committed artifact under `generated/` is the byte-exact output of
//! `openapi-to-rust openapi.yaml --generate client --types-path
//! large_upload_models`; it is wired in UNMODIFIED as a file module (its
//! `//!` header is only legal in that position — never under `include!`) so
//! its `large_upload_models::models`/`::views` imports resolve against the
//! shared types crate. Compiling this crate never compiles the Axum server
//! stack.

#[path = "../generated/client.rs"]
pub mod client;

/// Demo-only memory instrumentation (sampled RSS + getrusage high-water
/// mark, progress printers), included ONCE from the example-root
/// `memmon/mod.rs` so both transports share it without a fourth crate or a
/// client↔server dependency.
#[path = "../../memmon/mod.rs"]
pub mod memmon;

/// Hand-written streaming-transfer driver ([`transfers::run_transfers`])
/// plus the generated-client constructor shared verbatim by the binary and
/// the ignored smoke tests.
pub mod transfers;
