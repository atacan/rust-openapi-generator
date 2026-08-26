//! Reqwest client for the large-upload example
//! (`--generate client --types-path large_upload_models`).
//!
//! The committed artifact under `generated/` is the byte-exact output of
//! `openapi-to-rust openapi.yaml --generate client --types-path
//! large_upload_models`; it is include!d UNMODIFIED so its
//! `large_upload_models::models`/`::views` imports resolve against the
//! shared types crate. Compiling this crate never compiles the Axum server
//! stack.

pub mod client {
    include!("../generated/client.rs");
}

/// Hand-written streaming-transfer driver ([`transfers::run_transfers`])
/// plus the generated-client constructor shared verbatim by the binary and
/// the ignored smoke tests.
pub mod transfers;
