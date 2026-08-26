//! CLI entry point for the deterministic OpenAPI → Rust generator.
//!
//! Two modes share one hand-rolled argument loop (main spec §50
//! reproducibility: no timestamps, no paths; errors print diagnostics to
//! stderr and exit 1, usage errors exit 2):
//!
//! - `openapi-to-rust --dump <path>` prints the deterministic normalized
//!   dump to stdout (unchanged behavior).
//! - `openapi-to-rust <path> [--generate …] [--types-path …] [--out-dir …]`
//!   generates source artifacts into a directory. Artifacts are selected
//!   through the extensible `--generate` namespace (`types`, `client`,
//!   `server`, `all`; repeated and comma-separated forms are equivalent);
//!   omitting `--generate` selects everything, preserving the historical
//!   all-in-one output byte-for-byte (DECISIONS.md
//!   D-impl-selective-artifacts).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use openapi_to_rust_generator::codegen::client::generate_client_with_config;
use openapi_to_rust_generator::codegen::config::{CodegenConfig, TypesLocation};
use openapi_to_rust_generator::codegen::models::generate_models;
use openapi_to_rust_generator::codegen::plan::{plan_api, PlannedApi};
use openapi_to_rust_generator::codegen::server::generate_server_with_config;
use openapi_to_rust_generator::codegen::views::generate_views;
use openapi_to_rust_generator::diagnostics::{Diagnostic, Severity};
use openapi_to_rust_generator::normalize::dump::dump_normalized;
use openapi_to_rust_generator::normalize::NormalizedDocument;
use openapi_to_rust_generator::{normalize_with_config, parse::load_document, NormalizeConfig};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// One selectable generation artifact (D-impl-selective-artifacts). The
/// declaration order doubles as the canonical emission order, so argument
/// order can never affect which bytes land where; future artifacts (tests,
/// mocks, docs) join this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Artifact {
    /// Shared schema surface: `models.rs` + directional `views.rs`.
    Types,
    /// Reqwest client: `client.rs`.
    Client,
    /// Axum router/trait: `server.rs`.
    Server,
}

impl Artifact {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "types" => Some(Self::Types),
            "client" => Some(Self::Client),
            "server" => Some(Self::Server),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Types => "types",
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

const SUPPORTED_ARTIFACTS: &str = "types, client, server";

/// Expands repeated/comma-separated selections into the canonical set.
/// `all` is shorthand for every artifact; repeats deduplicate.
fn parse_selection(values: &[String]) -> Result<BTreeSet<Artifact>, String> {
    let mut selected = BTreeSet::new();
    for raw in values {
        for part in raw.split(',') {
            let value = part.trim();
            if value == "all" {
                selected.insert(Artifact::Types);
                selected.insert(Artifact::Client);
                selected.insert(Artifact::Server);
                continue;
            }
            let Some(artifact) = Artifact::parse(value) else {
                let hint = if value.is_empty() {
                    String::from("found an empty entry")
                } else {
                    format!("unknown value `{value}`")
                };
                return Err(format!(
                    "{hint} in --generate; supported values: {SUPPORTED_ARTIFACTS}, \
                     or `all` for {SUPPORTED_ARTIFACTS}"
                ));
            };
            selected.insert(artifact);
        }
    }
    Ok(selected)
}

struct GenerationOptions<'a> {
    document: &'a Path,
    selection: BTreeSet<Artifact>,
    types_path: Option<&'a str>,
    out_dir: &'a Path,
}

fn run(args: &[String]) -> Result<(), std::process::ExitCode> {
    let mut dump_target: Option<&str> = None;
    let mut document: Option<&str> = None;
    let mut generate_values: Vec<String> = Vec::new();
    let mut types_path: Option<&str> = None;
    let mut out_dir: Option<&str> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dump" => {
                index += 1;
                let Some(target) = args.get(index) else {
                    eprintln!("error: `--dump` requires a path to a YAML document");
                    return Err(std::process::ExitCode::from(2));
                };
                dump_target = Some(target.as_str());
            }
            "--generate" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    eprintln!(
                        "error: `--generate` requires an artifact list; supported \
                         values: {SUPPORTED_ARTIFACTS} (or `all`)"
                    );
                    return Err(std::process::ExitCode::from(2));
                };
                generate_values.push(value.clone());
            }
            "--types-path" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    eprintln!(
                        "error: `--types-path` requires a Rust module/crate path \
                         (e.g. `api_types`, `crate::generated::types`)"
                    );
                    return Err(std::process::ExitCode::from(2));
                };
                types_path = Some(value.as_str());
            }
            "--out-dir" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    eprintln!("error: `--out-dir` requires a directory path");
                    return Err(std::process::ExitCode::from(2));
                };
                out_dir = Some(value.as_str());
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => {
                if other.starts_with('-') {
                    eprintln!(
                        "error: unknown argument `{other}`; usage: openapi-to-rust \
                         --dump <path> | openapi-to-rust <path> [options]"
                    );
                    return Err(std::process::ExitCode::from(2));
                }
                if document.is_some() {
                    eprintln!(
                        "error: unexpected extra argument `{other}`; exactly one \
                         document path is accepted"
                    );
                    return Err(std::process::ExitCode::from(2));
                }
                document = Some(other);
            }
        }
        index += 1;
    }

    if let Some(target) = dump_target {
        if document.is_some()
            || !generate_values.is_empty()
            || types_path.is_some()
            || out_dir.is_some()
        {
            eprintln!("error: `--dump` cannot be combined with generation arguments");
            return Err(std::process::ExitCode::from(2));
        }
        return run_dump(target);
    }

    let Some(document) = document else {
        eprintln!("usage: openapi-to-rust --dump <path-to-yaml>");
        eprintln!("       openapi-to-rust <path-to-yaml> [options]");
        return Err(std::process::ExitCode::from(2));
    };

    let selection = match parse_selection(&generate_values) {
        Ok(selection) => selection,
        Err(message) => {
            eprintln!("error: {message}");
            return Err(std::process::ExitCode::from(2));
        }
    };

    // Default selection preserves the historical all-in-one behavior:
    // types + client + server (D-impl-selective-artifacts).
    let selection = if selection.is_empty() {
        [Artifact::Types, Artifact::Client, Artifact::Server]
            .into_iter()
            .collect()
    } else {
        selection
    };

    // Validation rule 1: client/server WITHOUT local types need an external
    // shared-types path.
    let transport_without_types: Vec<&'static str> = [Artifact::Client, Artifact::Server]
        .into_iter()
        .filter(|artifact| selection.contains(artifact) && !selection.contains(&Artifact::Types))
        .map(Artifact::name)
        .collect();
    if !transport_without_types.is_empty() && types_path.is_none() {
        let names = transport_without_types.join("`, `");
        eprintln!("error: generating `{names}` without `types` requires --types-path");
        eprintln!();
        eprintln!("example:");
        eprintln!("  openapi-to-rust api.yaml \\");
        eprintln!("    --generate {} \\", transport_without_types[0]);
        eprintln!("    --types-path api_types");
        return Err(std::process::ExitCode::from(2));
    }

    // Validation rule 2: naming BOTH sources of shared types in one
    // invocation is ambiguous — local siblings vs external base.
    if types_path.is_some() && selection.contains(&Artifact::Types) {
        eprintln!(
            "error: --types-path cannot be used when `types` is generated in \
             the same invocation"
        );
        return Err(std::process::ExitCode::from(2));
    }

    let options = GenerationOptions {
        document: Path::new(document),
        selection,
        types_path,
        out_dir: Path::new(out_dir.unwrap_or(".")),
    };
    run_generation(&options)
}

fn run_dump(target: &str) -> Result<(), std::process::ExitCode> {
    let (root_yaml, root_dir) = resolve_document(Path::new(target));

    // The root directory is the file's parent so relative external refs
    // resolve next to the document (D-§3).
    let ir = match load_document(&root_yaml, &root_dir, &Default::default()) {
        Ok(ir) => ir,
        Err(diagnostics) => return Err(fail(diagnostics)),
    };
    let normalized = match normalize_with_config(ir, &NormalizeConfig::default()) {
        Ok(normalized) => normalized,
        Err(diagnostics) => return Err(fail(diagnostics)),
    };

    print!("{}", dump_normalized(&normalized));
    Ok(())
}

fn run_generation(options: &GenerationOptions<'_>) -> Result<(), std::process::ExitCode> {
    let (root_yaml, root_dir) = resolve_document(options.document);

    let ir = match load_document(&root_yaml, &root_dir, &Default::default()) {
        Ok(ir) => ir,
        Err(diagnostics) => return Err(fail(diagnostics)),
    };
    let doc = match normalize_with_config(ir, &NormalizeConfig::default()) {
        Ok(doc) => doc,
        Err(diagnostics) => return Err(fail(diagnostics)),
    };
    report_warnings(&doc.diagnostics);
    if doc
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Err(fail(doc.diagnostics.clone()));
    }
    let plan = match plan_api(&doc) {
        Ok(plan) => plan,
        Err(diagnostics) => return Err(fail(diagnostics)),
    };

    // The shared-types location: sibling modules when generated in THIS
    // invocation, otherwise the validated external base path.
    let config = match options.types_path {
        Some(path) => match TypesLocation::external(path) {
            Ok(types_location) => CodegenConfig { types_location },
            Err(reason) => {
                eprintln!("error: invalid --types-path value `{path}`: {reason}");
                return Err(std::process::ExitCode::from(2));
            }
        },
        None => CodegenConfig::default(),
    };

    write_artifacts(options.out_dir, &options.selection, &doc, &plan, &config)?;
    Ok(())
}

/// Renders every selected artifact into `out_dir` (created on demand), then
/// reports the written files. Emission follows the canonical artifact order,
/// never user argument order.
fn write_artifacts(
    out_dir: &Path,
    selection: &BTreeSet<Artifact>,
    doc: &NormalizedDocument,
    plan: &PlannedApi,
    config: &CodegenConfig,
) -> Result<(), std::process::ExitCode> {
    let mut outputs: Vec<(&str, String)> = Vec::new();
    for artifact in selection {
        match artifact {
            // `types` is BOTH modules: shared models plus their directional
            // read/write views (companion §5); they are one logical unit.
            Artifact::Types => {
                outputs.push(("models.rs", generate_models(doc)));
                outputs.push(("views.rs", generate_views(doc)));
            }
            Artifact::Client => {
                outputs.push(("client.rs", generate_client_with_config(doc, plan, config)));
            }
            Artifact::Server => {
                outputs.push(("server.rs", generate_server_with_config(doc, plan, config)));
            }
        }
    }

    if let Err(err) = std::fs::create_dir_all(out_dir) {
        eprintln!(
            "error: cannot create output directory {}: {err}",
            out_dir.display()
        );
        return Err(std::process::ExitCode::from(1));
    }
    for (name, text) in &outputs {
        let path = out_dir.join(name);
        if let Err(err) = std::fs::write(&path, text) {
            eprintln!("error: cannot write {}: {err}", path.display());
            return Err(std::process::ExitCode::from(1));
        }
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// Prints Warning-severity diagnostics without stopping; Errors are handled
/// by the caller (stop-and-report policy).
fn report_warnings(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        if diagnostic.severity == Severity::Warning {
            eprintln!("{diagnostic}");
        }
    }
}

fn fail(diagnostics: Vec<Diagnostic>) -> std::process::ExitCode {
    for diagnostic in &diagnostics {
        eprintln!("{diagnostic}");
    }
    std::process::ExitCode::from(1)
}

fn print_help() {
    println!(
        "\
openapi-to-rust — deterministic OpenAPI to Rust generator

USAGE:
  openapi-to-rust --dump <path-to-yaml>
      Print the deterministic normalized dump to stdout.

  openapi-to-rust <path-to-yaml> [OPTIONS]
      Generate Rust source artifacts for the document.

OPTIONS:
  --generate <artifacts>
          Which artifacts to generate: {SUPPORTED_ARTIFACTS}.
          Accepts repeated flags and/or one comma-separated list;
          `all` is shorthand for `{SUPPORTED_ARTIFACTS}`.
          Defaults to `{SUPPORTED_ARTIFACTS}`.

  --types-path <RUST_PATH>
          Rust module/crate path where the externally generated shared
          types live (their `models` and `views` modules sit under this
          path). Required for client/server generation when `types` is
          NOT selected in the same invocation, and rejected together
          with `types` (the import base must have exactly one source).
          This is a RUST PATH, not a Cargo package name: a package named
          `api-types` has the crate identifier `api_types`, so pass
          `--types-path api_types`. Examples: `api_types`,
          `crate::types`, `crate::generated::types`, `company_api::v2`.

  --out-dir <dir>
          Directory receiving the generated files (default: `.`).

  -h, --help
          Print this help."
    );
}

/// Canonicalizes the document path (falling back to lexical normalization)
/// and splits it into `(file name, parent directory)` so relative external
/// refs resolve next to the document (D-§3).
fn resolve_document(given: &Path) -> (String, PathBuf) {
    let absolute = std::fs::canonicalize(given).unwrap_or_else(|_| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        normalize_path(&cwd.join(given))
    });
    split_parent(&absolute)
}

fn split_parent(path: &Path) -> (String, PathBuf) {
    match (path.file_name(), path.parent()) {
        (Some(file), Some(parent)) => (file.to_string_lossy().into_owned(), parent.to_path_buf()),
        _ => (path.to_string_lossy().into_owned(), PathBuf::from(".")),
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
