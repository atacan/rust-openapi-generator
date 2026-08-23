//! Client-emission harness: loads every committed fixture through
//! `load_document` → `normalize_with_config` → `codegen::plan::plan_api` →
//! `generate_client`, compares the rendered `client.rs` byte-for-byte against
//! snapshots under `tests/snapshots/`, asserts rustfmt-cleanliness (main spec
//! §50 test 40) and double-generation determinism (test 39), pins the §8/§9/
//! §10/§11/§23–§24 client shapes each fixture exists to cover, and greps every
//! snapshot for the forbidden patterns of §49.
//!
//! Snapshot regeneration: `CLIENT_SNAPSHOT_UPDATE=1 cargo test`.

use std::path::PathBuf;

use openapi_to_rust_generator::codegen::client::generate_client;
use openapi_to_rust_generator::codegen::plan::plan_api;
use openapi_to_rust_generator::normalize::{
    normalize_with_config, NormalizeConfig, NormalizedDocument,
};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

/// Fixtures with committed snapshots (byte-compare).
const SNAPSHOT_FIXTURES: &[&str] = &[
    "01_json_roundtrip.yaml",
    "02_streaming_binary.yaml",
    "03_nested_content.yaml",
    "04_status_ranges.yaml",
];

/// Every fixture must plan + render without diagnostics (07/08 included).
const ALL_FIXTURES: &[&str] = &[
    "01_json_roundtrip.yaml",
    "02_streaming_binary.yaml",
    "03_nested_content.yaml",
    "04_status_ranges.yaml",
    "05_composition.yaml",
    "06a_oas30.yaml",
    "06b_oas31.yaml",
    "07_matrix.yaml",
    "08_views.yaml",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn normalize_fixture(name: &str) -> NormalizedDocument {
    let ir = load_document(name, &fixtures_dir(), &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must load: {diags:?}"));
    normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must normalize: {diags:?}"))
}

fn generate_fixture(name: &str) -> String {
    let doc = normalize_fixture(name);
    let plan = plan_api(&doc).unwrap_or_else(|diags| panic!("{name} must plan: {diags:?}"));
    generate_client(&doc, &plan)
}

fn snapshot_name(fixture: &str) -> String {
    let stem = fixture.strip_suffix(".yaml").unwrap_or(fixture);
    format!("{stem}.client.rs")
}

// ----------------------------------------------------------------------
// Snapshots + double-generation determinism (main spec §50 tests 39)
// ----------------------------------------------------------------------

#[test]
fn client_snapshots_match_byte_for_byte_and_generation_is_deterministic() {
    std::fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
    for fixture in SNAPSHOT_FIXTURES {
        let generated = generate_fixture(fixture);

        // Double-generation check: an independent fresh load+plan+generate
        // must produce identical bytes.
        assert_eq!(
            generated,
            generate_fixture(fixture),
            "{fixture}: generation is not deterministic"
        );

        let snapshot = snapshots_dir().join(snapshot_name(fixture));
        if std::env::var("CLIENT_SNAPSHOT_UPDATE").as_deref() == Ok("1") {
            std::fs::write(&snapshot, &generated)
                .unwrap_or_else(|err| panic!("write snapshot {}: {err}", snapshot.display()));
            continue;
        }
        let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run with CLIENT_SNAPSHOT_UPDATE=1",
                snapshot.display()
            )
        });
        assert_eq!(
            generated, expected,
            "{fixture}: generated client diverged from snapshot"
        );
    }
}

#[test]
fn every_fixture_plans_and_renders_without_diagnostics() {
    for fixture in ALL_FIXTURES {
        let first = generate_fixture(fixture);
        let second = generate_fixture(fixture);
        assert_eq!(first, second, "{fixture}: generation is not deterministic");
        assert!(first.contains("pub struct Client"), "{fixture}");
    }
}

// ----------------------------------------------------------------------
// rustfmt-clean emission (main spec §50 test 40)
// ----------------------------------------------------------------------

#[test]
fn generated_clients_are_rustfmt_clean() {
    let Some(rustfmt) = locate_rustfmt() else {
        eprintln!(
            "WARNING: no rustfmt binary on PATH; skipping the rustfmt-clean assertion \
             (main spec §50 test 40)"
        );
        return;
    };
    for fixture in ALL_FIXTURES {
        let generated = generate_fixture(fixture);

        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "o2r-client-fmt-{}-{id}-{}",
            std::process::id(),
            fixture.trim_end_matches(".yaml")
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let source = dir.join(snapshot_name(fixture));
        std::fs::write(&source, &generated).expect("write generated client");

        let checked = std::process::Command::new(&rustfmt)
            .arg("--edition")
            .arg("2021")
            .arg("--check")
            .arg(&source)
            .output()
            .expect("spawn rustfmt");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            checked.status.success(),
            "{fixture}: generated output is not rustfmt-clean\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&checked.stdout),
            String::from_utf8_lossy(&checked.stderr),
        );
    }
}

/// Resolves a usable rustfmt: plain PATH lookup first, then the rustup
/// shim next to the running toolchain's cargo.
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

// ----------------------------------------------------------------------
// Forbidden patterns across EVERY snapshot (main spec §49)
// ----------------------------------------------------------------------

#[test]
fn no_forbidden_patterns_in_any_snapshot() {
    let mut checked = 0_usize;
    for entry in std::fs::read_dir(snapshots_dir()).expect("snapshots dir") {
        let path = entry.expect("snapshot entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let stem = path.file_name().unwrap().to_string_lossy().into_owned();
        if !stem.ends_with(".client.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("snapshot readable");
        for forbidden in [
            ".json(",
            ".form(",
            "serde_json::to_vec",
            "serde_json::to_string",
            "serde_urlencoded",
            ".bytes()",
            ".text()",
            "to_vec()",
            "axum::Json",
        ] {
            assert!(
                !text.contains(forbidden),
                "{stem}: forbidden pattern `{forbidden}` (main spec §49)"
            );
        }
        assert!(
            text.contains("collect_reqwest_limited"),
            "{stem}: bounded collection must be present"
        );
        checked += 1;
    }
    assert!(
        checked >= SNAPSHOT_FIXTURES.len(),
        "expected at least {} client snapshots, found {checked}",
        SNAPSHOT_FIXTURES.len()
    );
}

// ----------------------------------------------------------------------
// Fixture 01 — Example 1 shape (§8)
// ----------------------------------------------------------------------

#[test]
fn fixture_01_create_widget_takes_borrowed_body_and_decodes_enums() {
    let output = generate_fixture("01_json_roundtrip.yaml");

    assert!(
        output
            .contains("pub async fn create_widget(\n        &self,\n        body: &CreateWidget,"),
        "single-JSON-content ops take `&T` (D-§51.3):\n{output}"
    );
    assert!(
        output.contains("match serialize_json_limited(body, self.limits.structured_encode_bytes)"),
        "bounded serialization on the request path (§34.2):\n{output}"
    );
    assert!(
        output.contains("fn encode_overflow_error(limit: usize) -> ClientError {"),
        "§34.2 overflow helper must be emitted:\n{output}"
    );

    let response_enum = enum_block(&output, "CreateWidgetResponse");
    assert!(
        response_enum.contains("Created201(Widget),"),
        "\n{response_enum}"
    );
    assert!(
        response_enum.contains("BadRequest400(ProblemDetails),"),
        "\n{response_enum}"
    );

    // §29 Accept: operation-wide union over all statuses.
    assert!(
        output.contains("::http::header::ACCEPT")
            && output.contains("\"application/json, application/problem+json\""),
        "\n{output}"
    );
    // §30.1: redirect policy pinned off in the builder.
    assert!(output.contains("Policy::none()"), "\n{output}");
}

// ----------------------------------------------------------------------
// Fixture 02 — streaming upload/download (§9, §10, §32)
// ----------------------------------------------------------------------

#[test]
fn fixture_02_streaming_bodies_use_reqwest_native_types() {
    let output = generate_fixture("02_streaming_binary.yaml");

    assert!(
        output.contains("pub async fn put_object(\n        &self,\n        id: &str,\n        body: ::reqwest::Body,"),
        "binary uploads take `reqwest::Body` (§31):\n{output}"
    );

    let wrapper = struct_block(&output, "GetObject200");
    assert!(
        wrapper.contains("pub response: ::reqwest::Response,"),
        "\n{wrapper}"
    );
    assert!(
        output.contains("impl GetObject200 {") && output.contains("pub fn into_bytes_stream("),
        "wrapper carries the §32 convenience:\n{output}"
    );
    assert!(
        enum_block(&output, "GetObjectResponse").contains("Ok200(GetObject200),"),
        "\n{output}"
    );
    // Unit variant for the body-less 201 (§35 table row 1 shape).
    assert!(
        enum_block(&output, "PutObjectResponse").contains("Created201,"),
        "\n{output}"
    );
    // Server-variable builder method with declared default (companion §8).
    assert!(output.contains("pub fn region(mut self"), "\n{output}");
    assert!(output.contains("\"us-east\""), "\n{output}");
}

// ----------------------------------------------------------------------
// Fixture 02 — operation-level server override becomes its own base (§8)
// ----------------------------------------------------------------------

#[test]
fn fixture_02_operation_server_override_targets_its_own_base() {
    let output = generate_fixture("02_streaming_binary.yaml");

    // Companion §8: getObject's operation-level `servers` override wins over
    // the root-level array, so the two distinct effective defaults each get
    // their own stored base field.
    let client_struct = struct_block(&output, "Client");
    assert!(
        client_struct.contains("base_url: String,"),
        "primary base kept:\n{client_struct}"
    );
    assert!(
        client_struct.contains("base_url_storage: String,"),
        "distinct override base stored:\n{client_struct}"
    );

    // Each operation method resolves ITS base: put_object keeps the primary,
    // get_object targets the `/storage`-derived one.
    let put_method = method_block(&output, "put_object");
    assert!(
        put_method.contains("let mut url = self.base_url.clone();"),
        "\n{put_method}"
    );
    let get_method = method_block(&output, "get_object");
    assert!(
        get_method.contains("let mut url = self.base_url_storage.clone();"),
        "\n{get_method}"
    );

    // Secondary overrides are per-key (`base_url` replaces only the primary);
    // keys are deterministic snake_case derivations of the server URL and are
    // documented on the setter.
    assert!(
        output.contains("pub fn secondary_base_url(mut self, key: &str"),
        "\n{output}"
    );
    assert!(output.contains("- `storage`: `/storage`"), "\n{output}");

    // build() resolves the secondary independently of the primary.
    let build_block = item_block(&output, "pub fn build(self)");
    assert!(
        build_block.contains("self.secondary_base_urls.get(\"storage\")"),
        "\n{build_block}"
    );
    assert!(
        build_block.contains("base_url_storage: trimmed_storage.to_owned(),"),
        "\n{build_block}"
    );
}

// ----------------------------------------------------------------------
// Fixture 03 — nested content enum (§11)
// ----------------------------------------------------------------------

#[test]
fn fixture_03_nested_content_enum_selects_media_type() {
    let output = generate_fixture("03_nested_content.yaml");

    let content_enum = enum_block(&output, "GetArtifact200Content");
    assert!(
        content_enum.contains("Json(ArtifactMetadata),"),
        "\n{content_enum}"
    );
    assert!(
        content_enum.contains("OctetStream(::reqwest::Response),"),
        "\n{content_enum}"
    );
    // Negotiation ranks entries through match_entry (§28).
    assert!(
        output.contains("match_entry(&parsed, \"application/json\")"),
        "\n{output}"
    );
    assert!(output.contains("fn negotiation_rank("), "\n{output}");
}

// ----------------------------------------------------------------------
// Fixture 04 — ranges + default ordering (§23, §24)
// ----------------------------------------------------------------------

#[test]
fn fixture_04_ranges_default_and_explicit_precedence_ordering() {
    let output = generate_fixture("04_status_ranges.yaml");

    let response_enum = enum_block(&output, "GetWidgetResponse");
    assert!(
        response_enum.contains("Ok200(super::models::Widget),")
            || response_enum.contains("Ok200(Widget),"),
        "\n{response_enum}"
    );
    assert!(response_enum.contains("Success2xx {"), "\n{response_enum}");
    assert!(
        response_enum.contains("ClientError4xx {"),
        "\n{response_enum}"
    );
    assert!(response_enum.contains("Default {"), "\n{response_enum}");
    // Struct variants carry the wire status (§23).
    assert!(
        response_enum.contains("status: ::http::StatusCode,"),
        "\n{response_enum}"
    );

    // Literal 200 beats the range; Default matches LAST.
    let method = method_block(&output, "get_widget");
    let ok_arm = method
        .find("::http::StatusCode::OK =>")
        .expect("explicit 200 arm");
    let success_arm = method.find("(200..300)").expect("2XX guard arm");
    let error_arm = method.find("(400..500)").expect("4XX guard arm");
    let default_arm = method
        .rfind("Ok(GetWidgetResponse::Default")
        .expect("default arm");
    assert!(
        ok_arm < success_arm,
        "literal 200 must precede 2XX\n{method}"
    );
    assert!(
        success_arm < error_arm,
        "ranges keep declaration order\n{method}"
    );
    assert!(
        error_arm < default_arm,
        "`default` matches last (§24)\n{method}"
    );
    assert!(
        !method.contains("UndocumentedStatus"),
        "a documented `default` swallows every status\n{method}"
    );
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

/// The full text of one top-level item block, from its doc comments (or the
/// preceding blank line) through its closing line.
fn item_block(output: &str, marker: &str) -> String {
    let start = output
        .find(marker)
        .unwrap_or_else(|| panic!("marker `{marker}` not found in output:\n{output}"));
    let before = &output[..start];
    let block_start = before.rfind("\n\n").map_or(0, |index| index + 2);
    let after = &output[start..];
    let block_end = after.find("\n\n").map_or(after.len(), |index| index + 1);
    output[block_start..start + block_end].to_owned()
}

fn enum_block(output: &str, name: &str) -> String {
    item_block(output, &format!("pub enum {name} {{"))
}

fn struct_block(output: &str, name: &str) -> String {
    item_block(output, &format!("pub struct {name} {{"))
}

/// The whole `pub async fn <name>` method through its closing brace.
fn method_block(output: &str, name: &str) -> String {
    let marker = format!("pub async fn {name}(");
    let start = output
        .find(&marker)
        .unwrap_or_else(|| panic!("method `{name}` not found"));
    let rest = &output[start..];
    let end = rest
        .find("\n    }\n")
        .map_or(rest.len(), |index| index + "\n    }\n".len());
    rest[..end].to_owned()
}
