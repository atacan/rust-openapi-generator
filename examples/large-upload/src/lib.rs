//! Large-upload example crate (same generated-crate shape as the
//! kitchen-sink example, main spec §3).
//!
//! The four committed artifacts under `generated/` are the byte-exact output
//! of the generator pipeline run over `openapi.yaml` (two streaming binary
//! PUTs: `application/octet-stream` and `audio/wav`); they are include!d
//! UNMODIFIED into sibling modules so the emitters' `super::models::…`
//! references resolve inside this crate's own module tree.
//! `tests/determinism.rs` re-runs the full pipeline and proves the committed
//! files stay byte-stable (main spec §50 test 39).
//!
//! NOTE: `include!` paths resolve relative to THIS file's directory (`src/`),
//! so `../generated/…` reaches the crate-root `generated/` directory.

pub mod api {
    pub mod models {
        include!("../generated/models.rs");
    }
    pub mod views {
        include!("../generated/views.rs");
    }
    pub mod client {
        include!("../generated/client.rs");
    }
    pub mod server {
        include!("../generated/server.rs");
    }
}

/// Process memory monitoring shared by both binaries and the smoke tests:
/// baseline + sampled-peak RSS (`memory-stats`, 50 ms sampler) plus the
/// kernel high-water mark (getrusage `ru_maxrss`), rendered into a report
/// with a pass/fail verdict against the demo's bounded-memory threshold.
pub mod memmon;

/// Hand-written demo application ([`demo::LargeUploadApp`] implementing the
/// generated [`crate::api::server::Api`] trait in two modes — chunk-wise
/// disk persistence or whole-body proxy forwarding — plus the streaming
/// upload driver shared verbatim by the client binary and the smoke tests).
pub mod demo;
