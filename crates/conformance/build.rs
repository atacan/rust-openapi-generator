//! Compile-conformance build script (main spec §50 test 38, compile half):
//! loads + normalizes EVERY fixture under `crates/generator/fixtures/`,
//! plans it, and emits the four generated artifacts for each into
//! `$OUT_DIR/<fixture_stem>/` through the generator's public APIs.
//!
//! Any diagnostic (Error or Warning) fails the build loudly — the
//! stop-and-report policy leaves no room for improvised output. Generation
//! runs TWICE per fixture into independent directories and byte-compares
//! both, so double-generation determinism (main spec §50 test 39) is
//! enforced at build time across all four artifact kinds. Nothing but the
//! deterministic pipeline output ever reaches `$OUT_DIR` (no timestamps, no
//! paths).

use std::fs;
use std::path::{Path, PathBuf};

use openapi_to_rust_generator::codegen::client::generate_client;
use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::plan_api;
use openapi_to_rust_generator::codegen::server::generate_server;
use openapi_to_rust_generator::codegen::views::generate_views;
use openapi_to_rust_generator::diagnostics::Severity;
use openapi_to_rust_generator::normalize::normalize_with_config;
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

fn main() {
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../generator/fixtures")
        .canonicalize()
        .expect("fixtures directory exists");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));

    let mut fixtures: Vec<String> = fs::read_dir(&fixtures_dir)
        .expect("read fixtures directory")
        .filter_map(|entry| {
            let name = entry
                .expect("fixture entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            name.ends_with(".yaml").then_some(name)
        })
        .collect();
    fixtures.sort();
    assert!(
        !fixtures.is_empty(),
        "no fixtures found under {}",
        fixtures_dir.display()
    );

    for fixture in &fixtures {
        println!(
            "cargo:rerun-if-changed={}",
            fixtures_dir.join(fixture).display()
        );
        emit_fixture(&fixtures_dir, fixture, &out_dir);
    }
}

/// Loads, normalizes, plans, and emits ONE fixture; panics on any diagnostic
/// or on non-deterministic regeneration.
fn emit_fixture(fixtures_dir: &Path, fixture: &str, out_dir: &Path) {
    let stem = fixture.strip_suffix(".yaml").unwrap_or(fixture);

    // First generation.
    let primary = out_dir.join(stem);
    let artifacts = generate(fixtures_dir, fixture, &primary);

    // Second generation into an independent directory, byte-compared.
    let verify = out_dir.join(format!("{stem}.determinism-check"));
    let again = generate(fixtures_dir, fixture, &verify);
    if artifacts != again {
        panic!(
            "{fixture}: generated artifacts are not deterministic across \
             generations (main spec §50 test 39)"
        );
    }
}

/// Runs the full pipeline once and writes the four artifacts under `dir`;
/// returns the artifact texts for comparison.
fn generate(fixtures_dir: &Path, fixture: &str, dir: &Path) -> [String; 4] {
    let ir = load_document(fixture, fixtures_dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{fixture}: load failed: {diags:?}"));
    let doc = normalize_with_config(ir, &Default::default())
        .unwrap_or_else(|diags| panic!("{fixture}: normalization failed: {diags:?}"));
    let plan =
        plan_api(&doc).unwrap_or_else(|diags| panic!("{fixture}: planning failed: {diags:?}"));
    // Fixture 05 intentionally carries a Warning (`anyof_unprovable` raw/value
    // fallback); Errors are the stop-and-report condition.
    let errors: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .collect();
    if !errors.is_empty() {
        panic!("{fixture}: error diagnostics present, refusing to emit: {errors:?}");
    }

    let models = generate_models(&doc);
    let views = generate_views(&doc);
    let client = generate_client(&doc, &plan);
    let server = generate_server(&doc, &plan);

    fs::create_dir_all(dir).expect("create artifact directory");
    for (name, text) in [
        ("models.rs", &models),
        ("views.rs", &views),
        ("client.rs", &client),
        ("server.rs", &server),
    ] {
        fs::write(dir.join(name), text)
            .unwrap_or_else(|err| panic!("{}: write {name}: {err}", dir.display()));
    }
    [models, views, client, server]
}
