//! Shared-types location harness (DECISIONS.md D-impl-selective-artifacts):
//! proves that the configurable emission entry points
//! ([`generate_client_with_config`]/[`generate_server_with_config`])
//!
//! 1. preserve byte-identical sibling output through their backward-
//!    compatible wrappers (`generate_client`/`generate_server`),
//! 2. render external shared-type import prefixes exactly as configured
//!    (`<base>::models` / `<base>::views`) with NO residual `super::`
//!    references,
//! 3. stay deterministic across repeated generation, and
//! 4. remain `rustfmt`-clean in external mode (main spec §50 test 40),
//!    which pins the import ordering rule for plain external paths and
//!    keyword paths (`crate::…`) beside the `::`-prefixed support crates.

use std::path::PathBuf;
use std::process::Command;

use openapi_to_rust_generator::codegen::client::{generate_client, generate_client_with_config};
use openapi_to_rust_generator::codegen::config::{CodegenConfig, TypesLocation};
use openapi_to_rust_generator::codegen::plan::{plan_api, PlannedApi};
use openapi_to_rust_generator::codegen::server::{generate_server, generate_server_with_config};
use openapi_to_rust_generator::normalize::{
    normalize_with_config, NormalizeConfig, NormalizedDocument,
};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn normalize_fixture(name: &str) -> NormalizedDocument {
    let ir = load_document(name, &fixtures_dir(), &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must load: {diags:?}"));
    normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must normalize: {diags:?}"))
}

fn plan_fixture(doc: &NormalizedDocument) -> PlannedApi {
    plan_api(doc).unwrap_or_else(|diags| panic!("fixture must plan: {diags:?}"))
}

#[test]
fn sibling_wrappers_match_default_config_byte_for_byte() {
    for name in ["01_json_roundtrip.yaml", "08_views.yaml"] {
        let doc = normalize_fixture(name);
        let plan = plan_fixture(&doc);

        assert_eq!(
            generate_client(&doc, &plan),
            generate_client_with_config(&doc, &plan, &CodegenConfig::default()),
            "{name}: client wrapper diverges from the default configuration"
        );
        assert_eq!(
            generate_server(&doc, &plan),
            generate_server_with_config(&doc, &plan, &CodegenConfig::default()),
            "{name}: server wrapper diverges from the default configuration"
        );
    }
}

#[test]
fn sibling_output_keeps_super_imports() {
    // Fixture 01 exercises the shared-models bucket on the client side;
    // fixture 08 exercises BOTH buckets (models + directional views) on the
    // server side.
    let doc = normalize_fixture("01_json_roundtrip.yaml");
    let plan = plan_fixture(&doc);
    let client = generate_client(&doc, &plan);
    assert!(client.contains("use super::models::"), "\n{client}");

    let doc = normalize_fixture("08_views.yaml");
    let plan = plan_fixture(&doc);
    let server = generate_server(&doc, &plan);
    assert!(server.contains("use super::models::"), "\n{server}");
    assert!(server.contains("use super::views::"), "\n{server}");
}

#[test]
fn external_location_renders_base_module_imports() {
    // Same bucket coverage as [`sibling_output_keeps_super_imports`]:
    // client/models via fixture 01, server/models+views via fixture 08.
    let doc = normalize_fixture("01_json_roundtrip.yaml");
    let plan = plan_fixture(&doc);
    let config = CodegenConfig {
        types_location: TypesLocation::external("api_types").unwrap(),
    };
    let client = generate_client_with_config(&doc, &plan, &config);
    assert!(
        client.contains("use api_types::models::"),
        "client must import models through the external base:\n{client}"
    );
    assert!(!client.contains("super::models"), "\n{client}");
    assert!(!client.contains("super::views"), "\n{client}");

    let doc = normalize_fixture("08_views.yaml");
    let plan = plan_fixture(&doc);
    let server = generate_server_with_config(&doc, &plan, &config);
    assert!(server.contains("use api_types::models::"), "\n{server}");
    assert!(server.contains("use api_types::views::"), "\n{server}");
    assert!(!server.contains("super::models"), "\n{server}");
    assert!(!server.contains("super::views"), "\n{server}");
}

#[test]
fn keyword_and_nested_external_paths_render_verbatim() {
    let doc = normalize_fixture("01_json_roundtrip.yaml");
    let plan = plan_fixture(&doc);
    let config = CodegenConfig {
        types_location: TypesLocation::external("crate::generated::types").unwrap(),
    };
    let client = generate_client_with_config(&doc, &plan, &config);
    assert!(
        client.contains("use crate::generated::types::models::"),
        "\n{client}"
    );

    let doc = normalize_fixture("08_views.yaml");
    let plan = plan_fixture(&doc);
    let config = CodegenConfig {
        types_location: TypesLocation::external("company_api::v2").unwrap(),
    };
    let server = generate_server_with_config(&doc, &plan, &config);
    assert!(
        server.contains("use company_api::v2::views::"),
        "\n{server}"
    );
}

#[test]
fn external_generation_is_deterministic() {
    let doc = normalize_fixture("08_views.yaml");
    let plan = plan_fixture(&doc);
    let config = CodegenConfig {
        types_location: TypesLocation::external("api_types").unwrap(),
    };

    let first_client = generate_client_with_config(&doc, &plan, &config);
    let second_client = generate_client_with_config(&doc, &plan, &config);
    assert_eq!(first_client, second_client);

    let first_server = generate_server_with_config(&doc, &plan, &config);
    let second_server = generate_server_with_config(&doc, &plan, &config);
    assert_eq!(first_server, second_server);
}

/// Resolves a usable rustfmt: plain PATH lookup first, then the rustup
/// shim next to the running toolchain's cargo (same convention as the
/// snapshot suites; returns `None` so toolchain-less environments skip
/// rather than fail).
fn locate_rustfmt() -> Option<PathBuf> {
    if which_exists("rustfmt") {
        return Some(PathBuf::from("rustfmt"));
    }
    let cargo = PathBuf::from(std::env::var("CARGO").ok()?);
    let sibling = cargo.with_file_name("rustfmt");
    sibling.is_file().then_some(sibling)
}

fn which_exists(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

#[test]
fn external_output_is_rustfmt_clean() {
    let Some(rustfmt) = locate_rustfmt() else {
        eprintln!(
            "WARNING: no rustfmt binary on PATH; skipping the external-mode \
             rustfmt-clean assertion (main spec §50 test 40)"
        );
        return;
    };

    let doc = normalize_fixture("08_views.yaml");
    let plan = plan_fixture(&doc);
    let config = CodegenConfig {
        types_location: TypesLocation::external("api_types").unwrap(),
    };

    for (name, text) in [
        (
            "client.rs",
            generate_client_with_config(&doc, &plan, &config),
        ),
        (
            "server.rs",
            generate_server_with_config(&doc, &plan, &config),
        ),
    ] {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "o2r-types-location-fmt-{}-{id}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let source = dir.join(name);
        std::fs::write(&source, &text).expect("write generated module");

        let checked = Command::new(&rustfmt)
            .arg("--edition")
            .arg("2021")
            .arg("--check")
            .arg(&source)
            .output()
            .expect("spawn rustfmt");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            checked.status.success(),
            "{name}: external-mode output is not rustfmt-clean \
             (main spec §50 test 40)\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&checked.stdout),
            String::from_utf8_lossy(&checked.stderr),
        );
    }
}
