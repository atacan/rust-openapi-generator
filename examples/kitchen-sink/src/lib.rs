//! Kitchen-sink example crate (main spec §3 generated-crate shape).
//!
//! The four committed artifacts under `generated/` are the byte-exact output
//! of the generator pipeline run over `openapi.yaml` (union superset of the
//! fixture corpus); they are include!d UNMODIFIED into sibling modules so the
//! emitters' `super::models::…` references resolve inside this crate's own
//! module tree. `tests/determinism.rs` re-runs the full pipeline and proves
//! the committed files stay byte-stable (main spec §50 test 39).
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

/// Hand-written demo application (one [`demo::KitchenSinkApp`] implementing
/// every documented operation over in-memory data plus temp-dir binary
/// storage) and the full-operation sweep driver shared verbatim by the two
/// binaries and the ignored smoke test.
pub mod demo;
