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
use openapi_to_rust_generator::codegen::models::generate_models;
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
    "08_views.yaml",
    "10_forms_headers.yaml",
    "11_multipart.yaml",
    "12_multipart_order.yaml",
    "13_validation.yaml",
    "14_negotiation.yaml",
    "15_streams.yaml",
    "17_head.yaml",
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
    "10_forms_headers.yaml",
    "11_multipart.yaml",
    "12_multipart_order.yaml",
    "13_validation.yaml",
    "14_negotiation.yaml",
    "15_streams.yaml",
    "17_head.yaml",
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

#[test]
fn generated_client_starts_with_inner_module_docs() {
    let output = generate_fixture("18_no_schemas.yaml");
    assert!(output.starts_with("//! Reqwest client generated"));
    assert!(!output.starts_with("///"));
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

    // Literal 200 beats the range; Default matches LAST. Since the §31
    // factoring, the status arms live in the shared `decode_get_widget`
    // tail (called by the base method and any `_replaying` twin).
    let method = method_block(&output, "decode_get_widget");
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
// Fixture 14 — wildcard/negotiation completion (§22, §25, §5.2, §29, §44)
// ----------------------------------------------------------------------

#[test]
fn fixture_14_trio_and_exact_entries_rank_as_bounded_or_streaming_variants() {
    let output = generate_fixture("14_negotiation.yaml");

    // Example 18 trio (§25): all three FINITE variants on one status.
    let trio = enum_block(&output, "GetReport400Content");
    assert!(trio.contains("ProblemJson(ProblemDetails),"), "{trio}");
    assert!(trio.contains("Json(LegacyError),"), "{trio}");
    assert!(
        trio.contains("TextPlain(String),"),
        "text/plain + string stays a bounded String (§5.2):\n{trio}"
    );

    // §22: explicit JSON beats nothing here, but the octet-stream entry must
    // stay response-OWNED streaming (never a bounded buffer).
    let content_enum = enum_block(&output, "GetReport200Content");
    assert!(content_enum.contains("Json(Report),"), "\n{content_enum}");
    assert!(
        content_enum.contains("OctetStream(::reqwest::Response),"),
        "\n{content_enum}"
    );
}

#[test]
fn fixture_14_text_range_response_is_a_streaming_wrapper_not_a_string() {
    let output = generate_fixture("14_negotiation.yaml");

    // §5.2 range mode: `text/*` streams through a response-owned wrapper.
    let wrapper = struct_block(&output, "GetRawText200");
    assert!(
        wrapper.contains("pub response: ::reqwest::Response,"),
        "text/* must stream via the raw response:\n{wrapper}"
    );
    assert!(
        !wrapper.contains("String"),
        "text/* must never materialize as a bounded String:\n{wrapper}"
    );
    assert!(
        enum_block(&output, "GetRawTextResponse").contains("Ok200(GetRawText200),"),
        "\n{output}"
    );
}

#[test]
fn fixture_14_accept_includes_range_tokens_verbatim() {
    let output = generate_fixture("14_negotiation.yaml");

    // §29: the operation-wide Accept union carries the media RANGE token
    // verbatim (no concrete expansion).
    let method = method_block(&output, "get_raw_text");
    assert!(
        method.contains(".header(::http::header::ACCEPT, \"text/*\")"),
        "Accept must carry `text/*` verbatim:\n{method}"
    );

    // The trio operation unions every decodable literal across statuses in
    // declaration order (200's entries first, then 400's new ones).
    let report = method_block(&output, "get_report");
    assert!(
        report.contains(
            "\"application/json, application/octet-stream, \
             application/problem+json, text/plain\""
        ),
        "§29 union in declaration order across all statuses:\n{report}"
    );
}

#[test]
fn fixture_14_stream_override_switches_request_representation_both_ways() {
    let output = generate_fixture("14_negotiation.yaml");

    // §44 default: bounded text/plain requests take &str.
    assert!(
        output.contains("pub async fn put_note(&self, id: &str, body: &str)"),
        "plain text/plain stays bounded String/&str:\n{output}"
    );

    // §44 override: x-rust-body: stream takes reqwest::Body instead.
    assert!(
        output.contains(
            "pub async fn post_stream_note(\n        &self,\n        body: ::reqwest::Body,"
        ),
        "x-rust-body: stream must force the raw streaming parameter:\n{output}"
    );
    let method = method_block(&output, "post_stream_note");
    assert!(
        method.contains(".header(::http::header::CONTENT_TYPE, \"text/plain\")")
            && !method.contains("structured_encode_bytes"),
        "streaming override skips bounded encoding but keeps its literal \
         Content-Type:\n{method}"
    );

    // The request enum for {json, */*} keeps the wildcard variant streaming.
    let body_enum = enum_block(&output, "PostMirrorRequestBody");
    assert!(body_enum.contains("Json(Payload),"), "{body_enum}");
    assert!(
        body_enum.contains("Any(::reqwest::Body),"),
        "the */* entry attaches reqwest::Body verbatim:\n{body_enum}"
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

/// The whole `pub async fn <name>` (or private `async fn <name>`) method
/// through its closing brace.
fn method_block(output: &str, name: &str) -> String {
    let marker = format!("async fn {name}(");
    let start = output
        .find(&marker)
        .unwrap_or_else(|| panic!("method `{name}` not found"));
    let rest = &output[start..];
    let end = rest
        .find("\n    }\n")
        .map_or(rest.len(), |index| index + "\n    }\n".len());
    rest[..end].to_owned()
}

// ----------------------------------------------------------------------
// Typed response headers (§15) — synthetic document, no committed fixture
// ----------------------------------------------------------------------

const HEADERS_FIXTURE: &str = r#"openapi: 3.1.0
info:
  title: typed headers
  version: "1"
paths:
  /multi:
    get:
      operationId: getMulti
      responses:
        '200':
          description: Either representation plus typed headers
          headers:
            X-Request-Id:
              required: true
              schema:
                type: string
            Retry-After-Seconds:
              schema:
                type: integer
                format: int32
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Doc'
            text/plain:
              schema:
                type: string
        '302':
          description: Redirect with a header and no body
          headers:
            Location:
              required: true
              schema:
                type: string
components:
  schemas:
    Doc:
      type: object
      required: [id]
      properties:
        id:
          type: string
"#;

/// Pins the §15 client shapes that no committed fixture covers: multi-content
/// hoisting onto the status VARIANT, header-only struct variants, and
/// collision-suffixed header naming. The rendered module is additionally
/// checked against rustfmt so the new emission paths stay canonical.
#[test]
fn headers_hoist_onto_multi_content_variants_and_header_only_statuses() {
    let dir = std::env::temp_dir().join(format!("o2r-client-headers-{}-a", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture_path = dir.join("typed_headers.yaml");
    std::fs::write(&fixture_path, HEADERS_FIXTURE).expect("write synthetic fixture");

    let ir = load_document("typed_headers.yaml", &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must normalize: {diags:?}"));
    let plan =
        plan_api(&doc).unwrap_or_else(|diags| panic!("headers fixture must plan: {diags:?}"));
    let output = generate_client(&doc, &plan);

    let _ = std::fs::remove_dir_all(&dir);

    // Hoisted multi-content variant: typed fields beside the content enum;
    // snake_case naming keeps declaration order (companion §10/D-§6).
    let response_enum = enum_block(&output, "GetMultiResponse");
    assert!(response_enum.contains("Ok200 {"), "{response_enum}");
    assert!(
        response_enum.contains("x_request_id: String,"),
        "{response_enum}"
    );
    assert!(
        response_enum.contains("retry_after_seconds: Option<i32>,"),
        "{response_enum}"
    );
    assert!(
        response_enum.contains("content: GetMulti200Content,"),
        "{response_enum}"
    );

    // Header-only 302: exactly the typed headers, never a body field.
    assert!(output.contains("Found302 {"), "\n{output}");
    assert!(
        response_enum.contains("location: String,"),
        "{response_enum}"
    );

    // The 302 decode arm reads the header BEFORE constructing the variant
    // and never touches a body. The §31 factoring moved the status arms
    // into the shared `decode_get_multi` tail.
    let method = method_block(&output, "decode_get_multi");
    assert!(
        method.contains("parse_required_header::<String>(&response, \"location\")?"),
        "{method}"
    );
    assert!(
        method.contains("Ok(GetMultiResponse::Found302 {"),
        "{method}"
    );
}

/// The synthetic §15 module must also be rustfmt-clean so its emission paths
/// stay canonical (main spec §50 test 40).
#[test]
fn headers_fixture_client_is_rustfmt_clean() {
    let Some(rustfmt) = locate_rustfmt() else {
        eprintln!("WARNING: no rustfmt on PATH; skipping");
        return;
    };
    let workdir =
        std::env::temp_dir().join(format!("o2r-client-headers-fmt-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("create fixture dir");
    let fixture_path = workdir.join("typed_headers.yaml");
    std::fs::write(&fixture_path, HEADERS_FIXTURE).expect("write synthetic fixture");

    let ir = load_document("typed_headers.yaml", &workdir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must normalize: {diags:?}"));
    let plan =
        plan_api(&doc).unwrap_or_else(|diags| panic!("headers fixture must plan: {diags:?}"));
    let output = generate_client(&doc, &plan);

    let source = workdir.join("typed_headers.client.rs");
    std::fs::write(&source, &output).expect("write generated client");

    let checked = std::process::Command::new(&rustfmt)
        .arg("--edition")
        .arg("2021")
        .arg("--check")
        .arg(&source)
        .output()
        .expect("spawn rustfmt");
    let _ = std::fs::remove_dir_all(&workdir);
    assert!(
        checked.status.success(),
        "synthetic headers client is not rustfmt-clean\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr),
    );
}

// ----------------------------------------------------------------------
// Fixture 11 — multipart input builder (§17 Output A)
// ----------------------------------------------------------------------

#[test]
fn fixture_11_multipart_request_struct_streams_files_and_has_from_file() {
    let output = generate_fixture("11_multipart.yaml");

    // §17 Output A struct: owned scalar/JSON parts, streaming binary part,
    // optional filename/content-type beside it, and NO Vec<u8> anywhere.
    let request = struct_block(&output, "UploadDocumentRequest");
    assert!(
        request.contains("pub metadata: DocumentMetadata,"),
        "{request}"
    );
    assert!(
        request.contains("pub tags: Vec<String>,"),
        "repeated textual parts collect in wire order:\n{request}"
    );
    assert!(
        request.contains("pub file: ::reqwest::Body,"),
        "binary parts stay streaming:\n{request}"
    );
    assert!(request.contains("pub file_name: Option<String>,"));
    assert!(request.contains("pub file_content_type: Option<::mime::Mime>,"));
    assert!(!output.contains("Vec<u8>"), "\n{output}");

    // §17 from_file constructor streams the opened file through
    // tokio-util's ReaderStream into reqwest::Body::wrap_stream.
    let ctor = item_block(&output, "pub async fn from_file(");
    assert!(ctor.contains("::tokio::fs::File::open(path.as_ref()).await?"),);
    assert!(ctor.contains("::tokio_util::io::ReaderStream::new(file)"),);
    assert!(ctor.contains("::reqwest::Body::wrap_stream(stream)"),);

    // The method takes the OWNED input struct (§17; no `&T` convenience).
    assert!(
        output.contains(&format!("body: {}Request,", "UploadDocument"))
            || output.contains("body: UploadDocumentRequest,"),
        "\n{output}"
    );

    // JSON parts serialize bounded BEFORE any wire traffic (§34.2) and are
    // attached with their declared encoding content type.
    let method = item_block(&output, "pub async fn upload_document(");
    assert!(
        method.contains(
            "match serialize_json_limited(&body.metadata, self.limits.structured_encode_bytes)"
        ),
        "\n{method}"
    );
    assert!(
        method.contains(
            "part_with_mime(Part::bytes(Vec::from(&payload[..])), \"application/json\")?"
        ),
        "{method}"
    );
    assert!(method.contains(".multipart(form);"), "{method}");
    // The boundary-bearing Content-Type is written by reqwest itself.
    assert!(
        !method.contains("CONTENT_TYPE"),
        "static Content-Type headers would drop the multipart boundary:\n{method}"
    );
}

// ----------------------------------------------------------------------
// Fixture 15 — streaming record formats (§5.6–§5.8, §18–§20 Output A)
// ----------------------------------------------------------------------

#[test]
fn fixture_15_client_wrappers_expose_incremental_stream_decoders() {
    let output = generate_fixture("15_streams.yaml");

    // Each record-framed entry owns a `<Op><Status>Stream` wrapper carrying
    // the raw response plus the stored limits, and exposes the per-framing
    // incremental decoder.
    for (wrapper, method, item, error) in [
        (
            "ExportRecords200Stream",
            "into_ndjson_stream",
            "Record",
            "NdjsonDecodeError",
        ),
        (
            "StreamEvents200Stream",
            "into_sse_stream",
            "Event",
            "SseDecodeError",
        ),
        (
            "StreamEnvelopeEvents200Stream",
            "into_sse_stream",
            "EventPayload",
            "SseDecodeError",
        ),
    ] {
        let block = struct_block(&output, wrapper);
        assert!(
            block.contains("pub response: ::reqwest::Response,"),
            "{block}"
        );
        assert!(block.contains("pub limits: BodyLimits,"), "{block}");
        let method_sig = format!("pub fn {method}(");
        assert!(output.contains(&method_sig), "{method} missing:\n{output}");
        assert!(
            output.contains(&format!(
                "impl ::futures_core::Stream<Item = Result<{item}, {error}>>"
            )),
            "{wrapper} decode signature"
        );
        assert!(
            output.contains("self.limits.max_stream_record_bytes"),
            "per-record bound comes from the stored limits"
        );
    }

    // §18.1 override wins: the envelope schema never appears as the item
    // type of the SSE stream.
    let envelope_wrapper = struct_block(&output, "StreamEnvelopeEvents200Stream");
    assert!(
        !envelope_wrapper.contains("EventEnvelope"),
        "x-rust-stream-item must replace the envelope schema:\n{envelope_wrapper}"
    );

    // Request direction: boxed erased item-stream alias (documented shape).
    assert!(
        output.contains(
            "pub type PushMetricsJsonSeqBody =\n             \u{0020}   ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = Metric> \
             + ::std::marker::Send>>;"
        ) || output.contains("pub type PushMetricsJsonSeqBody ="),
        "boxed alias shape documented on the type"
    );
    let push = method_block(&output, "push_metrics");
    assert!(
        push.contains("stream_request_encoder(") && push.contains(".await?"),
        "eager head encode precedes the send:\n{push}"
    );
    assert!(
        push.contains("::reqwest::Body::wrap_stream(encoder)"),
        "items stream lazily mid-send:\n{push}"
    );

    // Accept union keeps the streaming literals verbatim (§29).
    assert!(
        method_block(&output, "export_records")
            .contains("\"application/x-ndjson, application/problem+json\""),
        "\n{}",
        method_block(&output, "export_records")
    );
}

/// Extended §49 greps: the streaming snapshots must never aggregate a body
/// and must never collect through the bounded collectors either.
#[test]
fn no_aggregation_in_streaming_snapshots() {
    let text = std::fs::read_to_string(snapshots_dir().join("15_streams.client.rs"))
        .expect("snapshot readable");
    for forbidden in [".bytes()", ".text()", "to_vec()", "serde_json::to_vec"] {
        assert!(
            !text.contains(forbidden),
            "streaming snapshot aggregates via `{forbidden}` (§49)"
        );
    }
}

// ----------------------------------------------------------------------
// Fixture 08 — directional view consumption (companion §5, §50 test 50)
// ----------------------------------------------------------------------

#[test]
fn fixture_08_client_request_enums_and_methods_take_write_views() {
    let output = generate_fixture("08_views.yaml");

    // Convenience methods borrow the WRITE views (readOnly fields cannot be
    // sent by construction).
    assert!(
        output
            .contains("pub async fn create_account(\n        &self,\n        body: &AccountWrite,"),
        "\n{output}"
    );
    assert!(output.contains("body: &SyncedRecordWrite"), "\n{output}");

    // Response enums decode into READ views.
    assert!(
        enum_block(&output, "CreateAccountResponse").contains("Created201(AccountRead),"),
        "\n{output}"
    );
    assert!(
        enum_block(&output, "ListAuditEntriesResponse").contains("Ok200(AuditEntryRead),"),
        "\n{output}"
    );
    assert!(
        enum_block(&output, "SyncRecordResponse").contains("Ok200(SyncedRecordRead),"),
        "\n{output}"
    );

    // The shared models are fully replaced on the wire paths: no bare
    // Account/AuditEntry/SyncedRecord payload remains anywhere.
    for shared in ["Account)", "AuditEntry)", "SyncedRecord)"] {
        assert!(
            !output.contains(shared),
            "shared-model payload `{shared}` must not remain:\n{output}"
        );
    }

    // View types import from `super::views`, and the module documents the
    // directional policy.
    assert!(output.contains("use super::views::"), "\n{output}");
    assert!(
        output.contains("Directional views (companion §5"),
        "\n{output}"
    );
}

// ----------------------------------------------------------------------
// Fixture 17 — HEAD header-only variants (§15, §35)
// ----------------------------------------------------------------------

#[test]
fn fixture_17_head_variants_carry_typed_headers_without_body_accessor() {
    let output = generate_fixture("17_head.yaml");

    // The HEAD response variant carries EXACTLY the typed headers: no body
    // field exists to reach, and the GET counterpart keeps its normal
    // decoded-payload shape beside it.
    let head_enum = enum_block(&output, "HeadWidgetResponse");
    assert!(
        head_enum.contains("Ok200 {")
            && head_enum.contains("e_tag: String,")
            && head_enum.contains("content_length: i64,"),
        "HEAD variant must carry the typed headers:\n{head_enum}"
    );
    assert!(
        !head_enum.contains("body"),
        "HEAD variants must not expose a body accessor:\n{head_enum}"
    );
    assert!(
        enum_block(&output, "GetWidgetResponse").contains("Ok200(Widget),"),
        "GET counterpart decodes its representation:\n{output}"
    );

    // The decode arm parses the headers and constructs the variant without
    // ever collecting or validating a body (§35), and sends no Accept
    // header (§29: HEAD decode nothing). The decode tail (shared
    // `decode_head_widget`) is where header parsing happens; slice BOTH fns.
    let head_tail = output
        .split_once("async fn decode_head_widget(")
        .expect("HEAD decode tail emitted")
        .1;
    let head_fn = head_tail.split("\n    }\n").next().expect("fn body");
    assert!(
        head_fn.contains("let e_tag = parse_required_header::<String>(&response, \"etag\")?;"),
        "\n{head_fn}"
    );
    assert!(
        head_fn.contains(
            "let content_length = parse_required_header::<i64>(&response, \
             \"content-length\")?;"
        ),
        "\n{head_fn}"
    );
    assert!(
        head_fn.contains("Ok(HeadWidgetResponse::Ok200 {\n                    e_tag,\n                    content_length,\n                })"),
        "\n{head_fn}"
    );
    assert!(
        !head_fn.contains("collect_reqwest_limited"),
        "HEAD must never buffer a response body:\n{head_fn}"
    );
    assert!(!head_fn.contains("ACCEPT"), "\n{head_fn}");
}

// ----------------------------------------------------------------------
// Issue #9 — schema named `<Operation>Response` collides with the response
// enum (R6)
// ----------------------------------------------------------------------

/// R6 reproducer for issue #9: a `components/schemas` entry named
/// `CreateItemResponse` collides with the `createItem` response enum.
/// `<Operation>Response` is generator-reserved, so the schema keeps the
/// clean name while the generated enum takes the companion §10 numeric
/// suffix (`CreateItemResponse_2`, the same rule `models.rs` applies to
/// nested anonymous collisions). The emitted client must define the name
/// exactly once, with the variant payload referencing the schema — never a
/// recursive infinite-size `E0072` self-reference behind an `E0255`
/// duplicate definition.
#[test]
fn issue_09_operation_response_schema_collision_suffixes_the_enum() {
    let dir = std::env::temp_dir().join(format!("o2r-issue-09-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let r6 = [
        "openapi: 3.0.0",
        "info: { title: R6, version: \"1.0.0\" }",
        "servers: [{ url: \"https://example.test/v1\" }]",
        "paths:",
        "  /items:",
        "    post:",
        "      operationId: createItem",
        "      responses:",
        "        \"201\":",
        "          description: ok",
        "          content:",
        "            application/json: { schema: { $ref: \"#/components/schemas/CreateItemResponse\" } }",
        "components:",
        "  schemas:",
        "    CreateItemResponse:",
        "      type: object",
        "      required: [id]",
        "      properties:",
        "        id: { type: string }",
        "",
    ]
    .join("\n");
    std::fs::write(dir.join("r6.yaml"), r6).expect("write R6 fixture");
    let ir = load_document("r6.yaml", &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("R6 must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("R6 must normalize: {diags:?}"));
    assert_eq!(
        doc.operations[0].response_enum, "CreateItemResponse_2",
        "the generated enum takes the suffix; the schema keeps the clean name"
    );
    let plan = plan_api(&doc).unwrap_or_else(|diags| panic!("R6 must plan: {diags:?}"));
    assert_eq!(
        plan.operations[0].response_enum_name, "CreateItemResponse_2",
    );

    let client = generate_client(&doc, &plan);
    assert!(
        client.contains("use super::models::CreateItemResponse;"),
        "the schema stays imported under its clean name:\n{client}"
    );
    let response_enum = enum_block(&client, "CreateItemResponse_2");
    assert!(
        response_enum.contains("Created201(CreateItemResponse),"),
        "the variant payload references the schema, not the enum itself:\n{response_enum}"
    );
    assert!(
        !client.contains("pub enum CreateItemResponse {"),
        "no bare duplicate enum definition (E0255) may remain:\n{client}"
    );

    let models = generate_models(&doc);
    assert!(
        struct_block(&models, "CreateItemResponse").contains("pub id: String,"),
        "the schema keeps its clean model definition:\n{models}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------------
// Issue #11 — inline anonymous composite bodies synthesize operation-based
// names (R5)
// ----------------------------------------------------------------------

/// R5 reproducer for issue #11: an inline object request body must plan
/// without `client_anonymous_json_schema` diagnostics by synthesizing
/// `CreateItemRequestBody` in models.rs. The scalar inline response stays an
/// inline `String`, and the response enum keeps the bare `CreateItemResponse`
/// name (the issue #9 reservation: synthesized response bodies would use the
/// `ResponseBody` suffix, never the bare enum name).
#[test]
fn issue_11_inline_request_body_synthesizes_operation_based_name() {
    let dir = std::env::temp_dir().join(format!("o2r-issue-11-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let r5 = [
        "openapi: 3.0.0",
        "info: { title: R5, version: \"1.0.0\" }",
        "servers: [{ url: \"https://example.test/v1\" }]",
        "paths:",
        "  /items:",
        "    post:",
        "      operationId: createItem",
        "      requestBody:",
        "        required: true",
        "        content:",
        "          application/json:",
        "            schema:",
        "              type: object",
        "              required: [name]",
        "              properties:",
        "                name: { type: string }",
        "      responses:",
        "        \"201\": { description: ok, content: { application/json: { schema: { type: string } } } }",
        "",
    ]
    .join("\n");
    std::fs::write(dir.join("r5.yaml"), r5).expect("write R5 fixture");
    let ir = load_document("r5.yaml", &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("R5 must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("R5 must normalize: {diags:?}"));
    assert_eq!(
        doc.names.synthetic_body_types.len(),
        1,
        "only the composite request body synthesizes a name: {:?}",
        doc.names.synthetic_body_types
    );
    let plan = plan_api(&doc).unwrap_or_else(|diags| panic!("R5 must plan: {diags:?}"));
    assert_eq!(
        plan.operations[0].request_contents[0].model_expr,
        "CreateItemRequestBody",
    );

    let models = generate_models(&doc);
    assert!(
        struct_block(&models, "CreateItemRequestBody").contains("pub name: String,"),
        "the synthesized body is defined in models.rs:\n{models}"
    );

    let client = generate_client(&doc, &plan);
    assert!(
        client.contains("use super::models::CreateItemRequestBody;"),
        "the synthesized body is imported under its operation-based name:\n{client}"
    );
    assert!(
        client.contains("&CreateItemRequestBody"),
        "the request method carries the synthesized body type:\n{client}"
    );
    assert!(
        enum_block(&client, "CreateItemResponse").contains("Created201(String),"),
        "the scalar inline response stays inline and the response enum keeps \
         its bare name (issue #9 reservation):\n{client}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------------------------------
// Issue #12 — security-scheme-aware auth + the default_headers escape hatch
// ----------------------------------------------------------------------

/// Synthetic bearer/apiKey/basic document: every operation's effective
/// requirement references a supported scheme, so the builder gains one typed
/// method per scheme on top of the general escape hatch.
const AUTH_FIXTURE: &str = r#"openapi: 3.1.0
info:
  title: auth api
  version: "1"
servers:
  - url: https://example.test/v1
security:
  - bearerAuth: []
  - apiKey: []
  - basicAuth: []
paths:
  /widgets:
    get:
      operationId: listWidgets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: string
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
    apiKey:
      type: apiKey
      in: header
      name: X-API-Key
    basicAuth:
      type: http
      scheme: basic
"#;

/// Renders the synthetic auth document through the full pipeline.
fn generate_auth_fixture(fixture: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("o2r-client-auth-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("auth.yaml"), fixture).expect("write synthetic fixture");

    let ir = load_document("auth.yaml", &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("auth fixture must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("auth fixture must normalize: {diags:?}"));
    let plan =
        plan_api(&doc).unwrap_or_else(|diags| panic!("auth fixture must plan: {diags:?}"));
    let output = generate_client(&doc, &plan);

    let _ = std::fs::remove_dir_all(&dir);
    output
}

#[test]
fn issue_12_bearer_api_key_and_basic_gain_typed_builder_methods() {
    let output = generate_auth_fixture(AUTH_FIXTURE, "typed");

    // The general escape hatch is always present: reqwest has no
    // middleware API, so `default_headers` is the only transport hook.
    assert!(
        output.contains("pub fn default_headers(mut self, headers: ::http::HeaderMap) -> Self {"),
        "the escape hatch must be emitted:\n{output}"
    );

    // One typed setter per supported scheme, storing the credential for
    // build-time materialization.
    assert!(
        output.contains("pub fn bearer_auth(mut self, token: impl Into<String>) -> Self {"),
        "\n{output}"
    );
    assert!(
        output.contains("pub fn api_key(mut self, key: impl Into<String>) -> Self {"),
        "\n{output}"
    );
    assert!(
        output.contains("pub fn basic_auth("),
        "\n{output}"
    );
    // Basic auth encodes dependency-free (no new manifest entry).
    assert!(
        output.contains(
            "fn encode_basic_auth_value(username: &str, password: Option<&str>) -> String {"
        ),
        "\n{output}"
    );

    // Build-time materialization: bearer and basic land on `authorization`,
    // the API key on its wire name, invalid values are InvalidHeader.
    let build = item_block(&output, "pub fn build(self)");
    assert!(
        build.contains("let mut auth_headers = ::http::HeaderMap::new();"),
        "\n{build}"
    );
    assert!(
        build.contains("format!(\"Bearer {token}\")"),
        "\n{build}"
    );
    assert!(
        build.contains("format!(\"Basic {raw}\")"),
        "\n{build}"
    );
    assert!(
        build.contains("::http::HeaderName::from_static(\"x-api-key\")"),
        "\n{build}"
    );
    assert!(
        build.contains("ClientError::InvalidHeader"),
        "\n{build}"
    );
    // The redirect-policy guarantee survives: still pinned off in `new`.
    assert!(output.contains("Policy::none()"), "\n{output}");
}

#[test]
fn issue_12_unsupported_and_unreferenced_schemes_gain_no_typed_methods() {
    // oauth2, query/cookie keys, and unknown types have no typed method —
    // `default_headers` still covers them — and schemes no requirement
    // references stay silent too.
    let fixture = r#"openapi: 3.1.0
info:
  title: partial auth
  version: "1"
servers:
  - url: https://example.test/v1
security:
  - bearerAuth: []
paths:
  /widgets:
    get:
      operationId: listWidgets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: string
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
    queryKey:
      type: apiKey
      in: query
      name: key
    oauth:
      type: oauth2
      flows: {}
    unreferenced:
      type: http
      scheme: bearer
"#;
    let output = generate_auth_fixture(fixture, "partial");

    assert!(
        output.contains("pub fn bearer_auth(mut self, token: impl Into<String>) -> Self {"),
        "\n{output}"
    );
    for absent in [
        "fn api_key(",
        "fn basic_auth(",
        "fn oauth(",
        "fn query_key(",
        "fn unreferenced(",
        "encode_basic_auth_value",
    ] {
        assert!(
            !output.contains(absent),
            "no typed method for `{absent}` may exist:\n{output}"
        );
    }
    assert!(
        output.contains("pub fn default_headers(mut self, headers: ::http::HeaderMap) -> Self {"),
        "the escape hatch still covers unmodeled schemes:\n{output}"
    );
}

#[test]
fn issue_12_documents_without_security_generate_no_auth_methods() {
    // Back-compat: schemes without any `security` requirement (and
    // documents without schemes at all) change only through the general
    // escape hatch — no typed methods, no credential fields, no helper.
    let fixture = r#"openapi: 3.1.0
info:
  title: no auth
  version: "1"
servers:
  - url: https://example.test/v1
paths:
  /widgets:
    get:
      operationId: listWidgets
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: string
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
"#;
    let output = generate_auth_fixture(fixture, "unrequired");

    assert!(
        output.contains("pub fn default_headers(mut self, headers: ::http::HeaderMap) -> Self {"),
        "\n{output}"
    );
    for absent in [
        "bearer_auth",
        "basic_auth",
        "api_key",
        "auth_headers",
        "encode_basic_auth_value",
        "InvalidHeader",
    ] {
        assert!(
            !output.contains(absent),
            "no auth surface for `{absent}` may exist without a requirement:\n{output}"
        );
    }
}
