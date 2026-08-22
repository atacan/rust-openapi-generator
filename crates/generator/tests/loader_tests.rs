//! Integration tests for the document loader and `$ref` resolution engine.
//!
//! Each test exercises one rule from companion §2/§3, DECISIONS.md D-§2/D-§3,
//! or main spec §5/§18.1/§23/§24/§35. Fixtures are small inline documents;
//! external-ref tests write temporary files that are cleaned up via `Drop`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use openapi_to_rust_generator::diagnostics::{Diagnostic, Severity};
use openapi_to_rust_generator::ir::document::{
    ContentEntryIr, IrDocument, MediaClass, OpenApiVersion, OperationIr, ParameterLocation,
    ParameterStyle, RangeClass, ResponseStatusKey,
};
use openapi_to_rust_generator::ir::schema::{
    AdditionalPropertiesPolicy, EnumValues, Indirection, SchemaKind, SchemaNode, UnsupportedReason,
};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

// ----------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------

static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "o2r-loader-tests-{}-{}-{tag}",
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

/// Wraps `body` in a minimal document with the given `openapi` version.
fn doc_yaml(version: &str, body: &str) -> String {
    format!("openapi: \"{version}\"\ninfo:\n  title: t\n  version: \"1\"\n{body}")
}

fn load(dir: &Path, name: &str) -> Result<IrDocument, Vec<Diagnostic>> {
    load_document(name, dir, &LoadConfig::default())
}

fn load_with(dir: &Path, name: &str, config: &LoadConfig) -> Result<IrDocument, Vec<Diagnostic>> {
    load_document(name, dir, config)
}

fn node<'a>(doc: &'a IrDocument, name: &str) -> &'a SchemaNode {
    let id = *doc
        .schemas
        .get(name)
        .unwrap_or_else(|| panic!("component schema `{name}` missing"));
    doc.arena.get(id)
}

/// Follows one `Ref` wrapper from a `$ref`-bearing schema position to its
/// resolved target.
fn ref_target(
    doc: &IrDocument,
    id: openapi_to_rust_generator::ir::schema::SchemaId,
) -> openapi_to_rust_generator::ir::schema::SchemaId {
    match &doc.arena.get(id).kind {
        SchemaKind::Ref { target, .. } => *target,
        _ => panic!("expected a Ref node at {id:?}"),
    }
}

/// Target of an edge after following a possible `Ref` wrapper.
fn resolved_target(
    doc: &IrDocument,
    edge: &openapi_to_rust_generator::ir::schema::SchemaEdge,
) -> openapi_to_rust_generator::ir::schema::SchemaId {
    match &doc.arena.get(edge.target).kind {
        SchemaKind::Ref { target, .. } => *target,
        _ => edge.target,
    }
}

fn first_operation(doc: &IrDocument) -> &OperationIr {
    &doc.paths[0].operations[0].1
}

fn first_content(doc: &IrDocument) -> &ContentEntryIr {
    &first_operation(doc).responses[0].content[0]
}

fn has_code(diags: &[Diagnostic], code: &str) -> bool {
    diags.iter().any(|d| d.code == code)
}

// ----------------------------------------------------------------------
// 1. RFC 6901 pointer walking (~0 / ~1)
// ----------------------------------------------------------------------

#[test]
fn rfc6901_pointer_round_trip_resolves_escaped_names() {
    let dir = TempDir::new("rfc6901");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    slash/name:
      type: integer
    tilde~name:
      type: boolean
paths:
  /x:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/slash~1name'
  /y:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/tilde~0name'
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");

    let slashed = first_content(&doc);
    assert_eq!(slashed.media_type, "application/json");
    assert!(matches!(
        doc.arena.get(ref_target(&doc, slashed.schema)).kind,
        SchemaKind::Integer { format: None }
    ));

    // Second operation resolves the ~0-escaped name to its own node.
    let second = &doc.paths[1].operations[0].1.responses[0].content[0];
    assert!(matches!(
        doc.arena.get(ref_target(&doc, second.schema)).kind,
        SchemaKind::Boolean
    ));
}

// ----------------------------------------------------------------------
// 2. Memoization: shared refs share one arena id
// ----------------------------------------------------------------------

#[test]
fn shared_component_refs_share_one_arena_id() {
    let dir = TempDir::new("memo");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Widget:
      type: object
      properties:
        left:
          $ref: '#/components/schemas/Thing'
        right:
          $ref: '#/components/schemas/Thing'
      additionalProperties:
        $ref: '#/components/schemas/Thing'
    Thing:
      type: string
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let thing_id = *doc.schemas.get("Thing").unwrap();
    let widget = node(&doc, "Widget");
    let SchemaKind::Object {
        properties,
        additional,
    } = &widget.kind
    else {
        panic!("Widget must be an object");
    };
    let AdditionalPropertiesPolicy::Schema(extra_edge) = additional else {
        panic!("additionalProperties must carry a schema edge");
    };
    // Every edge — two properties plus additionalProperties — resolves to the
    // single memoized arena id interned for `Thing`. The property edges are
    // acyclic, so they stay direct after the cycle-precise pass
    // (companion §3, D-impl-boxing).
    assert_eq!(resolved_target(&doc, extra_edge), thing_id);
    for prop in properties {
        assert_eq!(resolved_target(&doc, &prop.schema), thing_id);
        assert_eq!(prop.schema.indirection, Indirection::None);
    }
    assert_eq!(widget.diagnostics, Vec::new());
}

/// D-impl-boxing: within one object, an edge closing a property-recursion
/// cycle stays `Boxed` while its acyclic sibling property is direct.
#[test]
fn boxing_is_cycle_precise_recursive_boxed_acyclic_direct() {
    let dir = TempDir::new("cycle-precise");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    ListNode:
      type: object
      properties:
        value:
          type: string
        next:
          $ref: '#/components/schemas/ListNode'
        label:
          $ref: '#/components/schemas/Label'
    Label:
      type: string
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let SchemaKind::Object { properties, .. } = &node(&doc, "ListNode").kind else {
        panic!("ListNode must be an object");
    };
    let indirection_of = |name: &str| {
        properties
            .iter()
            .find(|p| p.wire_name == name)
            .unwrap_or_else(|| panic!("property `{name}`"))
            .schema
            .indirection
    };
    assert_eq!(
        indirection_of("next"),
        Indirection::Boxed,
        "`next` closes the self-recursion cycle"
    );
    assert_eq!(
        indirection_of("value"),
        Indirection::None,
        "acyclic scalar property stays direct"
    );
    assert_eq!(
        indirection_of("label"),
        Indirection::None,
        "acyclic ref-property stays direct"
    );
}

/// D-impl-boxing: a two-node property cycle boxes both participating edges
/// while unrelated objects stay direct.
#[test]
fn two_node_property_cycle_boxes_both_participants() {
    let dir = TempDir::new("cycle-pair");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Left:
      type: object
      properties:
        right:
          $ref: '#/components/schemas/Right'
        name:
          type: string
    Right:
      type: object
      properties:
        left:
          $ref: '#/components/schemas/Left'
    Standalone:
      type: object
      properties:
        other:
          $ref: '#/components/schemas/Standalone2'
    Standalone2:
      type: string
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let SchemaKind::Object {
        properties: left, ..
    } = &node(&doc, "Left").kind
    else {
        panic!("Left must be an object");
    };
    let indirection_of = |properties: &[openapi_to_rust_generator::ir::schema::PropertyIr],
                          name: &str| {
        properties
            .iter()
            .find(|p| p.wire_name == name)
            .unwrap_or_else(|| panic!("property `{name}`"))
            .schema
            .indirection
    };
    assert_eq!(indirection_of(left, "right"), Indirection::Boxed);
    assert_eq!(indirection_of(left, "name"), Indirection::None);

    let SchemaKind::Object {
        properties: right, ..
    } = &node(&doc, "Right").kind
    else {
        panic!("Right must be an object");
    };
    assert_eq!(indirection_of(right, "left"), Indirection::Boxed);

    // Acyclic chain across components stays direct.
    let SchemaKind::Object {
        properties: standalone,
        ..
    } = &node(&doc, "Standalone").kind
    else {
        panic!("Standalone must be an object");
    };
    assert_eq!(indirection_of(standalone, "other"), Indirection::None);
}

// ----------------------------------------------------------------------
// 3. Sibling rules around $ref (companion §3)
// ----------------------------------------------------------------------

#[test]
fn v30_ref_siblings_warn_and_are_ignored() {
    let dir = TempDir::new("v30-sibs");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.0.3",
            r#"components:
  schemas:
    Dog:
      type: string
    Cat:
      $ref: '#/components/schemas/Dog'
      description: should be ignored with a warning
      default: four
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("warnings do not fail the load");
    let dog_id = *doc.schemas.get("Dog").unwrap();

    let cat = node(&doc, "Cat");
    let SchemaKind::Ref {
        target,
        summary,
        description,
        inline_constraints,
    } = &cat.kind
    else {
        panic!("Cat must collapse to a Ref");
    };
    assert_eq!(*target, dog_id);
    assert_eq!(*summary, None);
    assert_eq!(*description, None);
    assert!(inline_constraints.is_empty());
    assert_eq!(cat.diagnostics.len(), 1);
    assert_eq!(cat.diagnostics[0].code, "sibling_ignored");
    assert_eq!(cat.diagnostics[0].severity, Severity::Warning);
    assert!(cat.diagnostics[0].message.contains("description"));
}

#[test]
fn v31_reference_object_accepts_summary_description() {
    let dir = TempDir::new("v31-refobj");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /items:
    get:
      parameters:
        - $ref: '#/components/parameters/Limit'
          summary: page size
          description: how many items to return
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: integer}
components:
  parameters:
    Limit:
      name: limit
      in: query
      schema: {type: integer}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let parameter = &first_operation(&doc).parameters[0];
    assert_eq!(parameter.name, "limit");
    assert_eq!(parameter.location, ParameterLocation::Query);
    assert_eq!(parameter.style, ParameterStyle::Form);
    assert!(parameter.explode);
    assert!(!parameter.required);
    assert_eq!(parameter.summary.as_deref(), Some("page size"));
    assert_eq!(
        parameter.description.as_deref(),
        Some("how many items to return")
    );
    assert!(matches!(
        doc.arena.get(parameter.schema).kind,
        SchemaKind::Integer { format: None }
    ));
}

#[test]
fn v31_reference_object_unknown_sibling_is_ignored_with_warning() {
    // Global warnings only survive when the load fails, so an unrelated
    // error (`999` is not a valid status key) forces the Err path.
    let dir = TempDir::new("v31-refobj-warn");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /items:
    get:
      parameters:
        - $ref: '#/components/parameters/Limit'
          required: true
      responses:
        '999':
          description: forces an error diagnostic
components:
  parameters:
    Limit:
      name: limit
      in: query
      schema: {type: integer}
"#,
        ),
    );
    let err = load(dir.path(), "root.yaml").expect_err("999 is invalid");
    assert!(has_code(&err, "sibling_ignored"), "{err:?}");
    assert!(err.iter().any(|d| d.message.contains("required")));
}

#[test]
fn inline_v31_entities_carry_their_own_descriptions() {
    // Inline (non-`$ref`) Parameter/Header objects declare only
    // `description`; it lands on the IR while `summary` stays `None`
    // (companion §3).
    let dir = TempDir::new("v31-inline-desc");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /items:
    get:
      parameters:
        - name: limit
          in: query
          description: how many items to return
          schema: {type: integer}
      responses:
        '200':
          description: the matching page
          headers:
            X-Total:
              description: total number of items
              schema: {type: integer}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");

    let parameter = &first_operation(&doc).parameters[0];
    assert_eq!(parameter.summary, None);
    assert_eq!(
        parameter.description.as_deref(),
        Some("how many items to return")
    );

    let response = &first_operation(&doc).responses[0];
    assert_eq!(response.summary, None);
    assert_eq!(response.description.as_deref(), Some("the matching page"));

    let header = &response.headers[0].1;
    assert_eq!(header.summary, None);
    assert_eq!(header.description.as_deref(), Some("total number of items"));
}

#[test]
fn v31_schema_position_ref_preserves_siblings_as_inline_constraints() {
    let dir = TempDir::new("v31-sibs");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Animal:
      type: string
    Cat:
      $ref: '#/components/schemas/Animal'
      maxLength: 5
      pattern: "^c"
      summary: domestic cat
      description: a cat
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let animal_id = *doc.schemas.get("Animal").unwrap();

    let cat = node(&doc, "Cat");
    let SchemaKind::Ref {
        target,
        summary,
        description,
        inline_constraints,
    } = &cat.kind
    else {
        panic!("Cat must be a Ref node");
    };
    assert_eq!(*target, animal_id);
    assert_eq!(summary.as_deref(), Some("domestic cat"));
    assert_eq!(description.as_deref(), Some("a cat"));
    assert_eq!(
        inline_constraints.len(),
        1,
        "siblings become conjunction terms"
    );

    let constraint = doc.arena.get(inline_constraints[0].target);
    assert_eq!(constraint.validation.max_length, Some(5));
    assert_eq!(constraint.validation.pattern.as_deref(), Some("^c"));
}

// ----------------------------------------------------------------------
// 4. External file references (D-§3)
// ----------------------------------------------------------------------

#[test]
fn external_relative_file_refs_resolve() {
    let dir = TempDir::new("external-ok");
    fs::create_dir_all(dir.path().join("inc")).unwrap();
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /w:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: './inc/common.yaml#/components/schemas/Thing'
"#,
        ),
    );
    dir.write(
        "inc/common.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Thing:
      type: object
      properties:
        inherited:
          $ref: '../shared.yaml#/components/schemas/Mixin'
"#,
        ),
    );
    dir.write(
        "shared.yaml",
        "components:\n  schemas:\n    Mixin:\n      type: string\n",
    );

    let doc = load(dir.path(), "root.yaml").expect("resolves across files");
    // The content schema is a `$ref` site; follow it to the target object.
    let thing = doc.arena.get(ref_target(&doc, first_content(&doc).schema));
    let SchemaKind::Object { properties, .. } = &thing.kind else {
        panic!("Thing must be an object");
    };
    assert_eq!(properties[0].wire_name, "inherited");
    let mixin = resolved_target(&doc, &properties[0].schema);
    assert!(matches!(
        doc.arena.get(mixin).kind,
        SchemaKind::String_ { binary: false, .. }
    ));
}

#[test]
fn missing_external_file_is_an_error() {
    let dir = TempDir::new("external-missing");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /w:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: './nowhere.yaml#/Thing'
"#,
        ),
    );
    let err = load(dir.path(), "root.yaml").expect_err("missing file");
    assert!(has_code(&err, "ref_external_missing"));
    assert!(err.iter().any(|d| d.message.contains("nowhere.yaml")));
}

#[test]
fn remote_http_ref_is_an_error() {
    let dir = TempDir::new("external-remote");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /w:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                $ref: 'https://example.com/api.yaml#/Thing'
"#,
        ),
    );
    let err = load(dir.path(), "root.yaml").expect_err("remote refs never fetched");
    assert!(has_code(&err, "ref_remote_url"), "{err:?}");
}

// ----------------------------------------------------------------------
// 5. Cycles (companion §3)
// ----------------------------------------------------------------------

#[test]
fn property_recursion_boxes_edges() {
    let dir = TempDir::new("cycle-prop");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Node:
      type: object
      properties:
        child:
          $ref: '#/components/schemas/Node'
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("property recursion terminates");
    let node_id = *doc.schemas.get("Node").unwrap();
    let SchemaKind::Object { properties, .. } = &node(&doc, "Node").kind else {
        panic!("Node must be an object");
    };
    let child = properties.iter().find(|p| p.wire_name == "child").unwrap();
    assert_eq!(
        resolved_target(&doc, &child.schema),
        node_id,
        "edge resolves to Node itself"
    );
    assert_eq!(child.schema.indirection, Indirection::Boxed);
}

#[test]
fn array_items_break_cycles_without_boxing() {
    let dir = TempDir::new("cycle-array");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Tree:
      items:
        $ref: '#/components/schemas/Tree'
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("array recursion terminates");
    let tree_id = *doc.schemas.get("Tree").unwrap();
    let SchemaKind::Array { items } = &node(&doc, "Tree").kind else {
        panic!("Tree must be an array");
    };
    assert_eq!(resolved_target(&doc, items), tree_id);
    assert_eq!(items.indirection, Indirection::None);
}

#[test]
fn self_referential_all_of_is_rejected() {
    let dir = TempDir::new("cycle-allof");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Loop:
      allOf:
        - $ref: '#/components/schemas/Loop'
"#,
        ),
    );
    let err = load(dir.path(), "root.yaml").expect_err("unbroken self-containment");
    assert!(has_code(&err, "ref_self_containment"), "{err:?}");
}

// ----------------------------------------------------------------------
// 6. Inline expansion depth cap
// ----------------------------------------------------------------------

#[test]
fn inline_depth_cap_errors_instead_of_truncating() {
    let dir = TempDir::new("depth-cap");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Deep:
      type: array
      items:
        type: array
        items:
          type: array
          items: {type: string}
"#,
        ),
    );
    let config = LoadConfig {
        max_inline_depth: 2,
        ..LoadConfig::default()
    };
    let err = load_with(dir.path(), "root.yaml", &config)
        .expect_err("nesting deeper than the cap is an error");
    assert!(has_code(&err, "inline_depth_exceeded"), "{err:?}");
}

// ----------------------------------------------------------------------
// 7. Version detection
// ----------------------------------------------------------------------

#[test]
fn version_detection_three_releases_ok_two_point_zero_rejected() {
    let dir = TempDir::new("versions");
    for (raw, expected) in [
        ("3.0.3", OpenApiVersion::V3_0),
        ("3.1.0", OpenApiVersion::V3_1),
        ("3.2.0", OpenApiVersion::V3_2),
    ] {
        dir.write("root.yaml", &doc_yaml(raw, ""));
        let doc =
            load(dir.path(), "root.yaml").unwrap_or_else(|e| panic!("{raw} should load: {e:?}"));
        assert_eq!(doc.version, expected);
        assert_eq!(doc.raw_version, raw);
    }

    dir.write("root.yaml", &doc_yaml("2.0", ""));
    let err = load(dir.path(), "root.yaml").expect_err("2.0 unsupported");
    assert!(has_code(&err, "version_unsupported"));
    assert!(err[0].message.contains("supported"));
}

// ----------------------------------------------------------------------
// 8. OAS 3.0 normalization rows + 3.1 type arrays
// ----------------------------------------------------------------------

#[test]
fn v30_normalization_rows_apply() {
    let dir = TempDir::new("v30-norm");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.0.3",
            r#"components:
  schemas:
    NullableString:
      type: string
      nullable: true
    PositiveInt:
      type: integer
      minimum: 5
      exclusiveMinimum: true
    Bytes:
      type: string
      format: binary
    LegacyBinaryType:
      type: binary
    LegacyFileType:
      type: file
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");

    let nullable = node(&doc, "NullableString");
    assert!(
        nullable.nullable,
        "nullable:true becomes the nullability dimension"
    );

    let positive = node(&doc, "PositiveInt");
    assert_eq!(
        positive.validation.numeric.exclusive_minimum,
        Some(5.0),
        "boolean exclusiveMinimum folds onto the bound"
    );
    assert_eq!(positive.validation.numeric.minimum, None);

    for name in ["Bytes", "LegacyBinaryType", "LegacyFileType"] {
        let SchemaKind::String_ { binary, .. } = &node(&doc, name).kind else {
            panic!("{name} must normalize to a string kind");
        };
        assert!(binary, "{name} carries the binary payload marker");
    }
}

#[test]
fn v31_type_arrays_normalize_or_fallback() {
    let dir = TempDir::new("v31-types");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    NullableString:
      type: [string, "null"]
    MixedTypes:
      type: [string, integer]
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml")
        .expect("mixed type arrays fall back instead of failing the load");

    let nullable = node(&doc, "NullableString");
    assert!(nullable.nullable);
    assert!(matches!(nullable.kind, SchemaKind::String_ { .. }));

    let mixed = node(&doc, "MixedTypes");
    assert!(matches!(
        mixed.kind,
        SchemaKind::NotSupported {
            reason: UnsupportedReason::MixedTypeArray
        }
    ));
    assert!(mixed
        .diagnostics
        .iter()
        .any(|d| d.code == "type_array_mixed"));
    assert_eq!(mixed.diagnostics[0].severity, Severity::Error);
}

// ----------------------------------------------------------------------
// 9. Informational statuses rejected (main spec §35)
// ----------------------------------------------------------------------

#[test]
fn informational_status_keys_are_rejected() {
    let dir = TempDir::new("status-1xx");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /x:
    get:
      responses:
        '150':
          description: not an operation outcome
        '200':
          description: ok
        '1XX':
          description: informational range
"#,
        ),
    );
    let err = load(dir.path(), "root.yaml").expect_err("1xx keys/ranges rejected");
    let informational: Vec<&Diagnostic> = err
        .iter()
        .filter(|d| d.code == "response_status_informational")
        .collect();
    assert_eq!(
        informational.len(),
        2,
        "'150' and '1XX' each produce one error"
    );
    assert!(informational.iter().any(|d| d.message.contains("150")));
    assert!(informational.iter().any(|d| d.message.contains("1XX")));
}

// ----------------------------------------------------------------------
// 10. `in: querystring` rejected (companion §6)
// ----------------------------------------------------------------------

#[test]
fn querystring_parameter_location_is_rejected() {
    let dir = TempDir::new("querystring");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.2.0",
            r#"paths:
  /search:
    get:
      parameters:
        - name: q
          in: querystring
          schema: {type: string}
      responses:
        '200':
          description: ok
"#,
        ),
    );
    let err = load(dir.path(), "root.yaml").expect_err("querystring rejected in v1");
    assert!(has_code(&err, "parameter_location_querystring"), "{err:?}");
}

// ----------------------------------------------------------------------
// 11. x-rust-stream-item override (main spec §18.1)
// ----------------------------------------------------------------------

#[test]
fn stream_item_override_captured_on_content_entry() {
    let dir = TempDir::new("stream-item");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /events:
    get:
      responses:
        '200':
          description: sse
          content:
            text/event-stream:
              schema:
                $ref: '#/components/schemas/Envelope'
              x-rust-stream-item:
                $ref: '#/components/schemas/Event'
components:
  schemas:
    Envelope:
      type: object
      properties:
        data:
          $ref: '#/components/schemas/Event'
    Event:
      type: string
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let entry = first_content(&doc);
    assert_eq!(entry.media_class, MediaClass::EventStream);
    assert!(!entry.is_wildcard);

    // Both the envelope schema and the stream-item override are `$ref` sites;
    // follow them to the memoized targets.
    let override_id = entry.stream_item_override.expect("override captured");
    let event_id = *doc.schemas.get("Event").unwrap();
    assert_eq!(
        ref_target(&doc, override_id),
        event_id,
        "override shares the memoized Event id"
    );
    let envelope_id = *doc.schemas.get("Envelope").unwrap();
    assert_ne!(entry.schema, override_id);
    assert_eq!(
        ref_target(&doc, entry.schema),
        envelope_id,
        "schema stays the envelope"
    );

    let envelope = node(&doc, "Envelope");
    let SchemaKind::Object { properties, .. } = &envelope.kind else {
        panic!("Envelope must be an object");
    };
    assert_eq!(resolved_target(&doc, &properties[0].schema), event_id);
}

// ----------------------------------------------------------------------
// 12. Servers at all three levels (companion §8)
// ----------------------------------------------------------------------

#[test]
fn servers_captured_at_all_levels_with_variables() {
    let dir = TempDir::new("servers");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"servers:
  - url: https://{region}.api.example.com:{port}/v1
    variables:
      region:
        default: us-east
        enum: [us-east, eu-west]
      port:
        default: "443"
paths:
  /p:
    servers:
      - url: /path-base
    get:
      servers:
        - url: /op-base
      responses:
        '200':
          description: ok
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");

    assert_eq!(doc.servers.len(), 1);
    let root_server = &doc.servers[0];
    assert_eq!(
        root_server.url,
        "https://{region}.api.example.com:{port}/v1"
    );
    assert_eq!(root_server.variables.len(), 2);
    assert_eq!(root_server.variables[0].0, "region");
    assert_eq!(root_server.variables[0].1.default, "us-east");
    assert_eq!(
        root_server.variables[0].1.allowed_enum,
        Some(vec!["us-east".to_owned(), "eu-west".to_owned()])
    );
    assert_eq!(root_server.variables[1].1.default, "443");
    assert_eq!(root_server.variables[1].1.allowed_enum, None);

    let entry = &doc.paths[0];
    assert_eq!(
        entry.servers.as_ref().map(|s| s[0].url.as_str()),
        Some("/path-base"),
        "path-level overrides root"
    );
    assert_eq!(
        first_operation(&doc)
            .servers
            .as_ref()
            .map(|s| s[0].url.as_str()),
        Some("/op-base"),
        "operation-level overrides path"
    );
}

// ----------------------------------------------------------------------
// Extras proving adjacent rules
// ----------------------------------------------------------------------

#[test]
fn wildcard_media_ranges_flagged_raw_unknown() {
    let dir = TempDir::new("wildcards");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /x:
    get:
      responses:
        '200':
          description: ok
          content:
            '*/*':
              schema: {}
            'text/*':
              schema: {}
            application/json:
              schema: {}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let entries = &first_operation(&doc).responses[0].content;
    assert_eq!(entries.len(), 3);

    assert!(entries[0].is_wildcard);
    assert_eq!(entries[0].media_class, MediaClass::RawUnknown);
    assert_eq!(entries[0].media_type, "*/*");

    assert!(entries[1].is_wildcard);
    assert_eq!(entries[1].media_class, MediaClass::RawUnknown);

    assert!(!entries[2].is_wildcard);
    assert_eq!(entries[2].media_class, MediaClass::JsonFamily);
}

#[test]
fn free_form_any_value_and_deny_policies() {
    let dir = TempDir::new("freeform");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    FreeForm:
      type: object
    Anything:
      {}
    DenyExtra:
      type: object
      additionalProperties: false
    UnevaluatedDeny:
      type: object
      unevaluatedProperties: false
    UnevaluatedActive:
      type: object
      unevaluatedProperties: {type: string}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");

    assert!(matches!(
        node(&doc, "FreeForm").kind,
        SchemaKind::FreeFormObject
    ));
    assert!(matches!(node(&doc, "Anything").kind, SchemaKind::AnyValue));

    let SchemaKind::Object { additional, .. } = &node(&doc, "DenyExtra").kind else {
        panic!("DenyExtra must stay an object");
    };
    assert_eq!(*additional, AdditionalPropertiesPolicy::Deny);

    // D-§2: standalone unevaluatedProperties:false ≡ additionalProperties:false.
    let SchemaKind::Object { additional, .. } = &node(&doc, "UnevaluatedDeny").kind else {
        panic!("UnevaluatedDeny must behave as a denying object");
    };
    assert_eq!(*additional, AdditionalPropertiesPolicy::Deny);

    let active = node(&doc, "UnevaluatedActive");
    assert!(matches!(
        active.kind,
        SchemaKind::NotSupported {
            reason: UnsupportedReason::UnevaluatedKeywordsActive
        }
    ));
}

#[test]
fn status_ranges_and_default_keys_parse_in_declaration_order() {
    let dir = TempDir::new("status-keys");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"paths:
  /x:
    get:
      responses:
        '200':
          description: explicit beats ranges at normalization time
        '2XX':
          description: other successes
        '404':
          description: gone
        default:
          description: undocumented errors
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");
    let statuses: Vec<ResponseStatusKey> = first_operation(&doc)
        .responses
        .iter()
        .map(|entry| entry.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            ResponseStatusKey::Explicit(200),
            ResponseStatusKey::RangeClass(RangeClass::Success2xx),
            ResponseStatusKey::Explicit(404),
            ResponseStatusKey::Default,
        ]
    );
}

#[test]
fn enums_classify_strings_integers_mixed_and_nullable() {
    let dir = TempDir::new("enums");
    dir.write(
        "root.yaml",
        &doc_yaml(
            "3.1.0",
            r#"components:
  schemas:
    Color: {enum: [red, green]}
    Size: {enum: [1, 2, 3]}
    Junk: {enum: [on, 1]}
    MaybeColor: {enum: [red, null]}
"#,
        ),
    );
    let doc = load(dir.path(), "root.yaml").expect("loads");

    let SchemaKind::Enum { values } = &node(&doc, "Color").kind else {
        panic!("Color must be an enum");
    };
    assert_eq!(
        *values,
        EnumValues::Strings(vec!["red".into(), "green".into()])
    );

    let SchemaKind::Enum { values } = &node(&doc, "Size").kind else {
        panic!("Size must be an enum");
    };
    assert_eq!(*values, EnumValues::Integers(vec![1, 2, 3]));

    let SchemaKind::Enum { values } = &node(&doc, "Junk").kind else {
        panic!("Junk must be an enum");
    };
    assert!(matches!(values, EnumValues::MixedFallback(_)));

    let maybe = node(&doc, "MaybeColor");
    assert!(
        maybe.nullable,
        "null constant lifts the nullability dimension"
    );
    let SchemaKind::Enum { values } = &maybe.kind else {
        panic!("MaybeColor must remain an enum");
    };
    assert_eq!(*values, EnumValues::Strings(vec!["red".into()]));
}
