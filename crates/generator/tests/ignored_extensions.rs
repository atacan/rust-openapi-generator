//! Vendor-extension ignore/internal policy: `x-internal`, `x-fern-ignore`
//! and equivalent aliases exclude paths/operations and component schemas
//! from generated code, with an explicit diagnostic for public references
//! to ignored schemas.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::plan_api;
use openapi_to_rust_generator::diagnostics::Diagnostic;
use openapi_to_rust_generator::ir::document::IrDocument;
use openapi_to_rust_generator::normalize::{normalize_with_config, NormalizeConfig};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "o2r-ignored-{}-{}-{tag}",
            std::process::id(),
            DIR_COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.0.join(name), contents).expect("write fixture");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn doc_yaml(version: &str, body: &str) -> String {
    format!("openapi: \"{version}\"\ninfo:\n  title: t\n  version: \"1\"\n{body}")
}

fn load(dir: &Path, name: &str) -> Result<IrDocument, Vec<Diagnostic>> {
    load_document(name, dir, &LoadConfig::default())
}

fn load_with(dir: &Path, name: &str, config: &LoadConfig) -> Result<IrDocument, Vec<Diagnostic>> {
    load_document(name, dir, config)
}

fn has_code(diags: &[Diagnostic], code: &str) -> bool {
    diags.iter().any(|d| d.code == code)
}

fn operation_keys(doc: &IrDocument) -> Vec<String> {
    doc.paths
        .iter()
        .flat_map(|entry| {
            entry
                .operations
                .iter()
                .map(|(method, _)| format!("{} {}", method.as_keyword(), entry.path))
        })
        .collect()
}

// ----------------------------------------------------------------------
// Ignored path items
// ----------------------------------------------------------------------

#[test]
fn ignored_path_item_produces_no_operations() {
    for (version, extension) in [
        ("3.1.0", "x-internal: true"),
        ("3.1.0", "x-fern-ignore: true"),
        ("3.0.3", "x-internal: true"),
        ("3.0.3", "x-fern-ignore: true"),
    ] {
        let dir = TempDir::new("path-item");
        dir.write(
            "root.yaml",
            &doc_yaml(
                version,
                &format!(
                    r#"paths:
  /internal/users:
    {extension}
    get:
      operationId: listInternalUsers
      responses:
        '200':
          description: OK
  /public/users:
    get:
      operationId: listPublicUsers
      responses:
        '200':
          description: OK
"#
                ),
            ),
        );
        let doc = load(dir.path(), "root.yaml").expect("must load");
        let keys = operation_keys(&doc);
        assert_eq!(keys, ["get /public/users"], "{version} {extension}");
    }
}

#[test]
fn path_item_false_and_absent_preserve_behavior() {
    for body_extension in ["x-internal: false", "x-fern-ignore: false"] {
        let dir = TempDir::new("path-false");
        dir.write(
            "root.yaml",
            &doc_yaml(
                "3.1.0",
                &format!(
                    r#"paths:
  /users:
    {body_extension}
    get:
      operationId: listUsers
      responses:
        '200':
          description: OK
"#
                ),
            ),
        );
        let doc = load(dir.path(), "root.yaml").expect("must load");
        assert_eq!(operation_keys(&doc), ["get /users"], "{body_extension}");
    }

    // Non-boolean values never exclude.
    for body_extension in [
        "x-internal: \"true\"",
        "x-internal: 1",
        "x-fern-ignore: yes",
    ] {
        let dir = TempDir::new("path-nonbool");
        dir.write(
            "root.yaml",
            &doc_yaml(
                "3.1.0",
                &format!(
                    r#"paths:
  /users:
    {body_extension}
    get:
      operationId: listUsers
      responses:
        '200':
          description: OK
"#
                ),
            ),
        );
        let doc = load(dir.path(), "root.yaml").expect("must load");
        assert_eq!(operation_keys(&doc), ["get /users"], "{body_extension}");
    }
}

// ----------------------------------------------------------------------
// Ignored individual operations
// ----------------------------------------------------------------------

#[test]
fn ignored_single_operation_leaves_siblings() {
    for (version, extension) in [
        ("3.1.0", "x-internal: true"),
        ("3.1.0", "x-fern-ignore: true"),
        ("3.0.3", "x-internal: true"),
    ] {
        let dir = TempDir::new("single-op");
        dir.write(
            "root.yaml",
            &doc_yaml(
                version,
                &format!(
                    r#"paths:
  /users:
    get:
      operationId: listUsers
      responses:
        '200':
          description: OK
    post:
      operationId: createUser
      {extension}
      responses:
        '200':
          description: OK
"#
                ),
            ),
        );
        let doc = load(dir.path(), "root.yaml").expect("must load");
        assert_eq!(
            operation_keys(&doc),
            ["get /users"],
            "{version} {extension}"
        );
    }
}

#[test]
fn operation_false_value_preserves_operation() {
    let dir = TempDir::new("op-false");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /users:
    get:
      operationId: listUsers
      x-internal: false
      responses:
        '200':
          description: OK
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("must load");
    assert_eq!(operation_keys(&doc), ["get /users"]);
}

// ----------------------------------------------------------------------
// Ignored component schemas
// ----------------------------------------------------------------------

#[test]
fn ignored_component_schemas_produce_no_types() {
    for (version, extension) in [
        ("3.1.0", "x-internal: true"),
        ("3.1.0", "x-fern-ignore: true"),
        ("3.0.3", "x-internal: true"),
        ("3.0.3", "x-fern-ignore: true"),
    ] {
        let dir = TempDir::new("schemas");
        dir.write(
            "root.yaml",
            &doc_yaml(
                version,
                &format!(
                    r#"paths:
  /users:
    get:
      operationId: listUsers
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/PublicUser'
components:
  schemas:
    InternalUser:
      {extension}
      type: object
      properties:
        id:
          type: string
    PublicUser:
      type: object
      properties:
        id:
          type: string
"#
                ),
            ),
        );
        let doc = load(dir.path(), "root.yaml").expect("must load");
        assert!(
            !doc.schemas.contains_key("InternalUser"),
            "{version} {extension}: ignored schema must be absent"
        );
        assert!(doc.schemas.contains_key("PublicUser"));

        let normalized =
            normalize_with_config(doc, &NormalizeConfig::default()).expect("must normalize");
        assert!(!normalized.schemas.contains_key("InternalUser"));
        let models = generate_models(&normalized);
        assert!(
            !models.contains("InternalUser"),
            "{version} {extension}: no Rust type for ignored schema"
        );
        assert!(models.contains("PublicUser"));

        let plan = plan_api(&normalized).expect("must plan");
        assert_eq!(plan.operations.len(), 1);
        assert_eq!(plan.operations[0].method, "list_users");
    }
}

#[test]
fn component_schema_false_and_absent_preserve_types() {
    let dir = TempDir::new("schema-false");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths: {}
components:
  schemas:
    KeptFalse:
      x-internal: false
      type: object
      properties:
        id: {type: string}
    KeptAbsent:
      type: object
      properties:
        id: {type: string}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("must load");
    assert!(doc.schemas.contains_key("KeptFalse"));
    assert!(doc.schemas.contains_key("KeptAbsent"));
}

// ----------------------------------------------------------------------
// Public references to ignored schemas
// ----------------------------------------------------------------------

#[test]
fn public_operation_referencing_ignored_schema_is_an_error() {
    for extension in ["x-internal: true", "x-fern-ignore: true"] {
        let dir = TempDir::new("ref-ignored");
        dir.write(
            "root.yaml",
            &doc_yaml(
                "3.1.0",
                &format!(
                    r#"paths:
  /users:
    get:
      operationId: getUser
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/InternalUser'
components:
  schemas:
    InternalUser:
      {extension}
      type: object
      properties:
        id:
          type: string
"#
                ),
            ),
        );
        let err = load(dir.path(), "root.yaml").expect_err("must fail");
        assert!(
            has_code(&err, "ignored_schema_referenced"),
            "{extension}: {err:?}"
        );
        let diag = err
            .iter()
            .find(|d| d.code == "ignored_schema_referenced")
            .unwrap();
        assert!(
            diag.message.contains("InternalUser"),
            "diagnostic names the schema: {}",
            diag.message
        );
    }
}

#[test]
fn public_schema_referencing_ignored_schema_is_an_error() {
    let dir = TempDir::new("schema-ref-ignored");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.0.3",
            r#"paths: {}
components:
  schemas:
    InternalUser:
      x-internal: true
      type: object
      properties:
        id: {type: string}
    PublicEnvelope:
      type: object
      properties:
        user:
          $ref: '#/components/schemas/InternalUser'
"#,
        ),
    );
    let err = load(dir.path(), "root.yaml").expect_err("must fail");
    assert!(has_code(&err, "ignored_schema_referenced"), "{err:?}");
}

#[test]
fn ignored_operation_referencing_ignored_schema_is_silent() {
    // References held only by excluded operations must not surface.
    let dir = TempDir::new("ignored-ref-silent");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /internal/users:
    x-internal: true
    get:
      operationId: listInternalUsers
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/InternalUser'
components:
  schemas:
    InternalUser:
      x-internal: true
      type: object
      properties:
        id: {type: string}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("ignored refs stay silent");
    assert!(doc.paths.is_empty());
    assert!(!doc.schemas.contains_key("InternalUser"));
}

// ----------------------------------------------------------------------
// Unrelated extensions and centralized aliases
// ----------------------------------------------------------------------

#[test]
fn unrelated_vendor_extensions_never_exclude() {
    let dir = TempDir::new("unrelated");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /users:
    x-foo: true
    x-custom-internal: true
    x-internal-note: true
    get:
      operationId: listUsers
      x-bar: true
      responses:
        '200':
          description: OK
components:
  schemas:
    User:
      x-foo: true
      type: object
      properties:
        id: {type: string}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("must load");
    assert_eq!(operation_keys(&doc), ["get /users"]);
    assert!(doc.schemas.contains_key("User"));
}

#[test]
fn centralized_aliases_exclude() {
    // Every alias in the centralized list behaves identically.
    for extension in ["x-hidden: true", "x-exclude: true", "x-ignore: true"] {
        let dir = TempDir::new("alias");
        let body = r#"paths:
  /hidden:
    __EXT__
    get:
      operationId: hidden
      responses:
        '200':
          description: OK
components:
  schemas:
    Hidden:
      __EXT__
      type: object
      properties:
        id: {type: string}
"#
        .replace("__EXT__", extension);
        dir.write("root.yaml", &doc_yaml("3.1.0", &body));
        let doc = load(dir.path(), "root.yaml").expect("must load");
        assert!(doc.paths.is_empty(), "{extension}: path excluded");
        assert!(!doc.schemas.contains_key("Hidden"), "{extension}");
    }
}

#[test]
fn extra_ignored_extensions_are_configurable() {
    let dir = TempDir::new("extra");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /legacy:
    x-acme-hidden: true
    get:
      operationId: legacy
      responses:
        '200':
          description: OK
components:
  schemas:
    Legacy:
      x-acme-hidden: true
      type: object
      properties:
        id: {type: string}
"#,
        ),
    );
    // Default config does not know the custom key: nothing excluded.
    let doc = load(dir.path(), "root.yaml").expect("must load");
    assert_eq!(operation_keys(&doc), ["get /legacy"]);
    assert!(doc.schemas.contains_key("Legacy"));

    // With the custom key configured, both are excluded through the same
    // centralized policy.
    let config = LoadConfig {
        extra_ignored_extensions: vec!["x-acme-hidden".to_owned()],
        ..LoadConfig::default()
    };
    let doc = load_with(dir.path(), "root.yaml", &config).expect("must load");
    assert!(doc.paths.is_empty());
    assert!(!doc.schemas.contains_key("Legacy"));
}
