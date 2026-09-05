//! Model-emission harness: loads every committed fixture through
//! `load_document` → `normalize_with_config` → `generate_models`, compares
//! the rendered `models.rs` byte-for-byte against snapshots under
//! `tests/snapshots/`, asserts rustfmt-cleanliness (main spec §50 test 40,
//! DECISIONS.md D-impl-codegen-emission) and double-generation determinism
//! (main spec §50 test 39), and pins the companion §2.1 matrix verdicts each
//! fixture exists to cover.
//!
//! Snapshot regeneration: `MODELS_SNAPSHOT_UPDATE=1 cargo test`.

use std::path::PathBuf;

use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::normalize::{normalize_with_config, NormalizeConfig};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

/// Every fixture in declaration order (file name sorted).
fn fixture_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures directory exists")
        .filter_map(|entry| {
            let path = entry.expect("fixture entry").path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            name.ends_with(".yaml").then_some(name)
        })
        .collect();
    names.sort();
    names
}

fn snapshot_name(fixture: &str) -> String {
    let stem = fixture.strip_suffix(".yaml").unwrap_or(fixture);
    format!("{stem}.models.rs")
}

/// Loads + normalizes one committed fixture.
fn normalize_fixture(name: &str) -> openapi_to_rust_generator::normalize::NormalizedDocument {
    let ir = load_document(name, &fixtures_dir(), &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must load: {diags:?}"));
    normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must normalize: {diags:?}"))
}

fn generate_fixture(name: &str) -> String {
    generate_models(&normalize_fixture(name))
}

// ----------------------------------------------------------------------
// Snapshots + double-generation determinism (main spec §50 test 39)
// ----------------------------------------------------------------------

#[test]
fn model_snapshots_match_byte_for_byte_and_generation_is_deterministic() {
    std::fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
    for fixture in fixture_names() {
        let generated = generate_fixture(&fixture);

        // Double-generation check: an independent fresh load+normalize+generate
        // must produce identical bytes.
        let again = generate_fixture(&fixture);
        assert_eq!(
            generated, again,
            "{fixture}: generation is not deterministic"
        );

        let snapshot = snapshots_dir().join(snapshot_name(&fixture));
        if std::env::var("MODELS_SNAPSHOT_UPDATE").as_deref() == Ok("1") {
            std::fs::write(&snapshot, &generated)
                .unwrap_or_else(|err| panic!("write snapshot {}: {err}", snapshot.display()));
            continue;
        }
        let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run with MODELS_SNAPSHOT_UPDATE=1",
                snapshot.display()
            )
        });
        assert_eq!(
            generated, expected,
            "{fixture}: generated models diverged from snapshot"
        );
    }
}

// ----------------------------------------------------------------------
// rustfmt-clean emission (main spec §50 test 40)
// ----------------------------------------------------------------------

#[test]
fn generated_models_are_rustfmt_clean() {
    let Some(rustfmt) = locate_rustfmt() else {
        eprintln!(
            "WARNING: no rustfmt binary on PATH; skipping the rustfmt-clean assertion \
             (main spec §50 test 40)"
        );
        return;
    };
    for fixture in fixture_names() {
        let generated = generate_fixture(&fixture);

        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "o2r-models-fmt-{}-{id}-{}",
            std::process::id(),
            fixture.trim_end_matches(".yaml")
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let source = dir.join(snapshot_name(&fixture));
        std::fs::write(&source, &generated).expect("write generated models");

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

#[test]
fn fixture_18_schema_free_models_are_a_valid_documented_module() {
    let output = generate_fixture("18_no_schemas.yaml");

    assert!(output.starts_with("//! Shared schema models generated"));
    assert!(
        output
            .lines()
            .all(|line| line.is_empty() || line.starts_with("//!")),
        "schema-free models must contain module docs only:\n{output}"
    );

    let dir = std::env::temp_dir().join(format!("o2r-empty-models-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("models.rs");
    std::fs::write(&source, output).expect("write generated models");
    let compiled = std::process::Command::new("rustc")
        .args(["--edition", "2021", "--crate-type", "lib"])
        .arg(&source)
        .arg("--out-dir")
        .arg(&dir)
        .output()
        .expect("spawn rustc");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        compiled.status.success(),
        "schema-free generated models must compile:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
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
// Fixture 01 — basic cells and required fields
// ----------------------------------------------------------------------

#[test]
fn fixture_01_basic_structs_cells_and_required_fields() {
    let output = generate_fixture("01_json_roundtrip.yaml");

    // CreateWidget: required `name`, optional non-nullable `description`
    // through the §2.1 optional cell wrapper.
    let create_widget = struct_block(&output, "CreateWidget");
    assert!(
        create_widget.contains("pub name: String,"),
        "required non-null cell must be plain T:\n{create_widget}"
    );
    assert!(
        create_widget.contains(
            "#[serde(default, skip_serializing_if = \"openapi_support::optional::is_absent\")]"
        ),
        "optional non-null cell carries default + skip_serializing_if:\n{create_widget}"
    );
    assert!(
        create_widget.contains("pub description: OptionalField<String>,"),
        "optional non-null cell must be OptionalField<T>:\n{create_widget}"
    );

    // Widget keeps `id` required.
    let widget = struct_block(&output, "Widget");
    assert!(widget.contains("pub id: String,"), "\n{widget}");
    assert!(widget.contains("pub name: String,"), "\n{widget}");

    assert!(struct_block(&output, "ProblemDetails").contains("pub title: String,"));

    // Imports appear exactly when needed.
    assert!(output.contains("use serde::{Deserialize, Serialize};"));
    assert!(output.contains("use openapi_support::optional::OptionalField;"));
    assert!(!output.contains("use std::collections::BTreeMap;"));
}

// ----------------------------------------------------------------------
// Fixture 05 — composition outputs
// ----------------------------------------------------------------------

#[test]
fn fixture_05_composition_outputs_merge_intersect_choice_fallback_boxing() {
    let output = generate_fixture("05_composition.yaml");

    // Merged allOf struct contains the unioned fields; required unions hold
    // (companion §4.1). Wire names ride on serde renames.
    let full_widget = struct_block(&output, "FullWidget");
    assert!(full_widget.contains("pub id: String,"), "\n{full_widget}");
    assert!(full_widget.contains("pub name:"), "\n{full_widget}");
    assert!(
        full_widget.contains("#[serde(rename = \"createdAt\")]"),
        "\n{full_widget}"
    );
    assert!(
        full_widget.contains("pub created_at: String,"),
        "\n{full_widget}"
    );

    // Intersected scalar alias carries BOTH minLength AND pattern docs
    // (main spec §50 test 51; D-impl-runtime-validation-timing).
    let slug = item_block(&output, "pub type Slug");
    assert!(slug.contains("minLength >= 3"), "\n{slug}");
    assert!(slug.contains("pattern `^[a-z]+$`"), "\n{slug}");

    // Proven oneOf → closed choice enum with discriminator documentation.
    let pet = item_block(&output, "pub enum Pet");
    assert!(pet.contains("#[serde(untagged)]"), "\n{pet}");
    assert!(pet.contains("Dog(Dog),"), "\n{pet}");
    assert!(pet.contains("Cat(Cat),"), "\n{pet}");
    assert!(pet.contains("property `kind`"), "\n{pet}");
    assert!(pet.contains("dog -> Dog"), "\n{pet}");

    // Unproven anyOf → Fallback newtype over raw JSON, never an enum.
    assert!(
        item_block(&output, "PaymentFallback").contains("(pub serde_json::Value);"),
        "unproven anyOf must fall back to the Fallback newtype"
    );

    // Recursion through properties stays heap-indirected (companion §3).
    let tree_node = struct_block(&output, "TreeNode");
    assert_eq!(
        tree_node.matches("OptionalField<Box<TreeNode>>").count(),
        2,
        "left/right must be boxed:\n{tree_node}"
    );
}

// ----------------------------------------------------------------------
// Fixtures 06a/06b — version normalization rows surface in models
// ----------------------------------------------------------------------

#[test]
fn fixture_06a_oas30_nullable_propagation_and_scalar_mappings() {
    let output = generate_fixture("06a_oas30.yaml");

    // Optional + nullable property: plain Option with #[serde(default)].
    let envelope = struct_block(&output, "LegacyEnvelope");
    assert!(envelope.contains("#[serde(default)]"), "\n{envelope}");
    assert!(
        envelope.contains("pub note: Option<String>,"),
        "\n{envelope}"
    );

    // Absent integer format maps to i64 (count is optional + non-nullable).
    assert!(
        envelope.contains("pub count: OptionalField<i64>,"),
        "\n{envelope}"
    );
    assert!(envelope.contains("exclusiveMinimum > 1"), "\n{envelope}");
    assert!(item_block(&output, "pub type NullableString").contains("= String;"));

    // Binary strings stay `String` with the Phase 1 warning doc.
    let bytes = item_block(&output, "pub type LegacyBytes");
    assert!(bytes.contains("= String;"), "\n{bytes}");
    assert!(bytes.contains("format: binary"), "\n{bytes}");
}

#[test]
fn fixture_06b_oas31_tuple_emitted_for_prefix_items() {
    let output = generate_fixture("06b_oas31.yaml");

    let coordinate = item_block(&output, "pub type Coordinate");
    assert!(coordinate.contains("= (String, i64);"), "\n{coordinate}");

    let envelope = struct_block(&output, "ModernEnvelope");
    assert!(
        envelope.contains("pub label: Option<NullableString31>,"),
        "nullable propagation across refs:\n{envelope}"
    );
}

// ----------------------------------------------------------------------
// Fixture 07 — all four §2.1 cells, enum flavors, deny + flatten
// ----------------------------------------------------------------------

#[test]
fn fixture_07_matrix_cells_enums_and_additional_properties_policies() {
    let output = generate_fixture("07_matrix.yaml");

    let record = struct_block(&output, "MatrixRecord");
    // Cell 1: required + non-nullable.
    assert!(record.contains("pub req_plain: String,"), "\n{record}");
    // Cell 2: required + nullable — presence-aware adapter, NO default attr.
    // (The attribute wraps onto continuation lines when over-wide; compare
    // whitespace-insensitively.)
    let condensed: String = record.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        condensed.contains(
            "#[serde(deserialize_with=\"openapi_support::optional::presence::deserialize_required_nullable\")]"
        ),
        "\n{record}"
    );
    assert!(
        !record.contains("deserialize_required_nullable\")\n    pub req_nullable"),
        "cell 2 must not gain a default attr:\n{record}"
    );
    assert!(
        record.contains("pub req_nullable: Option<String>,"),
        "\n{record}"
    );
    // Cell 3: optional + non-nullable.
    assert!(
        record.contains("pub opt_plain: OptionalField<i32>,"),
        "int32 maps to i32:\n{record}"
    );
    // Cell 4: optional + nullable.
    assert!(
        record.contains("pub opt_nullable: Option<i64>,"),
        "\n{record}"
    );

    // String enum with renames whenever variant != constant.
    let status = item_block(&output, "pub enum StringStatus");
    assert!(
        status.contains("#[serde(rename = \"draft\")]"),
        "\n{status}"
    );
    assert!(status.contains("Draft,"), "\n{status}");
    assert!(
        status.contains("#[serde(rename = \"in_review\")]"),
        "\n{status}"
    );
    assert!(status.contains("InReview,"), "\n{status}");

    // Integer enum uses manual bare-number codecs.
    let code = item_block(&output, "impl serde::Serialize for IntCode");
    assert!(code.contains("serialize_i64(*self as i64)"), "\n{code}");
    let decode = item_block(&output, "impl<'de> serde::Deserialize<'de> for IntCode");
    assert!(decode.contains("4 => Ok(Self::V4),"), "\n{decode}");
    assert!(
        decode
            .contains("unknown discriminant {other} for enum `IntCode`, expected one of [1, 2, 4]"),
        "\n{decode}"
    );

    // Mixed enum: typed scalar variants plus the Other catch-all, untagged.
    let mixed = item_block(&output, "pub enum MixedScalar");
    assert!(mixed.contains("#[serde(untagged)]"), "\n{mixed}");
    assert!(mixed.contains("Text(String),"), "\n{mixed}");
    assert!(mixed.contains("V7(i64),"), "\n{mixed}");
    assert!(mixed.contains("True(bool),"), "\n{mixed}");
    assert!(mixed.contains("Other(serde_json::Value),"), "\n{mixed}");

    // additionalProperties: false → deny_unknown_fields.
    let strict = struct_block(&output, "StrictRecord");
    assert!(
        strict.contains("#[serde(deny_unknown_fields)]"),
        "\n{strict}"
    );
    // Schema-valued additionalProperties → flattened BTreeMap.
    let bag = struct_block(&output, "TaggedBag");
    assert!(bag.contains("#[serde(flatten)]"), "\n{bag}");
    assert!(
        bag.contains("pub additional: BTreeMap<String, String>,"),
        "\n{bag}"
    );
    assert!(output.contains("use std::collections::BTreeMap;"));
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

/// The full text of one emitted struct definition, anchored on its
/// `pub struct Name {` header so attributes above it are included.
fn struct_block(output: &str, name: &str) -> String {
    item_block(output, &format!("pub struct {name} {{"))
}
