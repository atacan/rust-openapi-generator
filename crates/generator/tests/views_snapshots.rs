//! Directional-view emission harness (companion §5): loads every committed
//! fixture through `load_document` → `normalize_with_config` →
//! `generate_views`, compares the rendered `views.rs` byte-for-byte against
//! snapshots under `tests/snapshots/`, asserts rustfmt-cleanliness and
//! double-generation determinism (main spec §50 tests 39–40), and pins the
//! §5 directional field rules plus the asymmetric conversion policy for the
//! fixture 08 cases.
//!
//! Snapshot regeneration: `VIEWS_SNAPSHOT_UPDATE=1 cargo test`.

use std::path::PathBuf;

use openapi_to_rust_generator::codegen::views::generate_views;
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
    format!("{stem}.views.rs")
}

/// Loads + normalizes one committed fixture.
fn normalize_fixture(name: &str) -> openapi_to_rust_generator::normalize::NormalizedDocument {
    let ir = load_document(name, &fixtures_dir(), &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must load: {diags:?}"));
    normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must normalize: {diags:?}"))
}

fn generate_fixture(name: &str) -> String {
    generate_views(&normalize_fixture(name))
}

// ----------------------------------------------------------------------
// Snapshots + double-generation determinism (main spec §50 test 39)
// ----------------------------------------------------------------------

#[test]
fn view_snapshots_match_byte_for_byte_and_generation_is_deterministic() {
    std::fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
    for fixture in fixture_names() {
        let generated = generate_fixture(&fixture);

        // Double-generation check: an independent fresh load+normalize+generate
        // must produce identical bytes.
        let again = generate_fixture(&fixture);
        assert_eq!(
            generated, again,
            "{fixture}: view generation is not deterministic"
        );

        let snapshot = snapshots_dir().join(snapshot_name(&fixture));
        if std::env::var("VIEWS_SNAPSHOT_UPDATE").as_deref() == Ok("1") {
            std::fs::write(&snapshot, &generated)
                .unwrap_or_else(|err| panic!("write snapshot {}: {err}", snapshot.display()));
            continue;
        }
        let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run with VIEWS_SNAPSHOT_UPDATE=1",
                snapshot.display()
            )
        });
        assert_eq!(
            generated, expected,
            "{fixture}: generated views diverged from snapshot"
        );
    }
}

// ----------------------------------------------------------------------
// rustfmt-clean emission (main spec §50 test 40)
// ----------------------------------------------------------------------

#[test]
fn generated_views_are_rustfmt_clean() {
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
            "o2r-views-fmt-{}-{id}-{}",
            std::process::id(),
            fixture.trim_end_matches(".yaml")
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let source = dir.join(snapshot_name(&fixture));
        std::fs::write(&source, &generated).expect("write generated views");

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
// Fixture 08 — companion §5 directional field rules + conversion asymmetry
// ----------------------------------------------------------------------

/// (a) Account: required writeOnly `password` is required only in the write
/// direction; required directionless `id` is required in both; optional
/// directionless `note` keeps the OptionalField cell in both.
#[test]
fn fixture_08_account_write_view_holds_password_read_view_drops_it() {
    let output = generate_fixture("08_views.yaml");

    let account_write = struct_block(&output, "AccountWrite");
    assert!(
        account_write.contains("pub password: String,"),
        "required writeOnly must be required in the write view:\n{account_write}"
    );
    assert!(
        account_write.contains("pub id: String,"),
        "directionless required survives both views:\n{account_write}"
    );
    assert!(
        account_write.contains("pub note: OptionalField<String>,"),
        "\n{account_write}"
    );

    let account_read = struct_block(&output, "AccountRead");
    assert!(
        !account_read.contains("pub password"),
        "a read view cannot fabricate the password (§5 Widget rule):\n{account_read}"
    );
    assert!(account_read.contains("pub id: String,"), "\n{account_read}");
    assert!(
        account_read.contains("pub note: OptionalField<String>,"),
        "\n{account_read}"
    );
}

/// (b) AuditEntry: required readOnly `createdAt` is required only in the read
/// direction (with its serde rename); optional nullable `metadata` rides the
/// §2.1 row-4 cell; the write view drops createdAt entirely.
#[test]
fn fixture_08_audit_entry_directional_cells_and_renames() {
    let output = generate_fixture("08_views.yaml");

    let audit_write = struct_block(&output, "AuditEntryWrite");
    assert!(
        !audit_write.contains("pub created_at") && !audit_write.contains("createdAt"),
        "readOnly must be omitted from the write view:\n{audit_write}"
    );
    assert!(
        audit_write.contains("pub draft_note: OptionalField<String>,"),
        "optional writeOnly lives in the write view:\n{audit_write}"
    );
    assert!(
        audit_write.contains("#[serde(default)]")
            && audit_write.contains("pub metadata: Option<String>,"),
        "optional nullable keeps the row-4 cell:\n{audit_write}"
    );

    let audit_read = struct_block(&output, "AuditEntryRead");
    assert!(
        !audit_read.contains("pub draft_note") && !audit_read.contains("draftNote"),
        "writeOnly is treated as absent on the response wire:\n{audit_read}"
    );
    assert!(
        audit_read.contains("#[serde(rename = \"createdAt\")]"),
        "\n{audit_read}"
    );
    assert!(
        audit_read.contains("pub created_at: String,"),
        "required readOnly is required in the read view:\n{audit_read}"
    );
}

/// Conversion asymmetry (companion §5): projections always exist;
/// reconstructions exist exactly when lossless — never for a view that
/// dropped a required non-nullable shared field.
#[test]
fn fixture_08_conversions_generated_exactly_when_lossless() {
    let output = generate_fixture("08_views.yaml");

    // Projections From<&Shared> for *View: always present.
    assert!(
        output.contains("impl From<&Account> for AccountWrite {"),
        "\n{output}"
    );
    assert!(output.contains("impl From<&Account> for AccountRead {"));
    assert!(output.contains("impl From<&AuditEntry> for AuditEntryWrite {"));
    assert!(output.contains("impl From<&AuditEntry> for AuditEntryRead {"));
    assert!(output.contains("impl From<&SyncedRecord> for SyncedRecordWrite {"));
    assert!(output.contains("impl From<&SyncedRecord> for SyncedRecordRead {"));

    // Reconstructions From<&*View> for Shared: iff every dropped field is
    // optional in the shared model.
    assert!(
        output.contains("impl From<&AccountWrite> for Account {"),
        "AccountWrite drops nothing, so it reconstructs losslessly:\n{output}"
    );
    assert!(
        !output.contains("impl From<&AccountRead> for Account {"),
        "the password cannot be fabricated"
    );
    assert!(
        !output.contains("impl From<&AuditEntryWrite> for AuditEntry {"),
        "createdAt is required non-nullable in the shared model"
    );
    assert!(
        output.contains("impl From<&AuditEntryRead> for AuditEntry {"),
        "draftNote is optional, so the read reconstruction is lossless"
    );
    assert!(
        !output.contains("impl From<&SyncedRecordWrite> for SyncedRecord {"),
        "required readOnly id would be lost by the write reconstruction"
    );
    assert!(
        output.contains("impl From<&SyncedRecordRead> for SyncedRecord {"),
        "only the optional secretToken is missing from the read view"
    );

    // Phase 1 policy: fallible TryFrom completion steps are never generated —
    // a lossy direction simply gets NO conversion (values are never invented).
    assert!(!output.contains("TryFrom"), "\n{output}");
}

/// (c) SyncedRecord mixes readOnly AND writeOnly: each view keeps one marker
/// class plus every directionless field.
#[test]
fn fixture_08_synced_record_mixes_both_markers() {
    let output = generate_fixture("08_views.yaml");

    let synced_write = struct_block(&output, "SyncedRecordWrite");
    assert!(
        !synced_write.contains("pub id:"),
        "readOnly dropped:\n{synced_write}"
    );
    assert!(
        !synced_write.contains("pub reviewed_by"),
        "readOnly dropped even when optional:\n{synced_write}"
    );
    assert!(
        synced_write.contains("pub label: String,"),
        "\n{synced_write}"
    );
    assert!(
        synced_write.contains("pub secret_token: OptionalField<String>,"),
        "\n{synced_write}"
    );

    let synced_read = struct_block(&output, "SyncedRecordRead");
    assert!(
        synced_read.contains("pub id: String,"),
        "required readOnly is required here:\n{synced_read}"
    );
    assert!(
        synced_read.contains("pub reviewed_by: OptionalField<String>,"),
        "\n{synced_read}"
    );
    assert!(
        !synced_read.contains("pub secret_token"),
        "writeOnly dropped:\n{synced_read}"
    );
    assert!(
        synced_read.contains("pub label: String,"),
        "\n{synced_read}"
    );
}

/// (d) PlainNote carries no directional markers: no view types at all
/// (identity case).
#[test]
fn fixture_08_plain_model_gets_no_views() {
    let output = generate_fixture("08_views.yaml");

    // The plain model itself only appears as an imported shared type if some
    // view references it — nothing does, so it is fully absent.
    assert!(!output.contains("PlainNote"), "\n{output}");
    // Marker-free fixtures emit header-only modules.
    let empty = generate_fixture("01_json_roundtrip.yaml");
    assert!(!empty.contains("pub struct"), "\n{empty}");
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

/// Companion §9 continuity (main spec §50 test 50): Write views emit
/// `validate_request()` keyed by their SURVIVING field list — SyncedRecord's
/// `label` constraint rides into the write view; constraints on dropped
/// readOnly fields would not. Read views never carry validators.
#[test]
fn fixture_08_write_views_validate_kept_fields_only() {
    let output = generate_fixture("08_views.yaml");

    let validator = item_block(&output, "impl SyncedRecordWrite {");
    assert!(
        validator.contains("pub fn validate_request(")
            && validator.contains("validate_string")
            && validator.contains("min_length: Some(3)")
            && validator.contains("at_field(\"label\")"),
        "the kept constrained field must be checked:\n{validator}"
    );
    assert!(
        !output.contains("impl AccountWrite {"),
        "no surviving constraint → no validator:\n{output}"
    );
    assert!(!output.contains("impl AccountRead {"), "\n{output}");
    assert!(!output.contains("impl AuditEntryRead {"), "\n{output}");
}
