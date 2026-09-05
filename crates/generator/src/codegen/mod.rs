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
pub mod codecs;
pub mod config;
pub mod manifest;
pub mod models;
pub mod plan;
pub mod server;
mod validation;
pub mod views;

pub use config::{CodegenConfig, TypesLocation};

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

    /// Renders inner (`//!`) module-doc lines for file headers; an empty
    /// entry becomes a bare `//!`. File headers must use inner docs so they
    /// document the enclosing module and stay valid in otherwise-empty files
    /// (outer `///` would attach to the first item and fail to compile when
    /// there is no item).
    pub(crate) fn inner_docs(&mut self, indent: usize, lines: &[String]) {
        for doc_line in lines {
            if doc_line.is_empty() {
                self.line(indent, "//!");
            } else {
                let rendered = format!("//! {doc_line}");
                self.line(indent, &rendered);
            }
        }
    }

    /// Appends a multi-line plugin fragment, indenting every line.
    pub(crate) fn block(&mut self, indent: usize, text: &str) {
        for fragment_line in text.split('\n') {
            self.line(indent, fragment_line);
        }
    }

    /// Consumes the emitter into the accumulated source text.
    pub(crate) fn finish(self) -> String {
        self.out
    }
}

/// Sort key mirroring rustfmt's `reorder_imports` (empirically verified
/// against the pinned toolchain): keyword-first paths rank `self` < `super`
/// < `crate`; then `::`-prefixed extern paths; then plain external paths —
/// each group ordered by its full normalized path. The emitters already emit
/// exactly this order for default-config documents (verified by the committed
/// snapshot suites), so the stable sort never moves their lines and only
/// slots codec use-lines and external shared-type imports into place (main
/// spec §45; D-impl-selective-artifacts).
pub(crate) fn import_sort_key(line: &str) -> (u8, u8, String) {
    let rest = line.strip_prefix("use ").unwrap_or(line);
    let rest = rest.trim_end_matches(';');
    let absolute = rest.starts_with("::");
    let rest = rest.strip_prefix("::").unwrap_or(rest);
    let first = rest.split("::").next().unwrap_or("");
    match first {
        "self" => (0, 0, rest.to_owned()),
        "super" => (0, 1, rest.to_owned()),
        "crate" => (0, 2, rest.to_owned()),
        _ if absolute => (1, 0, rest.to_owned()),
        _ => (2, 0, rest.to_owned()),
    }
}
