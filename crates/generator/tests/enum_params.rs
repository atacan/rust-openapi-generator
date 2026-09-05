//! Enum-typed parameter coverage (issue #10): a `type: string` (or
//! `type: integer`) schema with an `enum` already generates a models.rs enum,
//! so `$ref` parameters over it must plan as scalar parameters — `Option<…>`
//! in both emitters, serialized through the serde shape — instead of failing
//! the whole document with `client_param_schema_unsupported`.
//!
//! The spec lives inline (written to a temp dir) so the directory-scanning
//! snapshot harnesses keep their committed fixture sets untouched.

use std::path::PathBuf;

use openapi_to_rust_generator::codegen::client::generate_client;
use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::{plan_api, PlannedApi};
use openapi_to_rust_generator::codegen::server::generate_server;
use openapi_to_rust_generator::normalize::{
    normalize_with_config, NormalizeConfig, NormalizedDocument,
};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

/// Issue #10 reproducer R4, extended: the string enum rides a path, query,
/// header, and cookie parameter plus an array query parameter, alongside an
/// integer enum and a plain string for control.
const SPEC: &str = r##"
openapi: 3.0.0
info: { title: R4, version: "1.0.0" }
servers: [{ url: "https://example.test/v1" }]
paths:
  /items/{category}:
    get:
      operationId: listItems
      parameters:
        - in: path
          name: category
          required: true
          schema: { $ref: "#/components/schemas/Format" }
        - in: query
          name: format
          required: false
          schema: { $ref: "#/components/schemas/Format" }
        - in: query
          name: limit
          required: true
          schema: { $ref: "#/components/schemas/PageSize" }
        - in: query
          name: tags
          required: false
          schema: { type: array, items: { $ref: "#/components/schemas/Format" } }
        - in: header
          name: X-Output
          required: false
          schema: { $ref: "#/components/schemas/Format" }
        - in: cookie
          name: session_format
          required: false
          schema: { $ref: "#/components/schemas/Format" }
        - in: query
          name: q
          required: false
          schema: { type: string }
      responses:
        "200": { description: ok, content: { application/json: { schema: { type: string } } } }
components:
  schemas:
    Format:
      type: string
      enum: [text, json]
    PageSize:
      type: integer
      enum: [10, 20, 50]
"##;

fn normalize_spec() -> NormalizedDocument {
    let dir = std::env::temp_dir().join(format!(
        "o2r-enum-params-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let name = "spec.yaml".to_owned();
    std::fs::write(dir.join(&name), SPEC).expect("write inline spec");
    let ir = load_document(&name, &dir, &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("spec must load: {diags:?}"));
    let _ = std::fs::remove_dir_all(&dir);
    normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("spec must normalize: {diags:?}"))
}

fn plan_spec(doc: &NormalizedDocument) -> PlannedApi {
    plan_api(doc).unwrap_or_else(|diags| panic!("enum params must plan: {diags:?}"))
}

#[test]
fn enum_typed_parameters_plan_without_diagnostics() {
    let doc = normalize_spec();
    let plan = plan_spec(&doc);
    let operation = plan.operations.iter().find(|op| op.operation_id.as_deref() == Some("listItems")).expect("listItems planned");
    let types: Vec<(&str, &str)> = operation
        .parameters
        .iter()
        .map(|parameter| (parameter.wire_name.as_str(), parameter.rust_type.as_str()))
        .collect();
    assert_eq!(
        types,
        vec![
            ("category", "Format"),
            ("format", "Format"),
            ("limit", "PageSize"),
            ("tags", "Vec<Format>"),
            ("X-Output", "Format"),
            ("session_format", "Format"),
            ("q", "String"),
        ],
        "enum params keep their models.rs identity; the plain scalar is untouched"
    );
}

#[test]
fn client_uses_enum_types_through_the_serde_shape() {
    let doc = normalize_spec();
    let plan = plan_spec(&doc);
    let client = generate_client(&doc, &plan);
    // The issue's expected signature, extended to every location.
    for signature in [
        "category: Format,",
        "format: Option<Format>,",
        "limit: PageSize,",
        "tags: Option<&[Format]>,",
        "x_output: Option<Format>,",
        "session_format: Option<Format>,",
        "q: Option<&str>,",
    ] {
        assert!(client.contains(signature), "missing `{signature}`\n{client}");
    }
    // Models import plus serde-shape conversion on every enum path.
    assert!(client.contains("use super::models::{Format, PageSize};"), "\n{client}");
    assert_eq!(
        client.matches("ParamValue::from_serde").count(),
        6,
        "path + query + required-int + array-map + header + cookie conversions\n{client}"
    );
    assert!(client.contains(
        "ParamValue::Array(raw.iter().map(ParamValue::from_serde).collect::<Vec<_>>())"
    ), "\n{client}");
    // Deterministic across repeated generation (main spec §50 test 39).
    assert_eq!(client, generate_client(&doc, &plan));
}

#[test]
fn server_decodes_enum_text_through_serde_helpers() {
    let doc = normalize_spec();
    let plan = plan_spec(&doc);
    let server = generate_server(&doc, &plan);
    // Mode A trait keeps the models.rs types end to end.
    for signature in [
        "category: Format,",
        "format: Option<Format>,",
        "limit: PageSize,",
        "tags: Option<Vec<Format>>,",
        "q: Option<String>,",
        "x_output: Option<Format>,",
        "session_format: Option<Format>,",
    ] {
        assert!(server.contains(signature), "missing `{signature}`\n{server}");
    }
    assert!(server.contains("use super::models::{Format, PageSize};"), "\n{server}");
    // String enums convert through the JSON string shape (the serde rename),
    // integer enums through the JSON number shape (the discriminant).
    assert!(server.contains("parse_string_enum::<Format>("), "\n{server}");
    assert!(server.contains("parse_int_enum::<PageSize>("), "\n{server}");
    assert!(server.contains("fn parse_string_enum<T: serde::de::DeserializeOwned>("), "\n{server}");
    assert!(server.contains("fn parse_int_enum<T: serde::de::DeserializeOwned>("), "\n{server}");
    assert_eq!(server, generate_server(&doc, &plan));
}

#[test]
fn models_still_emit_the_enums() {
    let models = generate_models(&normalize_spec());
    assert!(models.contains("pub enum Format {"), "\n{models}");
    assert!(models.contains("pub enum PageSize {"), "\n{models}");
}

#[test]
fn generated_enum_param_code_is_rustfmt_clean() {
    let Some(rustfmt) = locate_rustfmt() else {
        eprintln!(
            "WARNING: no rustfmt binary on PATH; skipping the rustfmt-clean assertion \
             (main spec §50 test 40)"
        );
        return;
    };
    let doc = normalize_spec();
    let plan = plan_spec(&doc);
    for (name, generated) in [
        ("client.rs", generate_client(&doc, &plan)),
        ("server.rs", generate_server(&doc, &plan)),
        ("models.rs", generate_models(&doc)),
    ] {
        let dir = std::env::temp_dir().join(format!(
            "o2r-enum-params-fmt-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let source = dir.join(name);
        std::fs::write(&source, &generated).expect("write generated module");
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
            "{name}: generated output is not rustfmt-clean\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&checked.stdout),
            String::from_utf8_lossy(&checked.stderr),
        );
    }
}

/// Resolves a usable rustfmt: plain PATH lookup first, then the rustup
/// shim next to the running toolchain's cargo (mirrors the client harness).
/// A candidate that cannot run (e.g. an uninstalled rustup shim) counts as
/// missing so the assertion skips instead of failing on the environment.
fn locate_rustfmt() -> Option<PathBuf> {
    let candidate = if which_exists("rustfmt") {
        PathBuf::from("rustfmt")
    } else {
        let cargo = PathBuf::from(std::env::var("CARGO").ok()?);
        let sibling = cargo.with_file_name("rustfmt");
        sibling.is_file().then_some(sibling)?
    };
    std::process::Command::new(&candidate)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| candidate)
}

fn which_exists(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}
