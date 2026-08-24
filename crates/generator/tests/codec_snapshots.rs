//! Codec-emission harness (main spec §5.9/§45, §50 tests 39–40;
//! DECISIONS.md D-impl-codec-plugins, D-impl-override-precedence).
//!
//! Generation is CONFIG-dependent now: fixture 16 (`16_codecs.yaml`) is
//! planned under THREE single-codec configurations (xml-only, cbor-only,
//! msgpack-only) plus the default (all codecs OFF), asserting
//!
//! 1. byte-determinism per configuration across independent generations and
//!    byte-for-byte agreement with committed per-config snapshots, and
//! 2. structural properties: decode calls reference the enabled runtime crate
//!    ONLY (`::quick_xml` / `::ciborium` / `::rmp_serde` paths), no streaming
//!    wrapper exists for codec-bound statuses, `Accept` literals are
//!    unchanged from the default run, and the default run keeps every entry
//!    raw-streaming with zero codec references (§5.9 fallback).
//!
//! Snapshot regeneration: `CODEC_SNAPSHOT_UPDATE=1 cargo test`.

use std::path::PathBuf;

use openapi_to_rust_generator::codegen::client::generate_client;
use openapi_to_rust_generator::codegen::plan::{
    plan_api_with_config, GeneratorPlanOptions, OperationPattern, PlanConfig,
    RepresentationOverride,
};
use openapi_to_rust_generator::codegen::server::generate_server;
use openapi_to_rust_generator::normalize::{normalize_with_config, NormalizeConfig};
use openapi_to_rust_generator::parse::{load_document, LoadConfig};

const FIXTURE: &str = "16_codecs.yaml";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

/// One single-codec configuration: `(config tag, enabled ids)`.
const CODEC_CONFIGS: &[(&str, &[&str])] = &[
    ("xml", &["xml"]),
    ("cbor", &["cbor"]),
    ("msgpack", &["msgpack"]),
];

fn options_with(ids: &[&'static str]) -> GeneratorPlanOptions {
    GeneratorPlanOptions {
        enabled_codecs: ids.iter().copied().collect(),
        overrides: Vec::new(),
    }
}

fn normalize_fixture() -> openapi_to_rust_generator::normalize::NormalizedDocument {
    let ir = load_document(FIXTURE, &fixtures_dir(), &LoadConfig::default())
        .unwrap_or_else(|diags| panic!("{FIXTURE} must load: {diags:?}"));
    normalize_with_config(ir, &NormalizeConfig::default())
        .unwrap_or_else(|diags| panic!("{FIXTURE} must normalize: {diags:?}"))
}

fn generate_with(options: &GeneratorPlanOptions) -> (String, String) {
    let doc = normalize_fixture();
    let config = PlanConfig {
        generator_options: options.clone(),
        ..PlanConfig::default()
    };
    let plan = plan_api_with_config(&doc, &config)
        .unwrap_or_else(|diags| panic!("{FIXTURE} must plan: {diags:?}"));
    let client = generate_client(&doc, &plan);
    let server = generate_server(&doc, &plan);
    (client, server)
}

fn snapshot_path(tag: &str, artifact: &str) -> PathBuf {
    snapshots_dir().join(format!("16_codecs.{tag}.{artifact}.rs"))
}

// ----------------------------------------------------------------------
// Per-config determinism + committed snapshots (main spec §50 test 39)
// ----------------------------------------------------------------------

#[test]
fn codec_configs_are_deterministic_and_match_snapshots() {
    std::fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
    for (tag, ids) in CODEC_CONFIGS {
        let options = options_with(ids);
        let (client, server) = generate_with(&options);

        // Double-generation check per configuration.
        let (client_again, server_again) = generate_with(&options);
        assert_eq!(client, client_again, "{tag}: client not deterministic");
        assert_eq!(server, server_again, "{tag}: server not deterministic");

        // The plan must actually claim entries under this configuration.
        assert!(
            client.contains("codec"),
            "{tag}: no codec claim visible in generated client"
        );

        for (artifact, text) in [("client", &client), ("server", &server)] {
            let snapshot = snapshot_path(tag, artifact);
            if std::env::var("CODEC_SNAPSHOT_UPDATE").as_deref() == Ok("1") {
                std::fs::write(&snapshot, text)
                    .unwrap_or_else(|err| panic!("write snapshot {}: {err}", snapshot.display()));
                continue;
            }
            let expected = std::fs::read_to_string(&snapshot).unwrap_or_else(|_| {
                panic!(
                    "missing snapshot {}; run with CODEC_SNAPSHOT_UPDATE=1",
                    snapshot.display()
                )
            });
            assert_eq!(
                text, &expected,
                "{tag}: generated {artifact} diverged from snapshot"
            );
        }
    }
}

// ----------------------------------------------------------------------
// Structural assertions per configuration
// ----------------------------------------------------------------------

#[test]
fn each_codec_references_only_its_runtime_crate_paths() {
    for (tag, _) in CODEC_CONFIGS {
        let text = std::fs::read_to_string(snapshot_path(tag, "client")).expect("codec snapshot");
        match *tag {
            "xml" => {
                assert!(text.contains("use ::quick_xml::de::from_reader;"), "{text}");
                assert!(text.contains("Serializer::with_root"), "{text}");
                assert!(!text.contains("::ciborium"), "{text}");
                assert!(!text.contains("::rmp_serde"), "{text}");
            }
            "cbor" => {
                assert!(text.contains("use ::ciborium::de::from_reader;"), "{text}");
                assert!(text.contains("::ciborium::ser::into_writer"), "{text}");
                assert!(!text.contains("::quick_xml"), "{text}");
                assert!(!text.contains("::rmp_serde"), "{text}");
            }
            "msgpack" => {
                assert!(text.contains("use ::rmp_serde::from_slice;"), "{text}");
                assert!(
                    text.contains("::rmp_serde::encode::write(writer, value)"),
                    "{text}"
                );
                // §49: MessagePack never buffers through to_vec.
                assert!(!text.contains("to_vec"), "{text}");
                assert!(!text.contains("::quick_xml"), "{text}");
                assert!(!text.contains("::ciborium"), "{text}");
            }
            other => panic!("unknown codec config tag {other}"),
        }
        // Bounded encode goes through the fail-fast counting writer on both
        // sides (§34).
        let server = std::fs::read_to_string(snapshot_path(tag, "server")).expect("snapshot");
        assert!(server.contains("serialize_with_writer_limited"), "{server}");
    }
}

#[test]
fn codec_bound_statuses_never_own_streaming_wrappers() {
    // The operation each codec claims, plus the model its 200/201 decodes.
    const CLAIMED: &[(&str, &str, &str)] = &[
        ("xml", "create_xml_document", "XmlDocument"),
        ("cbor", "put_cbor_state", "CborState"),
        ("msgpack", "post_msg_pack_event", "MsgPackEvent"),
    ];
    for (tag, method, model) in CLAIMED {
        let client = std::fs::read_to_string(snapshot_path(tag, "client")).expect("snapshot");
        let start = client
            .find(&format!("pub async fn {method}("))
            .unwrap_or_else(|| panic!("{tag}: method {method} missing"));
        let next = client[start + 10..]
            .find("pub async fn")
            .map(|offset| start + 10 + offset)
            .unwrap_or(client.len());
        let block = &client[start..next];
        // The claimed status decodes into the shared model through the codec
        // helper — never a streaming wrapper or raw response (§45).
        assert!(
            block.contains(&format!("{model}_decode_typed"))
                || block.contains(&format!("{}_decode_typed", tag)),
            "{tag}: bounded codec decode missing for {method}:\n{block}"
        );
        assert!(
            !block.contains("::reqwest::Response"),
            "{tag}: codec-bound status owns a raw response:\n{block}"
        );
        // Responses still collect through the bounded collector (§45).
        let whole = std::fs::read_to_string(snapshot_path(tag, "client")).expect("snapshot");
        assert!(
            whole.contains("collect_reqwest_limited"),
            "{tag}: bounded collection missing"
        );
    }
}

#[test]
fn accept_literals_match_the_default_configuration() {
    // §29: enabling a codec changes representation, never negotiation. Every
    // Accept literal must be identical across ALL configurations.
    let mut accepts: Vec<Vec<String>> = Vec::new();
    for (_, options) in [
        ("default", GeneratorPlanOptions::default()),
        ("xml", options_with(&["xml"])),
        ("cbor", options_with(&["cbor"])),
        ("msgpack", options_with(&["msgpack"])),
    ] {
        let (client, _) = generate_with(&options);
        let mut found: Vec<String> = Vec::new();
        for line in client.lines() {
            if line.contains("::http::header::ACCEPT") && !line.contains("request =") {
                found.push(line.trim().to_owned());
            }
        }
        found.sort();
        accepts.push(found);
    }
    let first = accepts.first().expect("default run present").clone();
    for (index, other) in accepts.iter().enumerate().skip(1) {
        assert_eq!(
            &first, other,
            "configuration #{index} changed the Accept literals (§29)"
        );
    }
    assert!(
        !first.is_empty(),
        "fixture 16 must declare at least one Accept header"
    );
}

#[test]
fn default_configuration_stays_raw_streaming_with_zero_codec_refs() {
    // §5.9: without a configured codec the XML/CBOR/MessagePack entries fall
    // back to raw streaming and are NEVER guessed into an eager form.
    let (client, server) = generate_with(&GeneratorPlanOptions::default());
    for text in [&client, &server] {
        for forbidden in [
            "quick_xml",
            "ciborium",
            "rmp_serde",
            "_decode_typed",
            "_decode_body",
            "XmlFmtSink",
        ] {
            assert!(
                !text.contains(forbidden),
                "default configuration leaked codec machinery `{forbidden}`:\n{text}"
            );
        }
    }
    assert!(
        client.contains("::reqwest::Response"),
        "raw fallback must own responses on the client:\n{client}"
    );
    assert!(
        server.contains("axum::body::Body") || server.contains("::axum::body::Body"),
        "raw fallback must stream bodies on the server:\n{server}"
    );
}

#[test]
fn force_streaming_override_beats_codec_claims_and_keeps_literals() {
    // D-impl-override-precedence: override > claiming plugin > default.
    // With EVERY codec enabled, an Exact(application/xml) override must keep
    // the XML operation raw-streaming while CBOR/MessagePack stay typed.
    let options = GeneratorPlanOptions {
        enabled_codecs: ["xml", "cbor", "msgpack"].into_iter().collect(),
        overrides: vec![RepresentationOverride::ForceStreaming {
            match_media: openapi_to_rust_generator::codegen::plan::MediaTypePattern::Exact(
                "application/xml".to_owned(),
            ),
            match_operation: OperationPattern::Any,
        }],
    };
    let doc = normalize_fixture();
    let config = PlanConfig {
        generator_options: options,
        ..PlanConfig::default()
    };
    let plan = plan_api_with_config(&doc, &config)
        .unwrap_or_else(|diags| panic!("{FIXTURE} must plan: {diags:?}"));

    // Find the xml operation in the plan.
    let xml_op = plan
        .operations
        .iter()
        .find(|op| op.method == "create_xml_document")
        .expect("xml operation planned");
    let request_entry = &xml_op.request_contents[0];
    assert!(
        request_entry.codec.is_none(),
        "override must suppress the codec claim"
    );
    assert_eq!(
        request_entry.media_type_literal, "application/xml",
        "override keeps the media-type literal verbatim"
    );

    // CBOR/MessagePack operations remain codec-bound.
    for method in ["put_cbor_state", "post_msg_pack_event"] {
        let op = plan
            .operations
            .iter()
            .find(|op| op.method == method)
            .unwrap_or_else(|| panic!("{method} planned"));
        assert!(
            op.request_contents[0].codec.is_some(),
            "{method} must stay codec-bound under the override"
        );
    }

    let client = generate_client(&doc, &plan);
    let xml_method_start = client
        .find("pub async fn create_xml_document")
        .expect("method");
    let next_method = client[xml_method_start + 10..]
        .find("pub async fn")
        .map(|offset| xml_method_start + 10 + offset)
        .unwrap_or(client.len());
    let xml_method = &client[xml_method_start..next_method];
    assert!(
        xml_method.contains("::reqwest::Body"),
        "forced-streaming operation must take reqwest::Body:\n{xml_method}"
    );
    assert!(
        !xml_method.contains("xml_encode_limited"),
        "forced-streaming operation must skip bounded encoding:\n{xml_method}"
    );
}

// ----------------------------------------------------------------------
// rustfmt-clean emission (main spec §50 test 40)
// ----------------------------------------------------------------------

#[test]
fn codec_generated_files_are_rustfmt_clean() {
    let Some(rustfmt) = locate_rustfmt() else {
        eprintln!(
            "WARNING: no rustfmt binary on PATH; skipping the rustfmt-clean assertion \
             (main spec §50 test 40)"
        );
        return;
    };
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    for (tag, ids) in CODEC_CONFIGS {
        let options = options_with(ids);
        let (client, server) = generate_with(&options);
        for (artifact, text) in [("client", &client), ("server", &server)] {
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let id = COUNTER.load(std::sync::atomic::Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "o2r-codec-fmt-{id}-{}-{tag}-{artifact}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            let source = dir.join(format!("16_codecs.{tag}.{artifact}.rs"));
            std::fs::write(&source, text).expect("write generated file");

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
                "{tag}/{artifact}: codec output is not rustfmt-clean\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&checked.stdout),
                String::from_utf8_lossy(&checked.stderr),
            );
        }
    }
}

/// Resolves a usable rustfmt: plain PATH lookup first, then the rustup shim
/// next to the running toolchain's cargo.
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
