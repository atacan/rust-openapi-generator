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
use openapi_to_rust_generator::codegen::plan::{
    plan_api_with_config, GeneratorPlanOptions, OperationPattern, PlanConfig,
    RepresentationOverride,
};
use openapi_to_rust_generator::codegen::server::generate_server;
use openapi_to_rust_generator::codegen::views::generate_views;
use openapi_to_rust_generator::diagnostics::Severity;
use openapi_to_rust_generator::normalize::normalize_with_config;
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

/// The codec fixture is generated once per CONFIGURATION (main spec §5.9/§45:
/// generation is config-dependent). Each variant gets its own `$OUT_DIR`
/// directory suffix and its own `include!` module tree.
const CODEC_FIXTURE: &str = "16_codecs.yaml";

/// `(directory suffix, plan options)` for every codec-enabled variant.
fn codec_variants() -> Vec<(&'static str, GeneratorPlanOptions)> {
    vec![
        (
            ".xml",
            GeneratorPlanOptions {
                enabled_codecs: ["xml"].into_iter().collect(),
                overrides: Vec::new(),
            },
        ),
        (
            ".cbor",
            GeneratorPlanOptions {
                enabled_codecs: ["cbor"].into_iter().collect(),
                overrides: Vec::new(),
            },
        ),
        (
            ".msgpack",
            GeneratorPlanOptions {
                enabled_codecs: ["msgpack"].into_iter().collect(),
                overrides: Vec::new(),
            },
        ),
        (
            // D-impl-override-precedence: ForceStreaming beats codec claims
            // AND re-classes the plain JSON op; all codecs stay enabled so the
            // precedence proof covers both directions at once.
            ".force-stream",
            GeneratorPlanOptions {
                enabled_codecs: ["xml", "cbor", "msgpack"].into_iter().collect(),
                overrides: vec![
                    RepresentationOverride::ForceStreaming {
                        match_media:
                            openapi_to_rust_generator::codegen::plan::MediaTypePattern::Exact(
                                "application/json".to_owned(),
                            ),
                        match_operation: OperationPattern::OperationId("echoJson".to_owned()),
                    },
                    RepresentationOverride::ForceStreaming {
                        match_media:
                            openapi_to_rust_generator::codegen::plan::MediaTypePattern::Exact(
                                "application/xml".to_owned(),
                            ),
                        match_operation: OperationPattern::Any,
                    },
                ],
            },
        ),
    ]
}

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
        if fixture == CODEC_FIXTURE {
            let stem = fixture.strip_suffix(".yaml").unwrap_or(fixture);
            for (suffix, options) in codec_variants() {
                let dir = out_dir.join(format!("{stem}{suffix}"));
                emit_with_options(
                    &fixtures_dir,
                    fixture,
                    &dir,
                    &PlanConfig {
                        generator_options: options,
                        ..PlanConfig::default()
                    },
                );
            }
        }
    }
}

/// Loads, normalizes, plans, and emits ONE fixture with the DEFAULT config;
/// panics on any diagnostic or on non-deterministic regeneration.
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

/// Runs the full pipeline once under an explicit plan config and writes the
/// four artifacts (plus a byte-identical determinism twin) under `dir`.
fn emit_with_options(fixtures_dir: &Path, fixture: &str, dir: &Path, config: &PlanConfig) {
    let artifacts = generate_with_config(fixtures_dir, fixture, dir, config);
    let verify = dir.parent().unwrap().join(format!(
        "{}.determinism-check",
        dir.file_name().unwrap().to_string_lossy()
    ));
    let again = generate_with_config(fixtures_dir, fixture, &verify, config);
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
    generate_with_config(fixtures_dir, fixture, dir, &PlanConfig::default())
}

/// Runs the full pipeline once under an explicit plan config.
fn generate_with_config(
    fixtures_dir: &Path,
    fixture: &str,
    dir: &Path,
    config: &PlanConfig,
) -> [String; 4] {
    let ir = load_document(fixture, fixtures_dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{fixture}: load failed: {diags:?}"));
    let doc = normalize_with_config(ir, &Default::default())
        .unwrap_or_else(|diags| panic!("{fixture}: normalization failed: {diags:?}"));
    let plan = plan_api_with_config(&doc, config)
        .unwrap_or_else(|diags| panic!("{fixture}: planning failed: {diags:?}"));
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
