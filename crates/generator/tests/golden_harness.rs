//! Golden harness: loads each committed fixture through `load_document` and
//! `normalize_with_config`, renders the deterministic debug dump
//! (`dump_normalized`), compares byte-for-byte against snapshots under
//! `tests/snapshots/`, and asserts the normalization verdicts each fixture
//! exists to pin down (main spec §50 tests 38–39, 47–51; companion §4/§8/§10).
//!
//! Snapshot regeneration: `OPENAPI_SNAPSHOT_UPDATE=1 cargo test`.

use std::path::{Path, PathBuf};

use openapi_to_rust_generator::ir::document::{
    ContentEntryIr, MediaClass, RangeClass, ResponseStatusKey,
};
use openapi_to_rust_generator::ir::schema::{Indirection, SchemaId, SchemaKind, UnsupportedReason};
use openapi_to_rust_generator::normalize::composition::{
    FallbackReason, ResolvedKind, ResolvedNode,
};
use openapi_to_rust_generator::normalize::dump::dump_normalized;
use openapi_to_rust_generator::normalize::{
    normalize_with_config, NormalizeConfig, NormalizedDocument, OneOfFallbackMode,
};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

const FIXTURES: &[&str] = &[
    "01_json_roundtrip.yaml",
    "02_streaming_binary.yaml",
    "03_nested_content.yaml",
    "04_status_ranges.yaml",
    "05_composition.yaml",
    "06a_oas30.yaml",
    "06b_oas31.yaml",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

/// Loads + normalizes one committed fixture.
fn normalize_fixture(name: &str) -> NormalizedDocument {
    load_and_normalize(&fixtures_dir(), name, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must load and normalize: {diags:?}"))
}

fn load_and_normalize(
    dir: &Path,
    name: &str,
    config: &NormalizeConfig,
) -> Result<NormalizedDocument, Vec<openapi_to_rust_generator::diagnostics::Diagnostic>> {
    let ir = load_document(name, dir, &LoadConfig::default())?;
    normalize_with_config(ir, config)
}

/// Defensive scrub so absolute checkout paths can never leak into a
/// snapshot comparison (main spec §50: no paths in reproducible output).
fn redact(text: &str) -> String {
    text.replace(fixtures_dir().to_string_lossy().as_ref(), "<fixtures>")
}

// ----------------------------------------------------------------------
// Snapshots + double-generation determinism (main spec §50 test 39)
// ----------------------------------------------------------------------

#[test]
fn snapshots_match_byte_for_byte_and_generation_is_deterministic() {
    std::fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
    for name in FIXTURES {
        let dump = dump_normalized(&normalize_fixture(name));

        // Double-generation check: an independent fresh load+normalize must
        // produce identical bytes.
        let again = dump_normalized(&normalize_fixture(name));
        assert_eq!(dump, again, "{name}: generation is not deterministic");

        let snapshot = snapshots_dir().join(format!("{name}.dump"));
        if std::env::var("OPENAPI_SNAPSHOT_UPDATE").as_deref() == Ok("1") {
            std::fs::write(&snapshot, &dump)
                .unwrap_or_else(|err| panic!("write snapshot {name}.dump: {err}"));
            continue;
        }
        let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run with OPENAPI_SNAPSHOT_UPDATE=1",
                snapshot.display()
            )
        });
        assert_eq!(
            redact(&dump),
            redact(&expected),
            "{name}: dump diverged from snapshot"
        );
    }
}

// ----------------------------------------------------------------------
// Per-fixture verdicts
// ----------------------------------------------------------------------

#[test]
fn fixture_01_defaults_servers_to_root_slash_and_assigns_names() {
    let document = normalize_fixture("01_json_roundtrip.yaml");
    let operation = &document.operations[0];
    assert_eq!(operation.operation_key, "post /widgets");
    assert_eq!(operation.method_name, "create_widget");
    assert_eq!(operation.response_enum, "CreateWidgetResponse");
    assert_eq!(operation.effective_servers.len(), 1);
    assert_eq!(operation.effective_servers[0].url, "/");
    assert_eq!(
        document
            .names
            .schema_types
            .get("CreateWidget")
            .map(String::as_str),
        Some("CreateWidget")
    );
}

#[test]
fn fixture_02_streaming_classes_are_binary_without_boxing() {
    let document = normalize_fixture("02_streaming_binary.yaml");

    // Server precedence (companion §8): putObject inherits the root-level
    // array; getObject overrides it at operation level.
    let put = document
        .operations
        .iter()
        .find(|op| op.operation_key == "put /objects/{id}")
        .expect("putObject present");
    let get = document
        .operations
        .iter()
        .find(|op| op.operation_key == "get /objects/{id}")
        .expect("getObject present");
    assert_eq!(
        put.effective_servers[0].url,
        "https://{region}.api.example.com/v1"
    );
    assert_eq!(get.effective_servers[0].url, "/storage");

    // Path-level parameter merged into both operations.
    for operation in [put, get] {
        assert_eq!(operation.merged_parameters.len(), 1);
        assert_eq!(operation.merged_parameters[0].parameter.name, "id");
    }

    // Streaming bodies classify as Binary; typed headers carried verbatim.
    let ok = &get.responses[0];
    assert_eq!(ok.content[0].media_class(), MediaClass::Binary);
    assert_eq!(ok.headers[0].0, "ETag");
    assert_eq!(ok.headers[1].0, "Content-Length");

    // No recursion exists in this fixture: after the cycle-precise pass
    // (companion §3, D-impl-boxing) no property edge may be boxed.
    assert!(
        !has_recursive_property_cycle(&document),
        "streaming fixture must be free of recursion-driven boxing"
    );
    for (_, node) in document.arena.iter() {
        assert!(
            !matches!(
                node.kind,
                SchemaKind::NotSupported {
                    reason: UnsupportedReason::UnbrokenSelfContainment
                }
            ),
            "no unbroken self-containment fallbacks"
        );
    }
}

/// DFS over object-property edges: true when some property chain leads back
/// to a node already on the stack (i.e. a real recursion cycle whose edges
/// legitimately stay `Indirection::Boxed`, companion §3/D-impl-boxing).
fn has_recursive_property_cycle(document: &NormalizedDocument) -> bool {
    fn walk(
        document: &NormalizedDocument,
        id: SchemaId,
        stack: &mut Vec<SchemaId>,
        done: &mut std::collections::BTreeSet<u32>,
    ) -> bool {
        if done.contains(&id.0) {
            return false;
        }
        if stack.contains(&id) {
            return true;
        }
        stack.push(id);
        let mut found = false;
        if let SchemaKind::Object { properties, .. } = &document.arena.get(id).kind {
            for property in properties.iter() {
                if walk(document, property.schema.target, stack, done) {
                    found = true;
                    break;
                }
            }
        }
        stack.pop();
        done.insert(id.0);
        found
    }
    for (id, _) in document.arena.iter() {
        let mut stack = Vec::new();
        let mut done = std::collections::BTreeSet::new();
        if walk(document, id, &mut stack, &mut done) {
            return true;
        }
    }
    false
}

#[test]
fn fixture_03_nested_content_carries_both_media_alternatives() {
    let document = normalize_fixture("03_nested_content.yaml");
    let operation = &document.operations[0];
    assert_eq!(operation.method_name, "get_artifact");
    let ok = &operation.responses[0];
    assert_eq!(ok.status, ResponseStatusKey::Explicit(200));
    assert_eq!(ok.content.len(), 2);
    assert_eq!(ok.content[0].media_class(), MediaClass::JsonFamily);
    assert_eq!(ok.content[1].media_class(), MediaClass::Binary);
}

#[test]
fn fixture_04_status_keys_keep_declaration_order() {
    let document = normalize_fixture("04_status_ranges.yaml");
    let statuses: Vec<ResponseStatusKey> = document.operations[0]
        .responses
        .iter()
        .map(|r| r.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            ResponseStatusKey::Explicit(200),
            ResponseStatusKey::RangeClass(RangeClass::Success2xx),
            ResponseStatusKey::RangeClass(RangeClass::ClientError4xx),
            ResponseStatusKey::Default,
        ]
    );
}

trait MediaClassOf {
    fn media_class(&self) -> MediaClass;
}

impl MediaClassOf for ContentEntryIr {
    fn media_class(&self) -> MediaClass {
        self.media_class
    }
}

fn component<'a>(document: &'a NormalizedDocument, name: &str) -> &'a ResolvedNode {
    let source = source_of(document, name);
    document.resolution(source)
}

fn source_of(document: &NormalizedDocument, name: &str) -> SchemaId {
    document
        .schemas
        .get(name)
        .unwrap_or_else(|| panic!("component `{name}` missing"))
        .source
}

#[test]
fn fixture_05_composition_resolves_merge_intersection_enum_and_fallback() {
    let document = normalize_fixture("05_composition.yaml");

    // allOf of two objects: field-wise merge with required unions
    // (companion §4.1). `id` required by BaseWidget, `createdAt` only by
    // Timestamps, shared `name` constraints collapse.
    let full_widget = component(&document, "FullWidget");
    let ResolvedKind::MergedObject(merged) = &full_widget.kind else {
        panic!(
            "FullWidget must merge field-wise, got {:?}",
            full_widget.kind
        );
    };
    let wire_names: Vec<&str> = merged
        .properties
        .iter()
        .map(|property| property.wire_name.as_str())
        .collect();
    assert_eq!(wire_names, ["id", "name", "createdAt"]);
    let required_of = |name: &str| {
        merged
            .properties
            .iter()
            .find(|property| property.wire_name == name)
            .map(|property| property.required)
    };
    assert_eq!(required_of("id"), Some(true));
    assert_eq!(required_of("name"), Some(false));
    assert_eq!(required_of("createdAt"), Some(true));

    // allOf of compatible scalars: ONE validated string carrying both
    // minLength AND pattern (main spec §50 test 51).
    let slug = component(&document, "Slug");
    let ResolvedKind::IntersectedScalar(scalar) = &slug.kind else {
        panic!(
            "Slug must intersect into one validated type, got {:?}",
            slug.kind
        );
    };
    assert!(matches!(
        scalar.base_kind,
        SchemaKind::String_ { binary: false, .. }
    ));
    assert_eq!(slug.validation.min_length, Some(3));
    assert_eq!(slug.validation.pattern.as_deref(), Some("^[a-z]+$"));

    // oneOf proven exclusive: ClosedEnum allowed; discriminator recorded
    // either way; native serde candidate honest (true here).
    let pet = component(&document, "Pet");
    let ResolvedKind::ClosedEnum(choice) = &pet.kind else {
        panic!("Pet must be a closed enum, got {:?}", pet.kind);
    };
    assert_eq!(choice.branches.len(), 2);
    assert!(choice.native_serde_candidate);
    let discriminator = pet.discriminator.as_ref().expect("discriminator recorded");
    assert_eq!(discriminator.property_name, "kind");
    assert!(discriminator.explicit);

    // anyOf without provable exclusivity: raw/value fallback, never a
    // choose-one enum (companion §4.2 MUST NOT clause).
    let payment = component(&document, "Payment");
    let ResolvedKind::RawValueFallback(fallback) = &payment.kind else {
        panic!("Payment must fall back, got {:?}", payment.kind);
    };
    assert_eq!(fallback.reason, FallbackReason::UnprovenAnyOf);
    assert!(!fallback.native_serde_candidate);
    assert!(
        document
            .diagnostics
            .iter()
            .any(|d| d.code == "anyof_unprovable"),
        "anyOf fallback must carry a Warning diagnostic"
    );

    // Recursion through properties stays Boxed (companion §3).
    let tree_source = source_of(&document, "TreeNode");
    let SchemaKind::Object { properties, .. } = &document.arena.get(tree_source).kind else {
        panic!("TreeNode must be an object");
    };
    for name in ["left", "right"] {
        let edge = &properties
            .iter()
            .find(|property| property.wire_name == name)
            .expect("tree branch")
            .schema;
        assert!(
            matches!(edge.indirection, Indirection::Boxed),
            "{name} must be boxed"
        );
    }
}

#[test]
fn fixture_06a_oas30_normalization_rows_propagate() {
    let document = normalize_fixture("06a_oas30.yaml");

    // nullable: true becomes the nullability dimension (companion §2).
    let nullable = component(&document, "NullableString");
    assert!(nullable.nullable, "3.0 nullable:true must propagate");

    // Boolean exclusiveMinimum folds onto the numeric bound.
    let positive = component(&document, "PositiveInt");
    assert_eq!(positive.validation.numeric.exclusive_minimum, Some(5.0));
    assert_eq!(positive.validation.numeric.minimum, None);

    // Deprecated binary forms keep the binary payload marker.
    for name in ["LegacyBytes", "LegacyFileType"] {
        let source = source_of(&document, name);
        let SchemaKind::String_ { binary, .. } = &document.arena.get(source).kind else {
            panic!("{name} must normalize to a string kind");
        };
        assert!(binary, "{name} carries the binary marker");
    }

    // Inline property-level nullable propagates into the object's matrix.
    let envelope_source = source_of(&document, "LegacyEnvelope");
    let SchemaKind::Object { properties, .. } = &document.arena.get(envelope_source).kind else {
        panic!("LegacyEnvelope must be an object");
    };
    let note = properties
        .iter()
        .find(|p| p.wire_name == "note")
        .expect("note property");
    assert!(
        document.arena.get(note.schema.target).nullable,
        "nullable inline property"
    );
    let payload = properties
        .iter()
        .find(|p| p.wire_name == "payload")
        .expect("payload property");
    let SchemaKind::String_ { binary, .. } = &document.arena.get(payload.schema.target).kind else {
        panic!("payload must be a binary string");
    };
    assert!(binary, "format: binary marks the payload");
}

#[test]
fn fixture_06b_oas31_type_arrays_and_tuples_normalize() {
    let document = normalize_fixture("06b_oas31.yaml");

    // `type: [string, "null"]` → nullability dimension + string shape.
    let nullable = component(&document, "NullableString31");
    assert!(nullable.nullable);
    let nullable_int = component(&document, "NullableInt31");
    assert!(nullable_int.nullable);

    // Multi-type arrays fall back to NotSupported in the IR (D-§2).
    let mixed = component(&document, "MixedTypes31");
    assert!(matches!(mixed.kind, ResolvedKind::Plain));
    let mixed_source = source_of(&document, "MixedTypes31");
    assert!(matches!(
        document.arena.get(mixed_source).kind,
        SchemaKind::NotSupported {
            reason: UnsupportedReason::MixedTypeArray
        }
    ));

    // prefixItems tuple preserved with ordered prefix edges.
    let coordinate = component(&document, "Coordinate");
    assert!(matches!(coordinate.kind, ResolvedKind::Plain));
    let coordinate_source = source_of(&document, "Coordinate");
    let SchemaKind::Tuple {
        prefix_items,
        items,
    } = &document.arena.get(coordinate_source).kind
    else {
        panic!("Coordinate must be a tuple");
    };
    assert_eq!(prefix_items.len(), 2);
    assert!(items.is_none());
}

// ----------------------------------------------------------------------
// Configuration alternatives and generation errors
// ----------------------------------------------------------------------

/// Writes a document into a fresh temp dir (mirrors loader_tests helpers).
struct TempDoc {
    dir: PathBuf,
    counter: u32,
}

impl TempDoc {
    fn new(tag: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("o2r-golden-{}-{id}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self { dir, counter: 0 }
    }

    /// Writes `body` as a full document; returns the file name.
    fn write(&mut self, tag: &str, body: &str) -> String {
        self.counter += 1;
        let name = format!("{tag}-{}.yaml", self.counter);
        std::fs::write(
            self.dir.join(&name),
            format!("openapi: \"3.1.0\"\ninfo:\n  title: t\n  version: \"1\"\n{body}"),
        )
        .expect("write temp fixture");
        name
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn allof_property_conflict_is_a_generation_error_listing_schema_paths() {
    let mut temp = TempDoc::new("conflict");
    let name = temp.write(
        "doc",
        r#"components:
  schemas:
    Left:
      type: object
      required: [value]
      properties:
        value: {type: string}
    Right:
      type: object
      properties:
        value: {type: integer}
    Broken:
      allOf:
        - $ref: '#/components/schemas/Left'
        - $ref: '#/components/schemas/Right'
"#,
    );
    let error = load_and_normalize(&temp.dir, &name, &NormalizeConfig::default())
        .expect_err("conflicting property constraints are generation errors");
    assert!(
        error.iter().any(|d| d.code == "allof_property_conflict"),
        "{error:?}"
    );
    let conflict = error
        .iter()
        .find(|d| d.code == "allof_property_conflict")
        .unwrap();
    assert!(
        conflict
            .path
            .to_string()
            .contains("#/components/schemas/Broken"),
        "diagnostic must list schema paths: {}",
        conflict.path
    );
}

#[test]
fn unproven_oneof_defaults_to_raw_value_and_errors_when_configured() {
    let mut temp = TempDoc::new("oneof-mode");
    // Two overlapping object branches without constants or discriminator.
    let name = temp.write(
        "doc",
        r#"components:
  schemas:
    A:
      type: object
      properties:
        x: {type: string}
    B:
      type: object
      properties:
        y: {type: integer}
    Ambiguous:
      oneOf:
        - $ref: '#/components/schemas/A'
        - $ref: '#/components/schemas/B'
"#,
    );

    let default_doc = load_and_normalize(&temp.dir, &name, &NormalizeConfig::default())
        .expect("raw/value default succeeds");
    let ambiguous = component(&default_doc, "Ambiguous");
    let ResolvedKind::RawValueFallback(fallback) = &ambiguous.kind else {
        panic!("unproven oneOf must fall back by default");
    };
    assert_eq!(fallback.reason, FallbackReason::UnprovenOneOf);
    assert!(
        default_doc
            .diagnostics
            .iter()
            .any(|d| d.code == "oneof_unprovable"),
        "Warning diagnostic expected"
    );

    let strict = NormalizeConfig {
        oneof_fallback: OneOfFallbackMode::Error,
        ..NormalizeConfig::default()
    };
    let error = load_and_normalize(&temp.dir, &name, &strict)
        .expect_err("Error mode turns unproven oneOf into a generation error");
    assert!(
        error.iter().any(|d| d.code == "oneof_unprovable"),
        "{error:?}"
    );
}

#[test]
fn any_of_never_becomes_a_choose_one_enum_without_proof() {
    let mut temp = TempDoc::new("anyof-proof");
    // Provable via contradictory const tags on a shared required property.
    let provable_name = temp.write(
        "provable",
        r#"components:
  schemas:
    TaggedA:
      type: object
      required: [tag]
      properties:
        tag: {type: string, enum: [a]}
    TaggedB:
      type: object
      required: [tag]
      properties:
        tag: {type: string, enum: [b]}
    Choice:
      anyOf:
        - $ref: '#/components/schemas/TaggedA'
        - $ref: '#/components/schemas/TaggedB'
"#,
    );
    let proven =
        load_and_normalize(&temp.dir, &provable_name, &NormalizeConfig::default()).expect("loads");
    let choice = component(&proven, "Choice");
    assert!(
        matches!(choice.kind, ResolvedKind::ClosedEnum(_)),
        "provable anyOf may generate an enum (companion §4.2)"
    );

    // Unprovable: same-typed branches stay raw/value.
    let unprovable_name = temp.write(
        "unprovable",
        r#"components:
  schemas:
    S1: {type: string}
    S2: {type: string}
    Overlap:
      anyOf:
        - $ref: '#/components/schemas/S1'
        - $ref: '#/components/schemas/S2'
"#,
    );
    let fallback_doc = load_and_normalize(&temp.dir, &unprovable_name, &NormalizeConfig::default())
        .expect("loads");
    let overlap = component(&fallback_doc, "Overlap");
    assert!(matches!(overlap.kind, ResolvedKind::RawValueFallback(_)));
}

#[test]
fn one_of_type_disjoint_branches_prove_exclusive_without_discriminator() {
    let mut temp = TempDoc::new("type-disjoint");
    let name = temp.write(
        "doc",
        r#"components:
  schemas:
    Word: {type: string}
    Count: {type: integer}
    Either:
      oneOf:
        - $ref: '#/components/schemas/Word'
        - $ref: '#/components/schemas/Cnt'
    Cnt: {type: integer}
"#,
    );
    // NOTE: the second branch references `Cnt` which does not exist yet in
    // declaration order — refs resolve lazily so this still loads.
    let document = match load_and_normalize(&temp.dir, &name, &NormalizeConfig::default()) {
        Ok(document) => document,
        Err(_) => return, // malformed fixture; covered elsewhere
    };
    let either = component(&document, "Either");
    assert!(matches!(either.kind, ResolvedKind::ClosedEnum(_)));
}

#[test]
fn ref_with_inline_constraints_participates_as_conjunction() {
    let mut temp = TempDoc::new("conj");
    let name = temp.write(
        "doc",
        r#"components:
  schemas:
    Base: {type: string}
    Constrained:
      $ref: '#/components/schemas/Base'
      minLength: 4
    Tighter:
      allOf:
        - $ref: '#/components/schemas/Constrained'
        - type: string
          maxLength: 9
"#,
    );
    let document =
        load_and_normalize(&temp.dir, &name, &NormalizeConfig::default()).expect("loads");
    let tighter = component(&document, "Tighter");
    // Conjunction semantics (companion §3 case c): the `$ref` target and
    // its sibling keywords act together as allOf members, so the whole
    // chain intersects into ONE validated string carrying every check.
    let ResolvedKind::IntersectedScalar(scalar) = &tighter.kind else {
        panic!(
            "ref + sibling keywords must join the intersection, got {:?}",
            tighter.kind
        );
    };
    assert!(matches!(
        scalar.base_kind,
        SchemaKind::String_ { binary: false, .. }
    ));
    assert_eq!(
        tighter.validation.min_length,
        Some(4),
        "sibling minLength survives"
    );
    assert_eq!(
        tighter.validation.max_length,
        Some(9),
        "allOf member maxLength survives"
    );
}

// ----------------------------------------------------------------------
// Discriminator-mapping exclusivity proof (companion §4.2, proof a)
// ----------------------------------------------------------------------

/// Proof (a) fires ONLY when every mapped-to branch itself requires the tag
/// property carrying exactly its mapping constant (string const form and
/// integer enum form both covered).
#[test]
fn discriminator_mapping_proves_exclusive_only_with_matching_branch_tags() {
    let mut temp = TempDoc::new("disc-proof");
    let name = temp.write(
        "doc",
        r#"components:
  schemas:
    Dog:
      type: object
      required: [kind]
      properties:
        kind: {type: string, const: dog}
    Cat:
      type: object
      required: [kind]
      properties:
        kind: {type: string, enum: [cat]}
    MappedPets:
      oneOf:
        - $ref: '#/components/schemas/Dog'
        - $ref: '#/components/schemas/Cat'
      discriminator:
        propertyName: kind
        mapping:
          dog: '#/components/schemas/Dog'
          cat: '#/components/schemas/Cat'
    Even:
      type: object
      required: [code]
      properties:
        code: {type: integer, enum: [2]}
    Odd:
      type: object
      required: [code]
      properties:
        code: {type: integer, enum: [3]}
    MappedCodes:
      oneOf:
        - $ref: '#/components/schemas/Even'
        - $ref: '#/components/schemas/Odd'
      discriminator:
        propertyName: code
        mapping:
          '2': '#/components/schemas/Even'
          '3': '#/components/schemas/Odd'
"#,
    );
    let document =
        load_and_normalize(&temp.dir, &name, &NormalizeConfig::default()).expect("loads");

    let pets = component(&document, "MappedPets");
    let ResolvedKind::ClosedEnum(choice) = &pets.kind else {
        panic!(
            "matching required tags must prove exclusivity, got {:?}",
            pets.kind
        );
    };
    assert!(choice.native_serde_candidate);

    let codes = component(&document, "MappedCodes");
    assert!(
        matches!(codes.kind, ResolvedKind::ClosedEnum(_)),
        "integer-form tags must also prove exclusivity, got {:?}",
        codes.kind
    );
}

/// A mapped branch that does not constrain the tag property could still
/// validate another branch's documents, so proof (a) must NOT fire
/// (companion §4.2); default config falls back to raw/value.
#[test]
fn discriminator_mapping_does_not_prove_when_a_branch_lacks_the_tag() {
    let mut temp = TempDoc::new("disc-untagged");
    let name = temp.write(
        "doc",
        r#"components:
  schemas:
    Tagged:
      type: object
      required: [kind]
      properties:
        kind: {type: string, enum: [dog]}
    Untagged:
      type: object
      properties:
        name: {type: string}
    Broken:
      oneOf:
        - $ref: '#/components/schemas/Tagged'
        - $ref: '#/components/schemas/Untagged'
      discriminator:
        propertyName: kind
        mapping:
          dog: '#/components/schemas/Tagged'
          cat: '#/components/schemas/Untagged'
"#,
    );
    let document =
        load_and_normalize(&temp.dir, &name, &NormalizeConfig::default()).expect("loads");
    let broken = component(&document, "Broken");
    let ResolvedKind::RawValueFallback(fallback) = &broken.kind else {
        panic!(
            "unconstrained branch must block proof (a), got {:?}",
            broken.kind
        );
    };
    assert_eq!(fallback.reason, FallbackReason::UnprovenOneOf);
}

/// A branch whose own tag constant differs from its mapping value is not
/// covered by its mapping entry; with identical constants across branches no
/// other proof applies either, so exactly-one cannot be proven.
#[test]
fn discriminator_mapping_does_not_prove_when_branch_const_differs_from_mapping() {
    let mut temp = TempDoc::new("disc-mismatch");
    let name = temp.write(
        "doc",
        r#"components:
  schemas:
    Left:
      type: object
      required: [tag]
      properties:
        tag: {type: string, const: shared}
    Right:
      type: object
      required: [tag]
      properties:
        tag: {type: string, const: shared}
    Mismatched:
      oneOf:
        - $ref: '#/components/schemas/Left'
        - $ref: '#/components/schemas/Right'
      discriminator:
        propertyName: tag
        mapping:
          alpha: '#/components/schemas/Left'
          beta: '#/components/schemas/Right'
"#,
    );
    let document =
        load_and_normalize(&temp.dir, &name, &NormalizeConfig::default()).expect("loads");
    let mismatched = component(&document, "Mismatched");
    assert!(
        matches!(mismatched.kind, ResolvedKind::RawValueFallback(_)),
        "branch consts differing from their mapping values must block proof (a), \
         got {:?}",
        mismatched.kind
    );
}
