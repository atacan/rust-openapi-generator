//! Emitted-manifest harness (main spec §3/§3.1, §50 tests 39;
//! DECISIONS.md D-impl-crate, D-impl-codec-plugins).
//!
//! [`generate_manifest`] is exercised over a fixture × configuration matrix:
//! fixture 01 (JSON-only) under default/client-only/server-only feature
//! selections, and fixture 16 under the default plus every codec-enabled
//! configuration. Assertions:
//!
//! 1. byte determinism across independent generations and byte-for-byte
//!    agreement with committed `<fixture>.<config-key>.Cargo.toml` snapshots
//!    (TOML has no rustfmt canon; ordering stability is guaranteed by
//!    construction through `BTreeMap` emission and asserted by test);
//! 2. §3.1 structural invariants: no `path =`, no wildcard requirements, no
//!    pre-release tags inside dependency values, pinned `rust-version`;
//! 3. codec dependency lines appear IFF the codec is enabled (checked in
//!    both directions), and the `[features]` section mirrors
//!    openapi-support's feature graph with §3 defaults.
//!
//! Snapshot regeneration: `MANIFEST_SNAPSHOT_UPDATE=1 cargo test`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use openapi_to_rust_generator::codegen::manifest::{
    generate_manifest, package_name, EmbeddedToolchain, FeatureSelection, ManifestConfig,
    ManifestOverrides,
};
use openapi_to_rust_generator::codegen::plan::{
    plan_api_with_config, GeneratorPlanOptions, PlanConfig,
};
use openapi_to_rust_generator::normalize::{normalize_with_config, NormalizeConfig};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

const JSON_FIXTURE: &str = "01_json_roundtrip.yaml";
const CODEC_FIXTURE: &str = "16_codecs.yaml";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn normalize_fixture(name: &str) -> openapi_to_rust_generator::normalize::NormalizedDocument {
    let ir = load_document(name, &fixtures_dir(), &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must load: {diags:?}"));
    normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{name} must normalize: {diags:?}"))
}

/// One matrix row: `(fixture stem, config tag, manifest config builder)`.
struct MatrixRow {
    fixture_stem: &'static str,
    fixture: &'static str,
    tag: &'static str,
    features: FeatureSelection,
    codecs: &'static [&'static str],
}

const MATRIX: &[MatrixRow] = &[
    MatrixRow {
        fixture_stem: "01_json_roundtrip",
        fixture: JSON_FIXTURE,
        tag: "default",
        features: FeatureSelection {
            client: true,
            server: true,
        },
        codecs: &[],
    },
    MatrixRow {
        fixture_stem: "01_json_roundtrip",
        fixture: JSON_FIXTURE,
        tag: "client",
        features: FeatureSelection {
            client: true,
            server: false,
        },
        codecs: &[],
    },
    MatrixRow {
        fixture_stem: "01_json_roundtrip",
        fixture: JSON_FIXTURE,
        tag: "server",
        features: FeatureSelection {
            client: false,
            server: true,
        },
        codecs: &[],
    },
    MatrixRow {
        fixture_stem: "16_codecs",
        fixture: CODEC_FIXTURE,
        tag: "default",
        features: FeatureSelection {
            client: true,
            server: true,
        },
        codecs: &[],
    },
    MatrixRow {
        fixture_stem: "16_codecs",
        fixture: CODEC_FIXTURE,
        tag: "codec-xml",
        features: FeatureSelection::BOTH,
        codecs: &["xml"],
    },
    MatrixRow {
        fixture_stem: "16_codecs",
        fixture: CODEC_FIXTURE,
        tag: "codec-cbor",
        features: FeatureSelection::BOTH,
        codecs: &["cbor"],
    },
    MatrixRow {
        fixture_stem: "16_codecs",
        fixture: CODEC_FIXTURE,
        tag: "codec-msgpack",
        features: FeatureSelection::BOTH,
        codecs: &["msgpack"],
    },
    MatrixRow {
        fixture_stem: "16_codecs",
        fixture: CODEC_FIXTURE,
        tag: "codecs-all",
        features: FeatureSelection::BOTH,
        codecs: &["msgpack", "xml", "cbor"],
    },
];

fn config_for(row: &MatrixRow) -> ManifestConfig {
    ManifestConfig {
        toolchain: EmbeddedToolchain::CURRENT,
        features: row.features,
        enabled_codecs: row.codecs.iter().copied().collect::<BTreeSet<_>>(),
        overrides: None,
    }
}

/// Plans one fixture with matching codec options so plan claims and manifest
/// configuration agree (the generator's real pipeline shape).
fn generate_for(row: &MatrixRow) -> String {
    let doc = normalize_fixture(row.fixture);
    let plan_config = PlanConfig {
        generator_options: GeneratorPlanOptions {
            enabled_codecs: row.codecs.iter().copied().collect(),
            overrides: Vec::new(),
        },
        ..PlanConfig::default()
    };
    let plan = plan_api_with_config(&doc, &plan_config)
        .unwrap_or_else(|diags| panic!("{} must plan: {diags:?}", row.fixture));
    let cfg = config_for(row);
    generate_manifest(&doc, &plan, &cfg)
        .unwrap_or_else(|diags| panic!("{} must emit manifest: {diags:?}", row.fixture))
}

fn snapshot_path(fixture_stem: &str, tag: &str) -> PathBuf {
    snapshots_dir().join(format!("{fixture_stem}.{tag}.Cargo.toml"))
}

// ----------------------------------------------------------------------
// Snapshots + double-run determinism (main spec §50 test 39)
// ----------------------------------------------------------------------

#[test]
fn manifests_match_snapshots_and_double_run_is_byte_stable() {
    std::fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
    for row in MATRIX {
        let first = generate_for(row);
        // Determinism: an independent second generation must be identical.
        assert_eq!(
            first,
            generate_for(row),
            "{}[{}]: manifest not deterministic",
            row.fixture_stem,
            row.tag
        );

        let snapshot = snapshot_path(row.fixture_stem, row.tag);
        if std::env::var("MANIFEST_SNAPSHOT_UPDATE").as_deref() == Ok("1") {
            std::fs::write(&snapshot, &first)
                .unwrap_or_else(|err| panic!("write {}: {err}", snapshot.display()));
            continue;
        }
        let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {}; run with MANIFEST_SNAPSHOT_UPDATE=1",
                snapshot.display()
            )
        });
        assert_eq!(
            first, expected,
            "{}[{}]: emitted manifest diverged from snapshot",
            row.fixture_stem, row.tag
        );
    }
}

// ----------------------------------------------------------------------
// §3.1 structural invariants
// ----------------------------------------------------------------------

#[test]
fn structural_invariants_hold_for_every_configuration() {
    for row in MATRIX {
        let manifest = generate_for(row);
        let msrv = EmbeddedToolchain::CURRENT.msrv;

        // Never a path dependency (§3.1/D-impl-crate).
        assert!(
            !manifest.contains("path ="),
            "{}[{}]: path dependency leaked:\n{manifest}",
            row.fixture_stem,
            row.tag
        );
        // No wildcards anywhere (requirements are the only place `*` could
        // appear; banners and keys never contain it by construction).
        assert!(
            !manifest.contains('*'),
            "{}[{}]: wildcard requirement leaked:\n{manifest}",
            row.fixture_stem,
            row.tag
        );
        // Pinned MSRV present.
        assert!(
            manifest.contains(&format!("rust-version = \"{msrv}\"")),
            "{}[{}]: pinned rust-version missing:\n{manifest}",
            row.fixture_stem,
            row.tag
        );
        // Package naming: deterministic `-api` suffix from info.title.
        let name_line = manifest
            .lines()
            .find(|line| line.starts_with("name = "))
            .expect("package name line");
        assert!(
            name_line.ends_with("-api\""),
            "{}[{}]: package name lacks the `-api` suffix: {name_line}",
            row.fixture_stem,
            row.tag
        );
        // Dependency VERSION values below [dependencies] never carry
        // pre-release tags or comparison operators (feature strings such as
        // `io-util` legitimately contain `-`; versions never do).
        let mut in_deps = false;
        for line in manifest.lines() {
            match line.strip_prefix('[') {
                Some(_) => in_deps = line == "[dependencies]",
                None if in_deps => {
                    let mut rest = line;
                    while let Some(position) = rest.find("version = \"") {
                        rest = &rest[position + "version = \"".len()..];
                        let Some(end) = rest.find('"') else {
                            panic!("unterminated version: {line}");
                        };
                        let requirement = &rest[..end];
                        assert!(
                            !requirement.contains(['-', '<', '>', '~', '*']),
                            "{}[{}]: suspicious version requirement \
                             `{requirement}`:\n{manifest}",
                            row.fixture_stem,
                            row.tag
                        );
                        rest = &rest[end..];
                    }
                }
                None => {}
            }
        }
    }
}

#[test]
fn codec_dependency_lines_appear_iff_enabled() {
    const FRAGMENTS: &[(&str, &str)] = &[
        ("xml", "quick-xml"),
        ("cbor", "ciborium"),
        ("msgpack", "rmp-serde"),
    ];
    for row in MATRIX {
        let manifest = generate_for(row);
        for (codec_id, crate_name) in FRAGMENTS {
            let enabled = row.codecs.contains(codec_id);
            let present = manifest.contains(&format!("{crate_name} = "));
            assert_eq!(
                enabled, present,
                "{}[{}]: `{crate_name}` dependency presence ({present}) \
                 does not match enablement ({enabled})",
                row.fixture_stem, row.tag
            );
        }
    }
}

#[test]
fn features_section_mirrors_support_graph_and_selection() {
    for row in MATRIX {
        let manifest = generate_for(row);
        let expected_default = {
            let mut names = Vec::new();
            if row.features.client {
                names.push("\"client\"");
            }
            if row.features.server {
                names.push("\"server\"");
            }
            format!("default = [{}]", names.join(", "))
        };
        assert!(
            manifest.contains(&expected_default),
            "{}[{}]: default features mismatch (wanted {expected_default}):\n{manifest}",
            row.fixture_stem,
            row.tag
        );
        // EXACT support-graph mirror (D-impl-crate): both lines always
        // declared; only `default` reflects the selection. Routing uses the
        // dependency's literal hyphenated key.
        assert!(manifest.contains(
            "client = [\"openapi-support/client\", \"dep:reqwest\", \
             \"dep:hyper\", \"dep:tokio\", \"dep:tokio-util\", \
             \"dep:futures-util\"]"
        ));
        assert!(manifest.contains(
            "server = [\"openapi-support/server\", \"dep:axum\", \
             \"dep:tokio\", \"dep:tokio-util\", \"dep:futures-util\"]"
        ));
    }
}

#[test]
fn key_ordering_is_stable_regardless_of_set_insertion_order() {
    // BTreeMap/BTreeSet guarantee sorted iteration; this proves the emitted
    // bytes do not depend on how callers built their sets.
    let doc = normalize_fixture(JSON_FIXTURE);
    let forward: BTreeSet<&'static str> = ["xml", "cbor", "msgpack"].into_iter().collect();
    let mut backward = BTreeSet::new();
    for id in ["msgpack", "cbor", "xml"] {
        backward.insert(id);
    }
    assert_eq!(forward, backward, "set equality is insertion-order free");
    let build = |codecs: BTreeSet<&'static str>| {
        let plan_config = PlanConfig {
            generator_options: GeneratorPlanOptions {
                enabled_codecs: codecs.clone(),
                overrides: Vec::new(),
            },
            ..PlanConfig::default()
        };
        let planned = plan_api_with_config(&doc, &plan_config).expect("plans");
        generate_manifest(
            &doc,
            &planned,
            &ManifestConfig {
                toolchain: EmbeddedToolchain::CURRENT,
                features: FeatureSelection::default(),
                enabled_codecs: codecs,
                overrides: None,
            },
        )
        .expect("emits")
    };
    // Fixture 01 declares no codec media types, but the manifest still
    // carries the configured runtime crates; both insertion orders must
    // render byte-identically (deps sort by crate name).
    assert_eq!(build(forward), build(backward));
}

// ----------------------------------------------------------------------
// Validation contract (§3.1: unsupported combinations are errors)
// ----------------------------------------------------------------------

#[test]
fn unknown_codec_ids_are_rejected_listing_registry_ids() {
    let doc = normalize_fixture(JSON_FIXTURE);
    let plan = plan_api_with_config(&doc, &PlanConfig::default()).expect("plans");
    let err = generate_manifest(
        &doc,
        &plan,
        &ManifestConfig {
            toolchain: EmbeddedToolchain::CURRENT,
            features: FeatureSelection::default(),
            enabled_codecs: ["bogus"].into_iter().collect(),
            overrides: None,
        },
    )
    .expect_err("unknown codec must be rejected");
    let message = err
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(message.contains("manifest_unknown_codec"), "{message}");
    for registry_id in ["xml", "cbor", "msgpack"] {
        assert!(
            message.contains(registry_id),
            "registry id `{registry_id}` not listed: {message}"
        );
    }
}

#[test]
fn unplanned_codec_claims_without_configuration_are_rejected() {
    // Plan WITH xml claimed, then ask the manifest emitter for a config
    // that forgot to enable it: the emitted code would reference an
    // undeclared runtime crate, so this is an error (§3.1 stop-and-report).
    let doc = normalize_fixture(CODEC_FIXTURE);
    let plan = plan_api_with_config(
        &doc,
        &PlanConfig {
            generator_options: GeneratorPlanOptions {
                enabled_codecs: ["xml"].into_iter().collect(),
                overrides: Vec::new(),
            },
            ..PlanConfig::default()
        },
    )
    .expect("plans");
    assert!(
        planned_uses_codec(&plan, "xml"),
        "fixture 16 must claim xml"
    );
    let err = generate_manifest(
        &doc,
        &plan,
        &ManifestConfig {
            toolchain: EmbeddedToolchain::CURRENT,
            features: FeatureSelection::default(),
            enabled_codecs: BTreeSet::new(),
            overrides: None,
        },
    )
    .expect_err("unconfigured claim must be rejected");
    assert!(
        err.iter()
            .any(|diagnostic| diagnostic.code == "manifest_codec_not_enabled"),
        "{err:?}"
    );
}

fn planned_uses_codec(
    plan: &openapi_to_rust_generator::codegen::plan::PlannedApi,
    plugin_id: &str,
) -> bool {
    plan.operations.iter().any(|operation| {
        operation
            .request_contents
            .iter()
            .chain(
                operation
                    .statuses
                    .iter()
                    .flat_map(|status| status.contents.iter()),
            )
            .any(|content| {
                content
                    .codec
                    .as_ref()
                    .is_some_and(|c| c.plugin_id == plugin_id)
            })
    })
}

#[test]
fn invalid_requirements_are_rejected() {
    let doc = normalize_fixture(JSON_FIXTURE);
    let plan = plan_api_with_config(&doc, &PlanConfig::default()).expect("plans");
    for (label, bad) in [
        ("wildcard", "*"),
        ("prerelease", "0.12.0-alpha.1"),
        ("range", "<0.12"),
        ("empty", ""),
    ] {
        let err = generate_manifest(
            &doc,
            &plan,
            &ManifestConfig {
                toolchain: EmbeddedToolchain::CURRENT,
                features: FeatureSelection::default(),
                enabled_codecs: BTreeSet::new(),
                overrides: Some(ManifestOverrides {
                    reqwest_version: Some(bad.to_owned()),
                    ..ManifestOverrides::default()
                }),
            },
        )
        .expect_err("{label} requirement must be rejected");
        assert!(
            err.iter()
                .any(|diagnostic| diagnostic.code == "manifest_invalid_requirement"),
            "{label}: {err:?}"
        );
    }
}

#[test]
fn featureless_selection_is_rejected() {
    let doc = normalize_fixture(JSON_FIXTURE);
    let plan = plan_api_with_config(&doc, &PlanConfig::default()).expect("plans");
    let err = generate_manifest(
        &doc,
        &plan,
        &ManifestConfig {
            toolchain: EmbeddedToolchain::CURRENT,
            features: FeatureSelection {
                client: false,
                server: false,
            },
            enabled_codecs: BTreeSet::new(),
            overrides: None,
        },
    )
    .expect_err("featureless selection must be rejected");
    assert!(
        err.iter()
            .any(|diagnostic| diagnostic.code == "manifest_featureless"),
        "{err:?}"
    );
}

// ----------------------------------------------------------------------
// Package-name derivation (§3.1 sanitation)
// ----------------------------------------------------------------------

#[test]
fn package_names_are_derived_deterministically() {
    assert_eq!(package_name(Some("widgets")), "widgets-api");
    assert_eq!(package_name(Some("Widget Store")), "widget-store-api");
    assert_eq!(package_name(Some("HTTP API")), "http-api");
    // Keyword sanitation collapses into single separators, never doubles.
    assert_eq!(package_name(Some("type")), "type-api");
    // Digit-leading titles keep digits (legal mid-name after sanitation).
    assert_eq!(package_name(Some("42 Crates")), "42-crates-api");
    // Non-ASCII drops deterministically.
    assert_eq!(package_name(Some("Café Store")), "caf-store-api");
    assert_eq!(package_name(None), "generated-api");
    assert_eq!(package_name(Some("   ")), "generated-api");
    assert_eq!(package_name(Some("***")), "generated-api");
}
