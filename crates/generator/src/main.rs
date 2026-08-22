//! CLI entry point: `openapi-to-rust --dump <path-to-yaml>` prints the
//! deterministic normalized dump to stdout (main spec §50 reproducibility:
//! no timestamps, no paths). Errors print diagnostics to stderr and exit 1.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use openapi_to_rust_generator::{normalize_with_config, parse::load_document, NormalizeConfig};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn run(args: &[String]) -> Result<(), std::process::ExitCode> {
    let mut dump_target: Option<&str> = None;
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
            other => {
                eprintln!(
                    "error: unknown argument `{other}`; usage: openapi-to-rust --dump <path>"
                );
                return Err(std::process::ExitCode::from(2));
            }
        }
        index += 1;
    }

    let Some(target) = dump_target else {
        eprintln!("usage: openapi-to-rust --dump <path-to-yaml>");
        return Err(std::process::ExitCode::from(2));
    };

    // The root directory is the file's parent so relative external refs
    // resolve next to the document (D-§3).
    let given = Path::new(target);
    let absolute = std::fs::canonicalize(given).unwrap_or_else(|_| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        normalize_path(&cwd.join(given))
    });
    let (root_yaml, root_dir) = split_parent(&absolute);

    let ir = match load_document(&root_yaml, &root_dir, &Default::default()) {
        Ok(ir) => ir,
        Err(diagnostics) => return Err(fail(diagnostics)),
    };
    let normalized = match normalize_with_config(ir, &NormalizeConfig::default()) {
        Ok(normalized) => normalized,
        Err(diagnostics) => return Err(fail(diagnostics)),
    };

    print!(
        "{}",
        openapi_to_rust_generator::normalize::dump::dump_normalized(&normalized)
    );
    Ok(())
}

fn fail(
    diagnostics: Vec<openapi_to_rust_generator::diagnostics::Diagnostic>,
) -> std::process::ExitCode {
    for diagnostic in &diagnostics {
        eprintln!("{diagnostic}");
    }
    std::process::ExitCode::from(1)
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
