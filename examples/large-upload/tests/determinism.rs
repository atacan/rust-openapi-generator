//! Determinism + snapshot gate for the large-upload example
//! (main spec §50 tests 38–39, end-to-end over the example document).
//!
//! Runs the FULL public pipeline (`load_document` → `normalize_with_config`
//! → `plan_api_with_config` → the four generators plus
//! [`generate_manifest`]) TWICE into independent temp dirs under
//! `std::env::temp_dir()`, byte-compares both runs against each other AND
//! against every committed `generated/` artifact (models.rs, views.rs,
//! client.rs, server.rs, and the emitted manifest Cargo.toml). No tempfile
//! crate: unique directory names come from pid + a process-wide counter,
//! mirroring the generator's golden-harness convention.
//!
//! Snapshot regeneration: `LARGE_UPLOAD_GENERATED_UPDATE=1 cargo test -p
//! large-upload` rewrites the committed files instead of comparing (the same
//! update-switch convention as KITCHEN_SINK_GENERATED_UPDATE).
//!
//! Diagnostic policy: ANY Error or Warning diagnostic fails loudly — none
//! are expected for this document, so even a Warning means stop-and-report.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use openapi_to_rust_generator::codegen::client::generate_client;
use openapi_to_rust_generator::codegen::manifest::{
    generate_manifest, FeatureSelection, ManifestConfig,
};
use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::{plan_api_with_config, Decompression, PlanConfig};
use openapi_to_rust_generator::codegen::server::generate_server;
use openapi_to_rust_generator::codegen::views::generate_views;
use openapi_to_rust_generator::normalize::{normalize_with_config, NormalizeConfig};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

/// Committed artifact names, in pipeline emission order.
const ARTIFACTS: [&str; 5] = [
    "models.rs",
    "views.rs",
    "client.rs",
    "server.rs",
    "Cargo.toml",
];

fn document_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("openapi.yaml")
}

fn generated_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("generated")
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

/// One full generation pass: writes the five artifacts into `dir` and returns
/// their texts for byte comparison.
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

    let models = generate_models(&doc);
    let views = generate_views(&doc);
    let client = generate_client(&doc, &plan);
    let server = generate_server(&doc, &plan);
    let manifest = generate_manifest(
        &doc,
        &plan,
        &ManifestConfig {
            enabled_codecs: plan.enabled_codecs.clone(),
            features: FeatureSelection {
                client: true,
                server: true,
                decompression: Decompression::OFF,
            },
            ..ManifestConfig::default()
        },
    )
    .unwrap_or_else(|diags| panic!("manifest generation failed: {diags:?}"));

    let artifacts = vec![models, views, client, server, manifest];
    fs::create_dir_all(dir).expect("create artifact dir");
    for (name, text) in ARTIFACTS.iter().zip(&artifacts) {
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
        fs::create_dir_all(generated_dir())
            .unwrap_or_else(|err| panic!("create {}: {err}", generated_dir().display()));
        for (name, text) in ARTIFACTS.iter().zip(&artifacts) {
            fs::write(generated_dir().join(name), text)
                .unwrap_or_else(|err| panic!("update {name}: {err}"));
        }
        println!(
            "LARGE_UPLOAD_GENERATED_UPDATE=1: rewrote committed artifacts in {}",
            generated_dir().display()
        );
        return;
    }

    // Two independent full-pipeline runs.
    let first_dir = TempDir::new("run1");
    let second_dir = TempDir::new("run2");
    let first = generate_into(&first_dir.path);
    let second = generate_into(&second_dir.path);

    for (index, name) in ARTIFACTS.iter().enumerate() {
        assert_eq!(
            first[index], second[index],
            "{name}: two generations diverge (main spec §50 test 39)"
        );

        let committed = fs::read_to_string(generated_dir().join(name)).unwrap_or_else(|err| {
            panic!(
                "read committed {name}: {err}; run with \
                     LARGE_UPLOAD_GENERATED_UPDATE=1 cargo test -p large-upload"
            )
        });
        assert_eq!(
            first[index],
            committed,
            "{name}: regenerated artifact differs from the committed file in \
             {}; run with LARGE_UPLOAD_GENERATED_UPDATE=1 to refresh",
            generated_dir().display()
        );
    }
}
