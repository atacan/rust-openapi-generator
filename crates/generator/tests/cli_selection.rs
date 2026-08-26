//! CLI selection harness (DECISIONS.md D-impl-selective-artifacts): drives
//! the real `openapi-to-rust` binary through `CARGO_BIN_EXE` and pins
//!
//! - the artifact selection grammar (`--generate` repeated and
//!   comma-separated forms are equivalent; `all`; deterministic dedup),
//! - the default all-in-one behavior (byte-equal to the library pipeline),
//! - argument-order independence of emitted bytes,
//! - the validation rules (`client`/`server` without `types` require
//!   `--types-path`; `--types-path` together with `types` is ambiguous;
//!   unknown artifacts and invalid Rust paths exit 2), and
//! - the preserved `--dump` mode.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use openapi_to_rust_generator::codegen::client::generate_client_with_config;
use openapi_to_rust_generator::codegen::config::CodegenConfig;
use openapi_to_rust_generator::codegen::manifest::{
    generate_manifest, FeatureSelection, ManifestConfig,
};
use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::plan_api;
use openapi_to_rust_generator::codegen::server::generate_server_with_config;
use openapi_to_rust_generator::codegen::views::generate_views;
use openapi_to_rust_generator::normalize::{normalize_with_config, NormalizeConfig};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

const BIN: &str = env!("CARGO_BIN_EXE_openapi-to-rust");
const FIXTURE_01: &str = "01_json_roundtrip.yaml";
const FIXTURE_08: &str = "08_views.yaml";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn document(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

/// Unique scratch directory per call site invocation (no tempfile crate,
/// mirroring the repo's pid+counter convention).
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "o2r-cli-selection-{}-{id}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch dir");
        Self { path }
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path.join(name))
            .unwrap_or_else(|err| panic!("read {}: {err}", self.path.join(name).display()))
    }

    fn exists(&self, name: &str) -> bool {
        self.path.join(name).exists()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs_remove(&self.path);
    }
}

fn fs_remove(path: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}

fn run_cli(args: &[&str]) -> Output {
    std::process::Command::new(BIN)
        .args(args)
        .output()
        .expect("spawn openapi-to-rust")
}

fn assert_exit_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} must succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn assert_exit_code(output: &Output, code: u8, context: &str) -> String {
    assert_eq!(
        output.status.code(),
        Some(i32::from(code)),
        "{context} must exit {code}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Library-side reference pipeline for byte comparisons.
fn library_artifacts(fixture: &str) -> Vec<(String, String)> {
    let ir = load_document(fixture, &fixtures_dir(), &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{fixture} must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{fixture} must normalize: {diags:?}"));
    let plan = plan_api(&doc).unwrap_or_else(|diags| panic!("{fixture} must plan: {diags:?}"));
    vec![
        ("models.rs".to_owned(), generate_models(&doc)),
        ("views.rs".to_owned(), generate_views(&doc)),
        (
            "client.rs".to_owned(),
            generate_client_with_config(&doc, &plan, &CodegenConfig::default()),
        ),
        (
            "server.rs".to_owned(),
            generate_server_with_config(&doc, &plan, &CodegenConfig::default()),
        ),
        (
            // The manifest is NOT part of the CLI's source-artifact
            // selection; generated here only to keep the reference honest.
            "__manifest_unused".to_owned(),
            generate_manifest(
                &doc,
                &plan,
                &ManifestConfig {
                    features: FeatureSelection::BOTH,
                    ..ManifestConfig::default()
                },
            )
            .expect("reference manifest"),
        ),
    ]
    .into_iter()
    .filter(|(name, _)| name != "__manifest_unused")
    .collect()
}

// ----------------------------------------------------------------------
// Default mode + grammar
// ----------------------------------------------------------------------

#[test]
fn default_selection_generates_all_four_source_artifacts() {
    let scratch = Scratch::new("default");
    let out = scratch.path.to_string_lossy().into_owned();
    let output = run_cli(&[document(FIXTURE_01).to_str().unwrap(), "--out-dir", &out]);
    assert_exit_success(&output, "default generation");

    for (name, text) in library_artifacts(FIXTURE_01) {
        assert_eq!(
            scratch.read(&name),
            text,
            "{name}: default CLI generation diverges from the library pipeline"
        );
    }
}

#[test]
fn comma_and_repeated_forms_are_equivalent() {
    let comma = Scratch::new("comma");
    let repeated = Scratch::new("repeated");

    assert_exit_success(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "types,client",
            "--out-dir",
            comma.path.to_str().unwrap(),
        ]),
        "comma form",
    );
    assert_exit_success(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "types",
            "--generate",
            "client",
            "--out-dir",
            repeated.path.to_str().unwrap(),
        ]),
        "repeated form",
    );

    for name in ["models.rs", "views.rs", "client.rs"] {
        assert!(!comma.exists("server.rs"));
        assert_eq!(
            comma.read(name),
            repeated.read(name),
            "{name}: comma and repeated --generate forms diverge"
        );
    }
    assert!(comma.exists("server.rs") == repeated.exists("server.rs"));
}

#[test]
fn types_selection_writes_models_and_views_only() {
    let scratch = Scratch::new("types-only");
    assert_exit_success(
        &run_cli(&[
            document(FIXTURE_08).to_str().unwrap(),
            "--generate",
            "types",
            "--out-dir",
            scratch.path.to_str().unwrap(),
        ]),
        "types-only generation",
    );
    assert!(scratch.exists("models.rs"));
    assert!(scratch.exists("views.rs"));
    assert!(!scratch.exists("client.rs"));
    assert!(!scratch.exists("server.rs"));

    // views.rs belongs to the types surface and imports its models sibling.
    let views = scratch.read("views.rs");
    assert!(views.contains("use super::models::"), "\n{views}");
}

#[test]
fn all_is_shorthand_for_every_artifact() {
    let explicit = Scratch::new("all-explicit");
    let shorthand = Scratch::new("all-shorthand");
    let doc_flag = document(FIXTURE_01).to_str().unwrap().to_owned();

    assert_exit_success(
        &run_cli(&[
            &doc_flag,
            "--generate",
            "types,client,server",
            "--out-dir",
            explicit.path.to_str().unwrap(),
        ]),
        "explicit full selection",
    );
    assert_exit_success(
        &run_cli(&[
            &doc_flag,
            "--generate",
            "all",
            "--out-dir",
            shorthand.path.to_str().unwrap(),
        ]),
        "all shorthand",
    );
    for name in ["models.rs", "views.rs", "client.rs", "server.rs"] {
        assert_eq!(explicit.read(name), shorthand.read(name), "{name}");
    }
}

#[test]
fn argument_order_never_affects_emitted_bytes() {
    let canonical = Scratch::new("order-canonical");
    let shuffled = Scratch::new("order-shuffled");
    let doc_flag = document(FIXTURE_08).to_str().unwrap().to_owned();

    assert_exit_success(
        &run_cli(&[&doc_flag, "--out-dir", canonical.path.to_str().unwrap()]),
        "canonical order",
    );
    assert_exit_success(
        &run_cli(&[
            &doc_flag,
            "--generate",
            "server,client,types",
            "--generate",
            "types",
            "--out-dir",
            shuffled.path.to_str().unwrap(),
        ]),
        "shuffled order with duplicate",
    );
    for name in ["models.rs", "views.rs", "client.rs", "server.rs"] {
        assert_eq!(
            canonical.read(name),
            shuffled.read(name),
            "{name}: argument order changed emitted bytes"
        );
    }
}

// ----------------------------------------------------------------------
// External shared-types path
// ----------------------------------------------------------------------

#[test]
fn external_types_path_rewrites_import_base() {
    let scratch = Scratch::new("external-client");
    assert_exit_success(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "client",
            "--types-path",
            "api_types",
            "--out-dir",
            scratch.path.to_str().unwrap(),
        ]),
        "external client",
    );
    let client = scratch.read("client.rs");
    assert!(client.contains("use api_types::models::"), "\n{client}");
    assert!(!client.contains("super::models"), "\n{client}");
    assert!(!scratch.exists("models.rs"));
    assert!(!scratch.exists("views.rs"));
}

#[test]
fn dedup_selection_client_client_server_uses_one_external_path() {
    let scratch = Scratch::new("dedup");
    assert_exit_success(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "client,client,server",
            "--types-path",
            "crate::shared_api",
            "--out-dir",
            scratch.path.to_str().unwrap(),
        ]),
        "deduplicated selection",
    );
    assert!(scratch.exists("client.rs"));
    assert!(scratch.exists("server.rs"));
    assert!(!scratch.exists("models.rs"));
    assert!(!scratch.exists("views.rs"));
    let server = scratch.read("server.rs");
    assert!(
        server.contains("use crate::shared_api::models::"),
        "\n{server}"
    );
}

// ----------------------------------------------------------------------
// Validation rules
// ----------------------------------------------------------------------

#[test]
fn client_without_types_requires_types_path() {
    let stderr = assert_exit_code(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "client",
        ]),
        2,
        "client without types",
    );
    assert!(
        stderr.contains("generating `client` without `types` requires --types-path"),
        "\n{stderr}"
    );
    assert!(stderr.contains("--types-path api_types"), "\n{stderr}");
}

#[test]
fn server_without_types_requires_types_path() {
    let stderr = assert_exit_code(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "server",
        ]),
        2,
        "server without types",
    );
    assert!(
        stderr.contains("generating `server` without `types` requires --types-path"),
        "\n{stderr}"
    );
}

#[test]
fn client_and_server_without_types_require_types_path() {
    let stderr = assert_exit_code(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "client,server",
        ]),
        2,
        "client,server without types",
    );
    assert!(
        stderr.contains("generating `client`, `server` without `types` requires --types-path"),
        "\n{stderr}"
    );
}

#[test]
fn types_path_is_rejected_when_types_generated_together() {
    let stderr = assert_exit_code(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "types,client",
            "--types-path",
            "api_types",
        ]),
        2,
        "ambiguous invocation",
    );
    assert!(
        stderr.contains(
            "--types-path cannot be used when `types` is generated in the \
             same invocation"
        ),
        "\n{stderr}"
    );
}

#[test]
fn unknown_artifact_value_is_rejected() {
    let stderr = assert_exit_code(
        &run_cli(&[
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "widgets",
        ]),
        2,
        "unknown artifact",
    );
    assert!(stderr.contains("unknown value `widgets`"), "\n{stderr}");
    assert!(stderr.contains("types, client, server"), "\n{stderr}");
}

#[test]
fn invalid_types_path_values_are_rejected() {
    for bad in ["api-types", "foo/bar", "foo::"] {
        let scratch = Scratch::new("bad-path");
        let stderr = assert_exit_code(
            &run_cli(&[
                document(FIXTURE_01).to_str().unwrap(),
                "--generate",
                "client",
                "--types-path",
                bad,
                "--out-dir",
                scratch.path.to_str().unwrap(),
            ]),
            2,
            bad,
        );
        assert!(
            stderr.contains(&format!("invalid --types-path value `{bad}`")),
            "`{bad}`:\n{stderr}"
        );
        assert!(!scratch.exists("client.rs"));
    }
}

// ----------------------------------------------------------------------
// Preserved --dump mode
// ----------------------------------------------------------------------

#[test]
fn dump_mode_still_prints_the_normalized_dump() {
    let output = run_cli(&["--dump", document(FIXTURE_01).to_str().unwrap()]);
    assert_exit_success(&output, "--dump");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("openapi 3.1"), "\n{stdout}");
}

#[test]
fn dump_mode_rejects_generation_arguments() {
    let stderr = assert_exit_code(
        &run_cli(&[
            "--dump",
            document(FIXTURE_01).to_str().unwrap(),
            "--generate",
            "types",
        ]),
        2,
        "--dump plus --generate",
    );
    assert!(stderr.contains("cannot be combined"), "\n{stderr}");
}

#[test]
fn help_exits_successfully() {
    for flag in ["-h", "--help"] {
        let output = run_cli(&[flag]);
        assert_exit_success(&output, flag);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("--generate"), "\n{stdout}");
        assert!(stdout.contains("--types-path"), "\n{stdout}");
    }
}
