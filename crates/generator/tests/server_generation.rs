//! Server-emission harness: loads every committed fixture through
//! `load_document` → `normalize_with_config` → `codegen::plan::plan_api` →
//! `generate_server`, compares the rendered `server.rs` byte-for-byte against
//! snapshots under `tests/snapshots/`, asserts rustfmt-cleanliness (main spec
//! §50 test 40) and double-generation determinism (test 39), pins the §8
//! Output B / §22 / §23–§24 / §38 / §39 server shapes each fixture exists to
//! cover, and greps every snapshot for the forbidden patterns of §49.
//!
//! Snapshot regeneration: `SERVER_SNAPSHOT_UPDATE=1 cargo test`.

use std::path::PathBuf;

use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::{plan_api, plan_api_with_config, PlanConfig};
use openapi_to_rust_generator::codegen::server::generate_server;
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
];

/// Every fixture must plan + render without diagnostics.
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
    generate_server(&doc, &plan)
}

fn snapshot_name(fixture: &str) -> String {
    let stem = fixture.strip_suffix(".yaml").unwrap_or(fixture);
    format!("{stem}.server.rs")
}

// ----------------------------------------------------------------------
// Snapshots + double-generation determinism (main spec §50 test 39)
// ----------------------------------------------------------------------

#[test]
fn server_snapshots_match_byte_for_byte_and_generation_is_deterministic() {
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
        if std::env::var("SERVER_SNAPSHOT_UPDATE").as_deref() == Ok("1") {
            std::fs::write(&snapshot, &generated)
                .unwrap_or_else(|err| panic!("write snapshot {}: {err}", snapshot.display()));
            continue;
        }
        let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run with SERVER_SNAPSHOT_UPDATE=1",
                snapshot.display()
            )
        });
        assert_eq!(
            generated, expected,
            "{fixture}: generated server diverged from snapshot"
        );
    }
}

#[test]
fn every_fixture_plans_and_renders_without_diagnostics() {
    for fixture in ALL_FIXTURES {
        let first = generate_fixture(fixture);
        let second = generate_fixture(fixture);
        assert_eq!(first, second, "{fixture}: generation is not deterministic");
        assert!(
            first.contains("pub fn router("),
            "{fixture} must expose the router:\n{first}"
        );
        assert!(
            first.contains("#[::async_trait::async_trait]"),
            "{fixture} must annotate its API trait:"
        );
        assert!(
            first.contains("pub trait Api: Send + Sync + 'static"),
            "{fixture} untagged documents name the trait `Api`:"
        );
    }
}

// ----------------------------------------------------------------------
// rustfmt-clean emission (main spec §50 test 40)
// ----------------------------------------------------------------------

#[test]
fn generated_servers_are_rustfmt_clean() {
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
            "o2r-server-fmt-{}-{id}-{}",
            std::process::id(),
            fixture.trim_end_matches(".yaml")
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let source = dir.join(snapshot_name(fixture));
        std::fs::write(&source, &generated).expect("write generated server");

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
        if !stem.ends_with(".server.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("snapshot readable");
        for forbidden in [
            "serde_json::to_vec",
            "serde_json::to_string",
            "serde_urlencoded",
            "axum::Json(",
            "Form(",
        ] {
            assert!(
                !text.contains(forbidden),
                "{stem}: forbidden pattern `{forbidden}` (main spec §49)"
            );
        }
        // Every bounded response encoder must route through the generated
        // limited serializer, never an unbounded responder.
        assert!(
            text.contains("into_response_with_limits"),
            "{stem}: bounded encoder must be present"
        );
        checked += 1;
    }
    assert!(
        checked >= SNAPSHOT_FIXTURES.len(),
        "expected at least {} server snapshots, found {checked}",
        SNAPSHOT_FIXTURES.len()
    );
}

// ----------------------------------------------------------------------
// Fixture 01 — Example 1 shape (§8 Output B): buffered route wiring
// ----------------------------------------------------------------------

#[test]
fn fixture_01_router_installs_default_body_limit_and_emits_problem_json() {
    let output = generate_fixture("01_json_roundtrip.yaml");

    // §38 wiring: buffered JSON route installs DefaultBodyLimit at the
    // purpose-specific request limit.
    assert!(
        output.contains("DefaultBodyLimit::max(limits.structured_request_bytes)"),
        "buffered routes must install DefaultBodyLimit:\n{output}"
    );
    assert!(
        output.contains("::axum::routing::post(route_create_widget).layer("),
        "the POST route must carry the limit layer:\n{output}"
    );

    // The documented 400 variant keeps application/problem+json distinct
    // from generic application/json (§41/§8 Output B note).
    let encoder = item_block(&output, "impl CreateWidgetResponse {");
    assert!(
        encoder.contains("\"application/problem+json\""),
        "BadRequest400 arm must emit problem+json:\n{encoder}"
    );
    assert!(
        encoder.contains("::http::StatusCode::BAD_REQUEST"),
        "\n{encoder}"
    );

    // Mode A trait shape (§37): direct enum return, owned body value.
    assert!(
        output.contains("pub trait Api: Send + Sync + 'static"),
        "\n{output}"
    );
    assert!(
        output
            .contains("async fn create_widget(&self, body: CreateWidget) -> CreateWidgetResponse;"),
        "\n{output}"
    );

    // §34.1 fallback: overflow fires the hook and emits a fixed empty 500.
    assert!(output.contains("on_encode_overflow(operation_id, variant, limit)"),);
    assert!(
        output.contains("::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()"),
        "\n{output}"
    );
}

// ----------------------------------------------------------------------
// Fixture 02 — streaming upload/download (§9, §10, §32): exemption
// ----------------------------------------------------------------------

#[test]
fn fixture_02_streaming_routes_skip_the_body_limit_and_take_axum_bodies() {
    let output = generate_fixture("02_streaming_binary.yaml");

    // Nothing aggregates here: no route may install the limit layer (§38).
    assert!(
        !output.contains("DefaultBodyLimit::max("),
        "streaming-body routes must stay exempt from DefaultBodyLimit:\n{output}"
    );

    // Streaming download wrapper owns the raw axum body (§10 Output B with
    // D-impl-typed-headers-phase2 deferring header fields).
    let wrapper = struct_block(&output, "GetObject200");
    assert!(
        wrapper.contains("pub body: ::axum::body::Body,"),
        "\n{wrapper}"
    );

    // Trait takes the streaming body natively (§9 Output B).
    assert!(
        output.contains(
            "async fn put_object(&self, id: String, body: ::axum::body::Body) -> PutObjectResponse;"
        ),
        "\n{output}"
    );

    // Unit variant for the body-less 201 (§35).
    assert!(
        enum_block(&output, "PutObjectResponse").contains("Created201,"),
        "\n{output}"
    );

    // Route registered verbatim under the axum 0.8 `{param}` syntax.
    assert!(
        output.contains(".route(\"/objects/{id}\", ::axum::routing::put(route_put_object))"),
        "\n{output}"
    );
}

// ----------------------------------------------------------------------
// Fixture 03 — nested content enum (§11)
// ----------------------------------------------------------------------

#[test]
fn fixture_03_nested_content_enum_carries_axum_streaming_payloads() {
    let output = generate_fixture("03_nested_content.yaml");

    let content_enum = enum_block(&output, "GetArtifact200Content");
    assert!(
        content_enum.contains("Json(ArtifactMetadata),"),
        "\n{content_enum}"
    );
    assert!(
        content_enum.contains("OctetStream(::axum::body::Body),"),
        "streaming payloads are axum bodies (Output B):\n{content_enum}"
    );

    // Encoder dispatches per nested variant behind the correct literal.
    // (rustfmt canonicalizes short streaming arms into block form.)
    let encoder = item_block(&output, "impl GetArtifactResponse {");
    assert!(
        encoder.contains("GetArtifact200Content::OctetStream(value) => {")
            && encoder.contains(
                "stream_response(::http::StatusCode::OK, \"application/octet-stream\", value)"
            ),
        "\n{encoder}"
    );
    assert!(
        encoder.contains("GetArtifact200Content::Json(value) => encode_json_limited("),
        "\n{encoder}"
    );
}

// ----------------------------------------------------------------------
// Fixture 04 — ranges + default (§23, §24, §48)
// ----------------------------------------------------------------------

#[test]
fn fixture_04_checked_range_constructors_and_debug_asserts_exist() {
    let output = generate_fixture("04_status_ranges.yaml");

    // §48 checked constructors for every range/default variant.
    assert!(output.contains("pub fn success_2xx("), "\n{output}");
    assert!(output.contains("pub fn client_error_4xx("), "\n{output}");
    assert!(output.contains("pub fn default_status("), "\n{output}");

    // Membership validation guards.
    assert!(
        output.contains("(200..300).contains(&status.as_u16())"),
        "\n{output}"
    );
    assert!(
        output.contains("(400..500).contains(&status.as_u16())"),
        "\n{output}"
    );

    // The IntoResponse path debug-asserts membership (§24/§48).
    assert!(output.contains("debug_assert!("), "\n{output}");

    // Shared error type for the fallible constructors.
    assert!(
        output.contains("pub struct InvalidStatusRange;"),
        "\n{output}"
    );

    // Range/default variants carry the wire status (§23).
    let response_enum = enum_block(&output, "GetWidgetResponse");
    assert!(
        response_enum.contains("status: ::http::StatusCode,"),
        "\n{response_enum}"
    );

    // `default_status` refuses statuses other variants already cover (§24):
    // explicit 200, the 2XX range, and the 4XX range all appear as guards.
    let ctor = item_block(&output, "pub fn default_status(");
    assert!(ctor.contains("status.as_u16() == 200"), "\n{ctor}");
    assert!(ctor.matches("(200..300)").count() >= 1, "\n{ctor}");
    assert!(ctor.matches("(400..500)").count() >= 1, "\n{ctor}");
}

// ----------------------------------------------------------------------
// Wildcard content (§22) — synthetic document, no committed fixture
// ----------------------------------------------------------------------

// ----------------------------------------------------------------------
// Fixture 14 — wildcard/negotiation completion (§22, §25, §5.2, §28.5, §44)
// ----------------------------------------------------------------------

#[test]
fn fixture_14_trio_status_keeps_all_three_bounded_payloads() {
    let output = generate_fixture("14_negotiation.yaml");

    // Example 18 (§25): problem+json / json / text/plain all bounded.
    let trio = enum_block(&output, "GetReport400Content");
    assert!(trio.contains("ProblemJson(ProblemDetails),"), "{trio}");
    assert!(trio.contains("Json(LegacyError),"), "{trio}");
    assert!(
        trio.contains("TextPlain(String),"),
        "text/plain + string stays a bounded String on the server (§5.2):\n{trio}"
    );
}

#[test]
fn fixture_14_text_range_response_is_any_like_with_app_supplied_content_type() {
    let output = generate_fixture("14_negotiation.yaml");

    // §22 Output B applied to the §5.2 range: the server cannot know a
    // concrete textual type behind `text/*`, so it is Any-like and requires
    // the application to supply the actual Content-Type.
    let wrapper = struct_block(&output, "GetRawText200");
    assert!(
        wrapper.contains("pub content_type: ::mime::Mime,")
            && wrapper.contains("pub body: ::axum::body::Body,"),
        "text/* must be an Any-style payload:\n{wrapper}"
    );
    assert!(
        !wrapper.contains("String"),
        "text/* must never materialize as a bounded String:\n{wrapper}"
    );
}

#[test]
fn fixture_14_router_keeps_wildcard_dispatch_and_stream_override() {
    let output = generate_fixture("14_negotiation.yaml");

    // §28.5 exactly-one-entry rule survives in the emitted router helpers.
    assert!(
        output.contains("fn best_request_entry(")
            && output.contains("if is_wildcard_incoming(parsed) {")
            && output.contains("return if entries.len() == 1 { Some(0) } else { None };"),
        "the §28.5 wildcard-incoming rejection must stay wired:\n{output}"
    );

    // The {json, */*} request enum keeps the wildcard variant as a struct
    // carrying the negotiated Content-Type beside the raw body.
    let body_enum = enum_block(&output, "PostMirrorRequestBody");
    assert!(body_enum.contains("Json(Payload),"), "{body_enum}");
    assert!(
        body_enum.contains(
            "Any {\n        content_type: ::mime::Mime,\n        body: ::axum::body::Body,\n    },"
        ),
        "{body_enum}"
    );

    // §44 default vs override: put_note decodes bounded String;
    // post_stream_note passes the raw axum body through.
    let notes_trait = trait_method_block(&output, "put_note");
    assert!(notes_trait.contains("body: String"), "{notes_trait}");
    let stream_trait = trait_method_block(&output, "post_stream_note");
    assert!(
        stream_trait.contains("body: ::axum::body::Body"),
        "x-rust-body: stream must pass the raw body through:\n{stream_trait}"
    );

    // The streaming route skips bounded collection entirely (no
    // classify-and-collect machinery between extraction and invoke).
    let handler = item_block(&output, "async fn route_post_stream_note(");
    assert!(
        !handler.contains("collect_body_limited") && !handler.contains("decode_text_body"),
        "streaming override must not buffer or decode:\n{handler}"
    );
    assert!(
        handler.contains("PostStreamNoteRequestBody") || handler.contains("request_body"),
        "{handler}"
    );
}

// ----------------------------------------------------------------------
// Wildcard shapes — synthetic document
// ----------------------------------------------------------------------

const WILDCARD_FIXTURE: &str = r#"openapi: 3.1.0
info:
  title: wildcard shapes
  version: "1"
paths:
  /documents:
    get:
      operationId: getDocument
      responses:
        '200':
          description: Any representation
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Document'
            '*/*':
              schema: {}
  /uploads:
    post:
      operationId: uploadAnything
      requestBody:
        required: true
        content:
          '*/*':
            schema: {}
      responses:
        '204':
          description: Stored
components:
  schemas:
    Document:
      type: object
      required: [id]
      properties:
        id:
          type: string
"#;

#[test]
fn wildcard_content_variants_are_structs_carrying_mime_and_body() {
    let dir = std::env::temp_dir().join(format!("o2r-server-wildcard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture_path = dir.join("wildcard_shapes.yaml");
    std::fs::write(&fixture_path, WILDCARD_FIXTURE).expect("write synthetic fixture");

    let ir = load_document("wildcard_shapes.yaml", &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("wildcard fixture must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("wildcard fixture must normalize: {diags:?}"));
    let plan =
        plan_api(&doc).unwrap_or_else(|diags| panic!("wildcard fixture must plan: {diags:?}"));
    let output = generate_server(&doc, &plan);

    let _ = std::fs::remove_dir_all(&dir);

    // §22 Output B: the wildcard RESPONSE variant is a struct variant
    // carrying the concrete Content-Type beside the raw body.
    let content_enum = enum_block(&output, "GetDocument200Content");
    assert!(
        content_enum.contains(
            "Any {\n        content_type: ::mime::Mime,\n        body: ::axum::body::Body,\n    },"
        ),
        "wildcard content variant must be a struct variant:\n{content_enum}"
    );

    // Single-content wildcard REQUEST bodies become a dedicated struct the
    // router fills with the negotiated Content-Type (§28.5 exception).
    let request_struct = struct_block(&output, "UploadAnythingRequestBody");
    assert!(
        request_struct.contains("pub content_type: ::mime::Mime,")
            && request_struct.contains("pub body: ::axum::body::Body,"),
        "\n{request_struct}"
    );
    assert!(
        output.contains("mime_of(parsed.as_ref())"),
        "router must hand the negotiated media type to the payload:\n{output}"
    );
    assert!(
        !output.contains("DefaultBodyLimit::max("),
        "wildcard passthrough streams; the route stays exempt:\n{output}"
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

/// One trait method signature through its closing `;` (Mode A trait shape).
fn trait_method_block(output: &str, name: &str) -> String {
    let marker = format!("async fn {name}(");
    let start = output
        .find(&marker)
        .unwrap_or_else(|| panic!("trait method `{name}` not found"));
    let rest = &output[start..];
    let end = rest.find(';').map_or(rest.len(), |index| index + 1);
    rest[..end].to_owned()
}

// ----------------------------------------------------------------------
// Typed response headers (§15) + forms (§16) — synthetic document
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
  /forms:
    post:
      operationId: postForm
      requestBody:
        required: true
        content:
          application/x-www-form-urlencoded:
            schema:
              $ref: '#/components/schemas/FormIn'
      responses:
        '201':
          description: Created with typed headers
          headers:
            Location:
              required: true
              schema:
                type: string
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Doc'
components:
  schemas:
    Doc:
      type: object
      required: [id]
      properties:
        id:
          type: string
    FormIn:
      type: object
      required: [name]
      properties:
        name:
          type: string
        count:
          type: integer
          format: int32
"#;

static HEADERS_RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn generate_headers_fixture() -> String {
    let id = HEADERS_RUN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("o2r-server-headers-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture_path = dir.join("typed_headers.yaml");
    std::fs::write(&fixture_path, HEADERS_FIXTURE).expect("write synthetic fixture");

    let ir = load_document("typed_headers.yaml", &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must normalize: {diags:?}"));
    let plan =
        plan_api(&doc).unwrap_or_else(|diags| panic!("headers fixture must plan: {diags:?}"));
    let output = generate_server(&doc, &plan);
    let _ = std::fs::remove_dir_all(&dir);
    output
}

/// Pins the §15 Output B server shapes that no committed fixture covers:
/// multi-content hoisting onto the status VARIANT, the header-only struct
/// variant, and the §48 checked constructor on typed wrapper payloads.
#[test]
fn headers_hoist_and_header_only_variants_are_pinned() {
    let output = generate_headers_fixture();

    // Multi-content WITH documented headers: fields hoist onto the VARIANT.
    let response_enum = enum_block(&output, "GetMultiResponse");
    assert!(
        response_enum.contains("pub x_request_id: String,"),
        "{response_enum}"
    );
    assert!(
        response_enum.contains("pub retry_after_seconds: Option<i32>,"),
        "{response_enum}"
    );
    assert!(
        response_enum.contains("content: GetMulti200Content,"),
        "{response_enum}"
    );

    // Header-only 302: exactly the typed headers, never a body field.
    assert!(output.contains("Found302 {"), "\n{output}");
    assert!(
        response_enum.contains("pub location: String,"),
        "{response_enum}"
    );

    // Typed wrapper payload for the 201 carries Location + a domain body,
    // plus its §48 checked constructor.
    let wrapper = struct_block(&output, "PostForm201");
    assert!(wrapper.contains("pub location: String,"), "{wrapper}");
    assert!(wrapper.contains("pub body: Doc,"), "{wrapper}");
    let ctor = item_block(&output, "pub fn new(");
    assert!(
        ctor.contains(
            "::openapi_support::response_headers::checked_value(\"location\", &location)?;"
        ),
        "{ctor}"
    );

    // The IntoResponse path writes the collected headers through the shared
    // helper whose failure takes the §34.1 fallback.
    assert!(output.contains("fn write_typed_headers("), "\n{output}");
    assert!(
        output.contains("hook.on_encode_overflow(operation_id, variant, 0);"),
        "header-conversion failure fires the hook:\n{output}"
    );
}

/// The generated router self-decodes forms through the bounded support
/// decoder; axum's `Form` extractor is never used (main spec §16).
#[test]
fn form_requests_decode_through_the_support_decoder() {
    let output = generate_headers_fixture();

    let handler = item_block(&output, "async fn route_post_form(");
    assert!(
        handler.contains("decode_form_body(&bytes, limits.structured_request_bytes)?;"),
        "form bodies decode bounded (§16):\n{handler}"
    );
    assert!(
        handler.contains("if bytes.is_empty() {"),
        "empty body on a required form is MalformedBody (§28.3):\n{handler}"
    );
    assert!(!handler.contains("axum::Form"), "{handler}");

    let decoder = item_block(&output, "fn decode_form_body<T>(");
    assert!(
        decoder.contains("decode_form_limited(bytes, limit)"),
        "{decoder}"
    );
}

/// The synthetic §15 module must also be rustfmt-clean so its emission paths
/// stay canonical (main spec §50 test 40).
#[test]
fn headers_fixture_server_is_rustfmt_clean() {
    let Some(rustfmt) = locate_rustfmt() else {
        eprintln!("WARNING: no rustfmt on PATH; skipping");
        return;
    };
    let workdir =
        std::env::temp_dir().join(format!("o2r-server-headers-fmt-{}-fmt", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("create fixture dir");
    let fixture_path = workdir.join("typed_headers.yaml");
    std::fs::write(&fixture_path, HEADERS_FIXTURE).expect("write synthetic fixture");

    let ir = load_document("typed_headers.yaml", &workdir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("headers fixture must normalize: {diags:?}"));
    let plan =
        plan_api(&doc).unwrap_or_else(|diags| panic!("headers fixture must plan: {diags:?}"));
    let output = generate_server(&doc, &plan);

    let source = workdir.join("typed_headers.server.rs");
    std::fs::write(&source, &output).expect("write generated server");

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
        "synthetic headers server is not rustfmt-clean\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr),
    );
}

// ----------------------------------------------------------------------
// Fixture 11 — typed streaming multipart input (§17 Output B, §17.1)
// ----------------------------------------------------------------------

#[test]
fn fixture_11_multipart_input_streams_and_enforces_cardinality_pre_handler() {
    let output = generate_fixture("11_multipart.yaml");

    // The trait receives the owned streaming input struct, never Bytes.
    assert!(
        output.contains(
            "async fn upload_document(&self, body: UploadDocumentMultipartInput) -> UploadDocumentResponse;"
        ),
        "\n{output}"
    );
    let input = struct_block(&output, "UploadDocumentMultipartInput");
    // Required metadata MAY still arrive behind the streaming handoff, so
    // its value defers onto the live part (wire-arrival-based §17.1).
    assert!(
        input.contains("pub metadata: Option<DocumentMetadata>,"),
        "{input}"
    );
    assert!(
        input.contains("pub tags: Vec<String>,"),
        "repeated array parts collect in wire order:\n{input}"
    );
    assert!(
        input.contains("pub file: UploadDocumentFilePart,"),
        "binary part stays a live stream:\n{input}"
    );

    // The streaming part exposes chunked delivery; no buffered byte fields.
    let part = struct_block(&output, "UploadDocumentFilePart");
    assert!(
        output.contains("pub async fn next_chunk("),
        "chunks are pulled through next_chunk:\n{output}"
    );
    let input_and_part = format!("{input}{part}");
    // Binary payloads never rest in a field: every PUBLIC field stays free
    // of byte storage. (The live part keeps one PRIVATE bounded `Vec<u8>`
    // solely to decode trailing scalar/JSON parts behind the stream.)
    for field_line in input_and_part
        .lines()
        .filter(|line| line.trim_start().starts_with("pub "))
    {
        assert!(
            !field_line.contains("u8"),
            "binary payloads never rest in a public field:\n{field_line}"
        );
        assert!(
            !field_line.contains(": ::bytes::Bytes,"),
            "binary payloads never rest in a public field:\n{field_line}"
        );
    }
    assert!(
        part.contains("pending_required: Vec<&'static str>,")
            && part.contains("seen_single_valued: Vec<String>,")
            && part.contains("pub trailing_parts: UploadDocumentTrailingParts,"),
        "the live part carries the deferred enforcement state:\n{part}"
    );
    // A clean end-of-message with pending names reports exactly once.
    assert!(
        output.contains("missing required part(s) `{names}`"),
        "the terminal error names every outstanding required part:\n{output}"
    );
    // Trailing declared scalar/JSON parts decode instead of draining.
    assert!(
        output.contains("UploadDocumentFilePartTailStage::Metadata")
            && output.contains("UploadDocumentFilePartTailStage::TagsElement"),
        "trailing scalar/JSON parts decode behind the stream:\n{output}"
    );

    // §38/§17.1: the collector rejects duplicates and cardinality violations
    // pre-handler, and required parts missing at the natural end of the
    // stream (no handoff happened). (Whole-output greps: the collector spans
    // several blank-line-separated sections, so item_block would truncate.)
    assert!(
        output.contains("stream_multipart(") && output.contains("extract_boundary(parsed)"),
        "the router drives our framing engine itself:\n{output}"
    );
    assert!(
        output.contains("let handed_off = file_part.is_some();")
            && output.contains("None if handed_off => None,")
            && output.contains("missing required part `metadata`")
            && output.contains("missing required part `file`"),
        "required parts missing at stream end reject 422 pre-handler:\n{output}"
    );
    assert!(
        output.contains("duplicate single-valued part"),
        "duplicate single-valued parts reject 422:\n{output}"
    );
    assert!(
        output.contains("limits.multipart_scalar_part_bytes"),
        "scalar/JSON parts stay bounded (§17.1):\n{output}"
    );
    assert!(
        handler_block_contains(
            &output,
            "collect_upload_document_multipart(body, parsed.as_ref(), &limits)"
        ),
        "the handler collects before invoking the trait"
    );

    // Multipart routes are buffered-exempt (nothing aggregates the body).
    let route = item_block(&output, "pub fn router(");
    assert!(
        !route.contains(".layer("),
        "streaming multipart routes must not install DefaultBodyLimit:\n{route}"
    );
}

// ----------------------------------------------------------------------
// Fixture 12 — file-first wire order (§17.1/§38): wire-arrival-based
// ----------------------------------------------------------------------

#[test]
fn fixture_12_file_first_order_defers_required_parts_to_the_live_part() {
    let output = generate_fixture("12_multipart_order.yaml");

    // Required scalar/JSON parts ride Option in the input struct because
    // they may lawfully arrive behind the file-first handoff.
    let input = struct_block(&output, "UploadDocumentMultipartInput");
    assert!(
        input.contains("pub file: UploadDocumentFilePart,"),
        "{input}"
    );
    assert!(
        input.contains("pub metadata: Option<DocumentMetadata>,"),
        "{input}"
    );
    assert!(input.contains("pub source: Option<String>,"), "{input}");

    // Handoff seeds BOTH unseen required names plus duplicate protection.
    assert!(
        output.contains("pending_required.push(\"metadata\");")
            && output.contains("pending_required.push(\"source\");"),
        "unseen required names defer to the live part:\n{output}"
    );
    assert!(
        output.contains("self.pending_required.retain(|name| *name != \"metadata\");")
            && output.contains("self.pending_required.retain(|name| *name != \"source\");"),
        "trailing arrivals satisfy their pending names:\n{output}"
    );
    assert!(
        output.contains("\"missing required part(s) `{names}`\""),
        "one terminal SchemaViolation covers the plural case:\n{output}"
    );
}

/// Two binary fields have no representable server shape (one live-part slot,
/// §51.4 sequential semantics): planning stops with an Error diagnostic.
#[test]
fn two_binary_parts_stop_at_plan_time_with_a_diagnostic() {
    let dir = std::env::temp_dir().join(format!("o2r-two-binary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture_path = dir.join("two_binary_parts.yaml");
    std::fs::write(
        &fixture_path,
        r#"openapi: 3.1.0
info:
  title: two binary parts
  version: "1"
paths:
  /bundles:
    post:
      operationId: uploadBundle
      requestBody:
        required: true
        content:
          multipart/form-data:
            schema:
              type: object
              required: [primary, attachment]
              properties:
                primary:
                  type: string
                  format: binary
                attachment:
                  type: string
                  format: binary
      responses:
        '204':
          description: Stored
"#,
    )
    .expect("write synthetic fixture");

    let ir = load_document("two_binary_parts.yaml", &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("fixture must load: {diags:?}"));
    let doc = normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("fixture must normalize: {diags:?}"));
    let _ = std::fs::remove_dir_all(&dir);

    let error = plan_api(&doc).expect_err("two binary parts must stop at plan time");
    let diagnostic = error
        .iter()
        .find(|d| d.code == "multipart_schema_unsupported")
        .expect("diagnostic present");
    assert_eq!(
        diagnostic.severity,
        openapi_to_rust_generator::diagnostics::Severity::Error
    );
}

fn handler_block_contains(output: &str, needle: &str) -> bool {
    let handler = item_block(output, "async fn route_upload_document(");
    let trimmed = needle.replace(", ", ",\n            ");
    handler.contains(needle) || handler.contains(&trimmed)
}

// ----------------------------------------------------------------------
// Companion §9 — bucket-2 runtime validation wiring (fixture 13)
// ----------------------------------------------------------------------

#[test]
fn fixture_13_wires_post_decode_validation_calls_into_routes() {
    let output = generate_fixture("13_validation.yaml");
    // Composite JSON/form bodies call their inherent validator.
    assert!(
        output.contains("require_valid_request(\"body\", value.validate_request())?;"),
        "composite bodies must validate post-decode"
    );
    // Constrained scalar alias bodies + multipart parts call the free fn.
    assert!(
        output.contains("require_valid_request(\"body\", validate_slug_request(&value))?;"),
        "alias bodies must route through the models.rs free validator"
    );
    assert!(
        output.contains("require_valid_request(\"part `kind`\","),
        "multipart scalar parts backed by an alias must validate in the collector"
    );
}

#[test]
fn fixture_13_emits_model_validators_alongside_router_calls() {
    let doc = normalize_fixture("13_validation.yaml");
    let models = generate_models(&doc);
    assert!(models.contains("pub fn validate_request("));
    assert!(models.contains("pub fn validate_slug_request("));
    assert!(models.contains(".map_err(|error| error.at_field(\"code\"))"));
    assert!(models.contains("::openapi_support::validation::located("));
}

#[test]
fn server_runtime_validation_off_skips_calls_but_keeps_model_validators() {
    let doc = normalize_fixture("13_validation.yaml");

    let config = PlanConfig {
        server_runtime_validation: false,
        ..PlanConfig::default()
    };
    let plan = plan_api_with_config(&doc, &config).expect("plans without diagnostics");
    let off = generate_server(&doc, &plan);
    assert!(
        !off.contains("require_valid_request"),
        "policy off must skip every router validation CALL"
    );
    assert!(!off.contains("validate_slug_request"));

    // Validators themselves stay emitted: the flag gates calls only.
    let models = generate_models(&doc);
    assert!(models.contains("pub fn validate_request("));
}

// ----------------------------------------------------------------------
// Fixture 15 — streaming record formats (§18–§20 Output B, §40 wiring)
// ----------------------------------------------------------------------

#[test]
fn fixture_15_server_aliases_are_erased_pin_box_streams() {
    let output = generate_fixture("15_streams.yaml");

    // §18/§19/§20 Output B concrete-erased aliases over ServerStreamError.
    for (alias, item) in [
        ("ExportRecords200Stream", "Record"),
        ("StreamEvents200Stream", "Event"),
        ("StreamEnvelopeEvents200Stream", "EventPayload"),
    ] {
        assert!(
            output.contains(&format!(
                "pub type {alias} = ErasedItems<Result<{item}, ServerStreamError>>;"
            )),
            "{alias} alias missing:\n{output}"
        );
    }
    assert!(
        output.contains("pub type ErasedItems<T> ="),
        "shared boxed-stream alias emitted"
    );
    assert!(
        output.contains(
            "::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>;"
        ),
        "alias is Pin<Box<dyn Stream + Send>>"
    );
}

#[test]
fn fixture_15_encoder_arms_fire_hook_then_terminate_per_section_40() {
    let output = generate_fixture("15_streams.yaml");

    // Status + Content-Type commit BEFORE the body attaches (§40 order).
    let ndjson_arm = output
        .find("Self::Ok200(items) => {")
        .map(|index| &output[index..index + 2_000])
        .expect("ndjson arm");
    assert!(
        ndjson_arm.contains("let mut encoded = ::http::StatusCode::OK.into_response();"),
        "status commits first:\n{ndjson_arm}"
    );
    assert!(
        ndjson_arm.contains("\"application/x-ndjson\""),
        "literal Content-Type commits beside the status"
    );
    assert!(
        ndjson_arm.find("*encoded.body_mut()").unwrap()
            > ndjson_arm.find("headers_mut().insert(").unwrap(),
        "body attaches AFTER status + Content-Type + headers"
    );

    // Per-item encoding under max_stream_record_bytes, hook before abort.
    assert!(
        output.contains("limits.max_stream_record_bytes,"),
        "per-item encode bound"
    );
    assert!(
        output.contains("this.hook.on_stream_failure(this.operation_id, &error);"),
        "hook fires with the operation id before termination"
    );
    assert!(
        output.contains("::std::sync::Arc::clone(&stream_failure_hook),"),
        "router-supplied stream hook threads into the encoder"
    );
    assert!(
        output.contains("ServerStreamError::new("),
        "encode overflow surfaces as a terminal body error (abrupt)"
    );

    // Router signature carries the stream-failure hook beside the encode
    // hook; state stores it only when needed.
    assert!(
        output.contains(
            "stream_failure_hook: ::std::sync::Arc<dyn ::openapi_support::hooks::StreamFailureHook>,"
        ),
        "router gains the stream_failure_hook parameter"
    );
    assert!(
        output.contains("async fn push_metrics(&self, body: PushMetricsJsonSeqInput)")
            && output.contains("next_item(&mut self)"),
        "request-direction input wrapper drains via next_item"
    );
    assert!(
        output.contains("ProtocolRejection::new(RejectionKind::BodyTooLarge)"),
        "oversized streamed request records reject 413"
    );
}

/// Extended §49 greps for the streaming server snapshot: no aggregation, no
/// fabricated statuses after commit.
#[test]
fn no_aggregation_or_fabricated_statuses_in_streaming_snapshot() {
    let text = std::fs::read_to_string(snapshots_dir().join("15_streams.server.rs"))
        .expect("snapshot readable");
    for forbidden in [
        ".bytes()",
        "to_vec()",
        "serde_json::to_vec",
        "serde_urlencoded",
    ] {
        assert!(
            !text.contains(forbidden),
            "streaming snapshot aggregates via `{forbidden}` (§49)"
        );
    }
    // No INTERNAL_SERVER_ERROR fallback may appear inside the committed-
    // stream encoder arms: §40 forbids upgrading a committed response.
    let arm_start = output_probe(&text);
    if let Some(start) = arm_start {
        let window = &text[start..start + 3_000];
        assert!(
            !window.contains("INTERNAL_SERVER_ERROR"),
            "committed-stream arms must never fabricate statuses (§40)"
        );
    }
}

fn output_probe(text: &str) -> Option<usize> {
    text.find("Self::Ok200(items) => {")
}
