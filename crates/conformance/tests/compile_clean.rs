//! Artifact hygiene for the compile-conformance harness (§49; main spec §50
//! test 38's static half).
//!
//! Compilation of this crate already asserts the generated artifacts are
//! valid Rust; these tests additionally scan every emitted artifact in
//! `$OUT_DIR` and assert:
//!
//! 1. every artifact is non-empty, and
//! 2. none of the §49-forbidden serialization/decoding shortcuts appear:
//!    buffered `serde_json::to_vec/to_string`, form encoding through
//!    `serde_urlencoded`, whole-response buffering via `.bytes()`/`.text()`,
//!    or axum extractor shortcuts (`axum::Json(` / `axum::Form(`).

use std::fs;
use std::path::PathBuf;

const ARTIFACTS: &[&str] = &[
    "models.rs",
    "views.rs",
    "client.rs",
    "server.rs",
    "Cargo.toml",
];

/// Substrings that must never appear in generated code.
const FORBIDDEN: &[&str] = &[
    // Buffered JSON serialization (§49: bounded serializers only).
    "serde_json::to_vec(",
    "serde_json::to_string(",
    // Form encoding is a later phase; it must not leak in disguised.
    "serde_urlencoded",
    // Whole-body response buffering (§32/§34: bounded collect or streaming).
    ".bytes()",
    ".text()",
    // Axum extractor shortcuts bypass the documented pipeline (§38/§39).
    "axum::Json(",
    "axum::Form(",
];

fn out_dir() -> PathBuf {
    PathBuf::from(env!("OUT_DIR"))
}

/// Every fixture artifact directory (the `.determinism-check` twins are
/// excluded; they are byte-identical by construction and checked at build
/// time).
fn fixture_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(out_dir())
        .expect("OUT_DIR readable")
        .filter_map(|entry| {
            let path = entry.expect("OUT_DIR entry").path();
            if path.is_dir()
                && !path
                    .file_name()
                    .map(|name| name.to_string_lossy().ends_with(".determinism-check"))
                    .unwrap_or(false)
            {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no fixture artifact directories emitted");
    dirs
}

#[test]
fn every_artifact_is_present_and_non_empty() {
    for dir in fixture_dirs() {
        for artifact in ARTIFACTS {
            let path = dir.join(artifact);
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{}: unreadable: {err}", path.display()));
            assert!(
                !content.trim().is_empty(),
                "{} must be non-empty",
                path.display()
            );
        }
    }
}

#[test]
fn no_forbidden_patterns_in_any_artifact() {
    for dir in fixture_dirs() {
        for artifact in ARTIFACTS {
            let path = dir.join(artifact);
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{}: unreadable: {err}", path.display()));
            for pattern in FORBIDDEN {
                assert!(
                    !content.contains(pattern),
                    "{} contains forbidden pattern `{pattern}` (§49)",
                    path.display()
                );
            }
        }
    }
}
