//! Emitted-crate `Cargo.toml` generation (main spec §3/§3.1;
//! DECISIONS.md D-impl-crate).
//!
//! [`generate_manifest`] renders the complete manifest text of a GENERATED
//! crate as a pure function of its inputs: the normalized document (package
//! naming), the plan (codec-claim integrity), and the [`ManifestConfig`]
//! (embedded toolchain tuple + feature selection). Output is deterministic —
//! no timestamps, no paths (main spec §50) — and dependency keys are ordered
//! by construction (`BTreeMap`), so byte stability never depends on
//! insertion order.
//!
//! # Version policy (§3.1)
//!
//! Every released generator version embeds its supported framework tuple in
//! [`EmbeddedToolchain::CURRENT`] — generator 0.1 targets Axum `0.8.x`,
//! Reqwest `0.12.x`, `http` `1.x`, `bytes` `1.x`, MSRV 1.85 — and emits
//! caret-implied requirements matching it. Requirements are overridable
//! through [`ManifestOverrides`], but overrides pass the same validation:
//! wildcard (`*`) requirements, pre-release tags, and path dependencies are
//! rejected as Errors, never emitted (§3.1 "no floating pre-release or path
//! dependencies"). Generating against an unsupported combination is an
//! error, not best-effort output.
//!
//! # Feature graph
//!
//! `[features]` mirrors `openapi-support`'s own graph EXACTLY (D-impl-crate)
//! plus the feature routing that activates the matching support half:
//!
//! ```toml
//! client = ["openapi_support/client", "dep:reqwest", "dep:hyper", "dep:tokio", "dep:tokio-util", "dep:futures-util"]
//! server = ["openapi_support/server", "dep:axum", "dep:tokio", "dep:tokio-util", "dep:futures-util"]
//! ```
//!
//! Both lines are always declared so consumers can flip a surface on
//! downstream; only `default` reflects the configured selection (§3 default
//! `["client", "server"]`). Support stays dependency-light: codec runtime
//! crates appear ONLY when enabled (D-impl-codec-plugins), rendered from
//! [`super::codecs::manifest_dependency_for`] verbatim.

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostics::{Diagnostic, DocumentPath};
use crate::normalize::naming;
use crate::normalize::NormalizedDocument;

use super::codecs;
use super::plan::PlannedApi;

/// Version requirements for crates outside the §3.1 tuple. Fixed per
/// generator release like the tuple itself; not user-overridable because
/// generated code compiles against their exact APIs.
const SERDE_REQ: &str = "1";
const SERDE_JSON_REQ: &str = "1";
const MIME_REQ: &str = "0.3";
const TOKIO_REQ: &str = "1";
const TOKIO_UTIL_REQ: &str = "0.7";
const FUTURES_CORE_REQ: &str = "0.3";
const FUTURES_UTIL_REQ: &str = "0.3";
const ASYNC_TRAIT_REQ: &str = "0.1";
const HYPER_REQ: &str = "1";
/// Generated crates start at 0.1.0; regeneration is expected to overwrite.
const PACKAGE_VERSION: &str = "0.1.0";
/// Banner identity of THIS generator release (matches
/// [`EmbeddedToolchain::CURRENT::support_crate_version`]'s line).
const GENERATOR_VERSION: &str = "0.1";

/// Fallback package name when the document declares no usable `info.title`.
pub const FALLBACK_PACKAGE_NAME: &str = "generated-api";

/// Embedded supported toolchain tuple of one generator release (main spec
/// §3.1): a property of the RELEASE, never computed from ecosystem state at
/// generation time, so output stays deterministic across machines and points
/// in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedToolchain {
    /// Axum requirement (`"0.8"` → caret-implied `^0.8`).
    pub axum: &'static str,
    /// Reqwest requirement (`"0.12"`).
    pub reqwest: &'static str,
    /// `http` requirement (`"1"`).
    pub http: &'static str,
    /// `bytes` requirement (`"1"`).
    pub bytes: &'static str,
    /// Pinned MSRV emitted as `rust-version` (`"1.85"`).
    pub msrv: &'static str,
    /// `openapi-support` requirement matching this release (`"0.1"`,
    /// D-impl-crate: caret dep on the release, never a path dep).
    pub support_crate_version: &'static str,
}

impl EmbeddedToolchain {
    /// Tuple embedded by THIS generator release (main spec §3.1 example:
    /// Axum 0.8.x, Reqwest 0.12.x, http 1.x, bytes 1.x, MSRV 1.85).
    pub const CURRENT: Self = Self {
        axum: "0.8",
        reqwest: "0.12",
        http: "1",
        bytes: "1",
        msrv: "1.85",
        support_crate_version: "0.1",
    };
}

/// Which transport surfaces the generated crate wires by default (§3):
/// both `client` and `server` by default; the feature DEFINITIONS are always
/// emitted so consumers can enable either later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureSelection {
    /// Wire `client` into `default`.
    pub client: bool,
    /// Wire `server` into `default`.
    pub server: bool,
}

impl FeatureSelection {
    /// Both surfaces wired (the §3 default shape).
    pub const BOTH: Self = Self {
        client: true,
        server: true,
    };
}

impl Default for FeatureSelection {
    fn default() -> Self {
        Self::BOTH
    }
}

/// Requirement overrides for the §3.1 tuple (overridable through generator
/// configuration). `None` fields keep the embedded value. Overrides must
/// still satisfy §3.1: caret-implied, no wildcards, no pre-release tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestOverrides {
    /// Replaces [`EmbeddedToolchain::axum`].
    pub axum_version: Option<String>,
    /// Replaces [`EmbeddedToolchain::reqwest`].
    pub reqwest_version: Option<String>,
    /// Replaces [`EmbeddedToolchain::http`].
    pub http_version: Option<String>,
    /// Replaces [`EmbeddedToolchain::bytes`].
    pub bytes_version: Option<String>,
    /// Replaces [`EmbeddedToolchain::support_crate_version`].
    pub support_version: Option<String>,
}

/// Full manifest configuration: embedded tuple + selection + codecs +
/// overrides.
#[derive(Debug, Clone)]
pub struct ManifestConfig {
    /// Supported toolchain tuple (§3.1); defaults to
    /// [`EmbeddedToolchain::CURRENT`].
    pub toolchain: EmbeddedToolchain,
    /// Default transport surfaces; defaults to client + server (§3).
    pub features: FeatureSelection,
    /// Enabled codec plugin ids (main spec §45); their runtime crates are
    /// emitted ONLY when listed here (D-impl-codec-plugins). Defaults to ALL
    /// OFF.
    pub enabled_codecs: BTreeSet<&'static str>,
    /// Requirement overrides; `None` keeps the embedded tuple verbatim.
    pub overrides: Option<ManifestOverrides>,
}

impl Default for ManifestConfig {
    fn default() -> Self {
        Self {
            toolchain: EmbeddedToolchain::CURRENT,
            features: FeatureSelection::default(),
            enabled_codecs: BTreeSet::new(),
            overrides: None,
        }
    }
}

/// Renders the complete `Cargo.toml` text of the generated crate.
///
/// Pure function of `(doc, plan, cfg)`; calling it twice with equal inputs
/// yields byte-identical output (main spec §50 test 39 applies to manifests
/// exactly as to source artifacts).
///
/// # Errors
///
/// Returns every diagnostic when the configuration is unsupported:
///
/// - an id in [`ManifestConfig::enabled_codecs`] (or claimed by the plan)
///   that no registry plugin provides, with the sorted registry ids listed;
/// - a codec CLAIMED by the plan whose plugin is absent from
///   `enabled_codecs` (the emitted code would reference an undeclared
///   runtime crate);
/// - a requirement (embedded or overridden) that is empty, wildcarded,
///   pre-release-tagged, or otherwise not a plain caret-implied version;
/// - a featureless selection (`client == false && server == false`), which
///   cannot link the generated operation surfaces.
pub fn generate_manifest(
    doc: &NormalizedDocument,
    plan: &PlannedApi,
    cfg: &ManifestConfig,
) -> Result<String, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    validate_codecs(cfg, plan, &mut diags);
    if !cfg.features.client && !cfg.features.server {
        diags.push(Diagnostic {
            severity: crate::diagnostics::Severity::Error,
            path: DocumentPath::root().key("features"),
            code: "manifest_featureless",
            message: "a generated crate must wire at least one of the \
                      `client`/`server` surfaces (main spec §3)"
                .to_owned(),
        });
    }

    let effective = effective_requirements(cfg);
    for (crate_name, requirement) in &effective {
        if let Err(reason) = check_requirement(requirement) {
            diags.push(Diagnostic {
                severity: crate::diagnostics::Severity::Error,
                path: DocumentPath::root()
                    .key("dependencies")
                    .key((*crate_name).to_owned()),
                code: "manifest_invalid_requirement",
                message: format!(
                    "`{crate_name} = \"{requirement}\"` violates the §3.1 \
                     version policy: {reason}"
                ),
            });
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }

    Ok(render(doc, cfg, &effective))
}

/// Effective `(crate name, requirement)` pairs for the tuple-managed crates,
/// with overrides applied over the embedded values.
fn effective_requirements(cfg: &ManifestConfig) -> Vec<(&'static str, String)> {
    let overrides = cfg.overrides.clone().unwrap_or_default();
    let resolve =
        |embedded: &'static str, over: Option<String>| over.unwrap_or_else(|| embedded.to_owned());
    // Order here is irrelevant; rendering sorts by crate name.
    vec![
        ("axum", resolve(cfg.toolchain.axum, overrides.axum_version)),
        (
            "reqwest",
            resolve(cfg.toolchain.reqwest, overrides.reqwest_version),
        ),
        ("http", resolve(cfg.toolchain.http, overrides.http_version)),
        (
            "bytes",
            resolve(cfg.toolchain.bytes, overrides.bytes_version),
        ),
        (
            "openapi-support",
            resolve(
                cfg.toolchain.support_crate_version,
                overrides.support_version,
            ),
        ),
    ]
}

/// §3.1 validation: a requirement must be a plain caret-implied version —
/// non-empty, digit-led, digits/dots only after the leading run, and never a
/// wildcard or pre-release tag.
fn check_requirement(requirement: &str) -> Result<(), &'static str> {
    if requirement.is_empty() {
        return Err("empty requirement");
    }
    if !requirement.starts_with(|c: char| c.is_ascii_digit()) {
        return Err("requirements must start with a digit (wildcard and \
                    comparison operators are floating, §3.1)");
    }
    if !requirement.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err("pre-release tags and other separators are forbidden \
                    (§3.1: no floating pre-release dependencies)");
    }
    if requirement.ends_with('.') || requirement.contains("..") {
        return Err("malformed version segment");
    }
    Ok(())
}

/// Codec validation across BOTH sources: configured ids must exist in the
/// registry (`manifest_unknown_codec`, listing every registry id), and every
/// codec claim carried by the plan must be configured
/// (`manifest_codec_not_enabled`) or the emitted code would reference an
/// undeclared runtime crate.
fn validate_codecs(cfg: &ManifestConfig, plan: &PlannedApi, diags: &mut Vec<Diagnostic>) {
    let registry_ids: Vec<&'static str> =
        codecs::default_registry().iter().map(|p| p.id()).collect();

    let mut unknown: BTreeSet<&str> = BTreeSet::new();
    for id in &cfg.enabled_codecs {
        if !registry_ids.contains(id) {
            unknown.insert(id);
        }
    }
    for id in planned_codec_ids(plan) {
        if !registry_ids.contains(&id) {
            unknown.insert(id);
        }
        if !cfg.enabled_codecs.contains(id) {
            diags.push(Diagnostic {
                severity: crate::diagnostics::Severity::Error,
                path: DocumentPath::root()
                    .key("dependencies")
                    .key("openapi-support"),
                code: "manifest_codec_not_enabled",
                message: format!(
                    "the plan claims codec `{id}` but `enabled_codecs` does \
                     not list it; the emitted code would reference an \
                     undeclared runtime crate"
                ),
            });
        }
    }
    if !unknown.is_empty() {
        let known = registry_ids.join(", ");
        diags.push(Diagnostic {
            severity: crate::diagnostics::Severity::Error,
            path: DocumentPath::root().key("dependencies"),
            code: "manifest_unknown_codec",
            message: format!(
                "unknown codec ids [{}]; this release's registry provides \
                 [{known}] (main spec §45)",
                unknown.into_iter().collect::<Vec<_>>().join(", ")
            ),
        });
    }
}

/// Every codec plugin id claimed anywhere in the plan (deterministic order
/// via `BTreeSet`).
fn planned_codec_ids(plan: &PlannedApi) -> BTreeSet<&'static str> {
    let mut ids = BTreeSet::new();
    for operation in &plan.operations {
        for content in &operation.request_contents {
            if let Some(binding) = &content.codec {
                ids.insert(binding.plugin_id);
            }
        }
        for status in &operation.statuses {
            for content in &status.contents {
                if let Some(binding) = &content.codec {
                    ids.insert(binding.plugin_id);
                }
            }
        }
    }
    ids
}

/// Package name from `info.title` (§3.1 deterministic sanitation): title →
/// snake_case pipeline → kebab-case → `-api` suffix unless the sanitized
/// title already ends in an `api` segment (`HTTP API` → `http-api`, never
/// `http-api-api`). Non-ASCII characters drop, separator runs collapse, and
/// digit-leading titles keep their digits (legal mid-name); anything
/// unusable falls back to [`FALLBACK_PACKAGE_NAME`]. Same title ⇒ same
/// name, always.
#[must_use]
pub fn package_name(info_title: Option<&str>) -> String {
    let Some(raw) = info_title.map(str::trim).filter(|title| !title.is_empty()) else {
        return FALLBACK_PACKAGE_NAME.to_owned();
    };
    let snake = naming::ident(raw, naming::NameStyle::Snake);
    let filtered: String = snake
        .chars()
        .filter_map(|c| match c {
            '_' | '-' => Some('-'),
            c if c.is_ascii_lowercase() || c.is_ascii_digit() => Some(c),
            _ => None,
        })
        .collect();
    let mut parts: Vec<&str> = filtered
        .split('-')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return FALLBACK_PACKAGE_NAME.to_owned();
    }
    if parts.last() != Some(&"api") {
        parts.push("api");
    }
    parts.join("-")
}

/// Renders the manifest text. Dependency lines come from one `BTreeMap`
/// keyed by crate name, so ordering is stable BY CONSTRUCTION regardless of
/// insertion or registry order.
fn render(
    doc: &NormalizedDocument,
    cfg: &ManifestConfig,
    requirements: &[(&'static str, String)],
) -> String {
    let mut out = String::new();

    // Banner: pure function of the config; never timestamps or paths.
    out.push_str("# Generated by openapi-to-rust generator ");
    out.push_str(GENERATOR_VERSION);
    out.push_str(" (main spec §3/§3.1). DO NOT EDIT.\n#\n");
    out.push_str("# Embedded toolchain tuple (§3.1): axum ");
    out.push_str(cfg.toolchain.axum);
    out.push_str(", reqwest ");
    out.push_str(cfg.toolchain.reqwest);
    out.push_str(", http ");
    out.push_str(cfg.toolchain.http);
    out.push_str(", bytes ");
    out.push_str(cfg.toolchain.bytes);
    out.push('\n');
    out.push_str("# MSRV ");
    out.push_str(cfg.toolchain.msrv);
    out.push_str(". Requirements are caret-implied and pinned to this\n");
    out.push_str("# generator release; never path or pre-release dependencies.\n\n");

    let name = package_name(doc.info_title.as_deref());
    out.push_str("[package]\n");
    out.push_str(&format!("name = \"{name}\"\n"));
    out.push_str(&format!("version = \"{PACKAGE_VERSION}\"\n"));
    out.push_str("edition = \"2021\"\n");
    out.push_str(&format!("rust-version = \"{}\"\n", cfg.toolchain.msrv));
    out.push('\n');

    out.push_str("[features]\n");
    let mut default_features = Vec::new();
    if cfg.features.client {
        default_features.push("client");
    }
    if cfg.features.server {
        default_features.push("server");
    }
    out.push_str(&format!("default = [{}]\n", quoted_list(&default_features)));
    // Feature lines mirror openapi-support's own graph EXACTLY (D-impl-
    // crate), prefixed by the routing into the matching support half so the
    // generated code can see the support-side items behind those features.
    // Routing uses the dependency's LITERAL key (`openapi-support`);
    // underscored forms are rejected by cargo here.
    out.push_str(&format!(
        "client = [{}, {}, {}, {}, {}, {}]\n",
        "\"openapi-support/client\"",
        "\"dep:reqwest\"",
        "\"dep:hyper\"",
        "\"dep:tokio\"",
        "\"dep:tokio-util\"",
        "\"dep:futures-util\""
    ));
    out.push_str(&format!(
        "server = [{}, {}, {}, {}, {}]\n",
        "\"openapi-support/server\"",
        "\"dep:axum\"",
        "\"dep:tokio\"",
        "\"dep:tokio-util\"",
        "\"dep:futures-util\""
    ));
    out.push('\n');

    // [dependencies]: every entry lands in the map; alphabetical key order
    // is the emission order.
    let mut deps: BTreeMap<String, String> = BTreeMap::new();
    for (crate_name, requirement) in requirements {
        deps.insert((*crate_name).to_owned(), format!("\"{requirement}\""));
    }
    deps.insert("async-trait".to_owned(), format!("\"{ASYNC_TRAIT_REQ}\""));
    deps.insert(
        "axum".to_owned(),
        format!(
            "{{ version = \"{}\", features = [\"multipart\"], optional = true }}",
            cfg.toolchain.axum
        ),
    );
    deps.insert("futures-core".to_owned(), format!("\"{FUTURES_CORE_REQ}\""));
    deps.insert(
        "futures-util".to_owned(),
        format!("{{ version = \"{FUTURES_UTIL_REQ}\", optional = true }}"),
    );
    deps.insert(
        "hyper".to_owned(),
        format!("{{ version = \"{HYPER_REQ}\", optional = true }}"),
    );
    deps.insert("mime".to_owned(), format!("\"{MIME_REQ}\""));
    deps.insert(
        "reqwest".to_owned(),
        format!(
            "{{ version = \"{}\", default-features = false, features = \
             [\"json\", \"multipart\", \"stream\"], optional = true }}",
            cfg.toolchain.reqwest
        ),
    );
    deps.insert(
        "serde".to_owned(),
        format!("{{ version = \"{SERDE_REQ}\", features = [\"derive\"] }}"),
    );
    deps.insert("serde_json".to_owned(), format!("\"{SERDE_JSON_REQ}\""));
    deps.insert(
        "tokio".to_owned(),
        format!(
            "{{ version = \"{TOKIO_REQ}\", features = [\"fs\", \"io-util\"], \
             optional = true }}"
        ),
    );
    deps.insert(
        "tokio-util".to_owned(),
        format!(
            "{{ version = \"{TOKIO_UTIL_REQ}\", features = [\"io\"], \
             optional = true }}"
        ),
    );

    // Codec runtime crates appear ONLY for enabled ids (D-impl-codec-
    // plugins): fragments come verbatim from the plugin registry (§3.1
    // metadata carrier) keyed by their leading crate name so they sort into
    // place alongside everything else. Registry membership was validated
    // above, and `manifest_dependency_for` covers exactly those ids.
    for id in &cfg.enabled_codecs {
        if let Some(fragment) = codecs::manifest_dependency_for(id) {
            if let Some((crate_name, rhs)) = fragment.split_once(" = ") {
                deps.insert(crate_name.to_owned(), rhs.to_owned());
            }
        }
    }

    out.push_str("[dependencies]\n");
    for (crate_name, value) in &deps {
        out.push_str(crate_name);
        out.push_str(" = ");
        out.push_str(value);
        out.push('\n');
    }

    out
}

/// `"a", "b"`-style TOML string-array body.
fn quoted_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|item| format!("\"{item}\""))
        .collect::<Vec<_>>()
        .join(", ")
}
