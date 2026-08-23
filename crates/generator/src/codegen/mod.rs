//! Code generation layer (main spec §2.6/§3 generated module layout;
//! DECISIONS.md D-impl-codegen-emission).
//!
//! Emission is deterministic string templating verified against `rustfmt`:
//! no syn/quote dependency, stable ordering everywhere, and never timestamps
//! or file-system paths in output (main spec §50 tests 39–40).
//!
//! Phase 1 scope: shared schema models ([`models::generate_models`]) plus the
//! directional read/write view types enforcing companion §5 directionality at
//! the type level ([`views::generate_views`]). The client and server
//! operation packages land later in Phase 1; their modules are declared now
//! so the tree is stable for parallel work packages.

pub mod client;
pub mod models;
pub mod plan;
pub mod server;
pub mod views;

/// Indentation-aware line writer shared by the emitters: fixed four-space
/// levels, append-only, fully determined by the call sequence.
pub(crate) struct Emitter {
    out: String,
}

impl Emitter {
    pub(crate) fn new() -> Self {
        Self { out: String::new() }
    }

    /// Appends one source line indented by `indent` four-space levels.
    pub(crate) fn line(&mut self, indent: usize, text: &str) {
        for _ in 0..indent {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// Appends a bare newline (blank separator line).
    pub(crate) fn blank(&mut self) {
        self.out.push('\n');
    }

    /// Renders doc-comment lines; an empty entry becomes a bare `///`.
    pub(crate) fn docs(&mut self, indent: usize, lines: &[String]) {
        for doc_line in lines {
            if doc_line.is_empty() {
                self.line(indent, "///");
            } else {
                let rendered = format!("/// {doc_line}");
                self.line(indent, &rendered);
            }
        }
    }

    /// Consumes the emitter into the accumulated source text.
    pub(crate) fn finish(self) -> String {
        self.out
    }
}
