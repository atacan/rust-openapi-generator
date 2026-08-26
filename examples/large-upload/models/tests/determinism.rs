//! Determinism + snapshot gate for the large-upload example
//! (main spec §50 tests 38–39, end-to-end over the two-operation document).
//!
//! Runs the FULL public pipeline (`load_document` → `normalize_with_config`
//! → `plan_api_with_config` → the four generators) TWICE into independent
//! temp dirs under `std::env::temp_dir()`, byte-compares both runs against
//! each other AND against every committed artifact across the example's
//! THREE crates: `../models/generated/{models.rs,views.rs}`,
//! `../client/generated/client.rs`, `../server/generated/server.rs`. The
//! client/server passes use the same external shared-types path as the
//! committed files (`--types-path large_upload_models`). No tempfile crate:
//! unique directory names come from pid + a process-wide counter, mirroring
//! the generator's golden-harness convention. Normal generation writes no
//! Cargo.toml — manifests are hand-maintained per crate.
//!
//! Snapshot regeneration: `LARGE_UPLOAD_GENERATED_UPDATE=1 cargo test -p
//! large-upload-models` rewrites the committed files instead of comparing
//! (the same update-switch convention as OPENAPI_SNAPSHOT_UPDATE in
//! crates/generator/tests/golden_harness.rs).
//!
//! Diagnostic policy: ANY Error or Warning diagnostic fails loudly — none
//! are expected for this document, so even a Warning means stop-and-report.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use openapi_to_rust_generator::codegen::client::generate_client_with_config;
use openapi_to_rust_generator::codegen::config::{CodegenConfig, TypesLocation};
use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::{plan_api_with_config, PlanConfig};
use openapi_to_rust_generator::codegen::server::generate_server_with_config;
use openapi_to_rust_generator::codegen::views::generate_views;
use openapi_to_rust_generator::normalize::{normalize_with_config, NormalizeConfig};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

/// Committed artifacts: `(crate-relative directory, file name)` pairs, in
/// pipeline emission order, relative to THIS crate's manifest directory.
const ARTIFACTS: [(&str, &str); 4] = [
    ("generated", "models.rs"),
    ("generated", "views.rs"),
    ("../client/generated", "client.rs"),
    ("../server/generated", "server.rs"),
];

/// The shared-types base the transport crates were generated against.
const TYPES_PATH: &str = "large_upload_models";

fn document_path() -> PathBuf {
    // The document lives at the example root, one level above this crate.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../openapi.yaml")
}

/// Unique temp directory per call site invocation (no tempfile dependency).
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "large-upload-determinism-{}-{id}-{tag}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// One full generation pass: writes the four artifacts into `dir` and
/// returns their texts for byte comparison.
fn generate_into(dir: &Path) -> Vec<String> {
    let document = document_path();
    let root_dir = document.parent().expect("document has a parent directory");
    let root_yaml = document
        .file_name()
        .expect("document has a file name")
        .to_string_lossy()
        .into_owned();

    let ir = load_document(&root_yaml, root_dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("load failed: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("normalization failed: {diags:?}"));
    let plan = plan_api_with_config(&doc, &PlanConfig::default())
        .unwrap_or_else(|diags| panic!("planning failed: {diags:?}"));

    // ANY diagnostic — Warning or Error — fails here. None are expected for
    // this document; a Warning appearing means stop-and-report.
    assert!(
        doc.diagnostics.is_empty(),
        "unexpected generation diagnostics (none are expected for the \
         large-upload document): {:?}",
        doc.diagnostics
    );

    let transport_config = CodegenConfig {
        types_location: TypesLocation::external(TYPES_PATH)
            .unwrap_or_else(|reason| panic!("internal types path invalid: {reason}")),
    };

    let models = generate_models(&doc);
    let views = generate_views(&doc);
    let client = generate_client_with_config(&doc, &plan, &transport_config);
    let server = generate_server_with_config(&doc, &plan, &transport_config);

    let artifacts = vec![models, views, client, server];
    fs::create_dir_all(dir).expect("create artifact dir");
    for (index, text) in artifacts.iter().enumerate() {
        let name = ARTIFACTS[index].1;
        fs::write(dir.join(name), text)
            .unwrap_or_else(|err| panic!("{}: write {name}: {err}", dir.display()));
    }
    artifacts
}

#[test]
fn generated_artifacts_are_deterministic_and_match_commit() {
    if std::env::var("LARGE_UPLOAD_GENERATED_UPDATE").as_deref() == Ok("1") {
        let out = TempDir::new("update");
        let artifacts = generate_into(&out.path);
        for ((relative_dir, name), text) in ARTIFACTS.iter().zip(&artifacts) {
            let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_dir);
            fs::create_dir_all(&target_dir)
                .unwrap_or_else(|err| panic!("create {}: {err}", target_dir.display()));
            fs::write(target_dir.join(name), text)
                .unwrap_or_else(|err| panic!("update {name}: {err}"));
        }
        println!(
            "LARGE_UPLOAD_GENERATED_UPDATE=1: rewrote committed artifacts under {}",
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).display()
        );
        return;
    }

    // Two independent full-pipeline runs.
    let first_dir = TempDir::new("run1");
    let second_dir = TempDir::new("run2");
    let first = generate_into(&first_dir.path);
    let second = generate_into(&second_dir.path);

    for (index, (relative_dir, name)) in ARTIFACTS.iter().enumerate() {
        assert_eq!(
            first[index], second[index],
            "{name}: two generations diverge (main spec §50 test 39)"
        );

        let committed_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(relative_dir)
            .join(name);
        let committed = fs::read_to_string(&committed_path).unwrap_or_else(|err| {
            panic!(
                "read committed {name} at {}: {err}; run with \
                     LARGE_UPLOAD_GENERATED_UPDATE=1 cargo test -p \
                     large-upload-models",
                committed_path.display()
            )
        });
        assert_eq!(
            first[index],
            committed,
            "{name}: regenerated artifact differs from the committed file at \
             {}; run with LARGE_UPLOAD_GENERATED_UPDATE=1 to refresh",
            committed_path.display()
        );
    }
}
