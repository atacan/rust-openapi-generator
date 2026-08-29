//! Replay harness for the DOCUMENTED example-regeneration commands
//! (D-impl-selective-artifacts): drives the real `oapi-to-rust` binary
//! through `CARGO_BIN_EXE` with EXACTLY the invocations printed in
//! `examples/kitchen-sink/README.md` and `examples/large-upload/README.md`
//! (§ Regeneration), redirecting only `--output-dir` into a scratch tree,
//! then byte-compares every emitted file against the committed artifacts.
//!
//! This pins the documented CLI surface, not just the library pipeline the
//! per-example determinism tests exercise (`models/tests/determinism.rs`):
//! if a README command stops reproducing the committed bytes — through a CLI
//! grammar change, an output-naming change, or doc drift — this test fails.
//!
//! Diagnostic policy matches those gates: any stderr output (Warning or
//! Error) fails, because neither example document is expected to produce
//! diagnostics.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_oapi-to-rust");

/// One example's regeneration contract, mirroring its README verbatim.
struct ExampleSpec {
    /// Directory name under `examples/`.
    dir: &'static str,
    /// The `--types-path` value the README passes for BOTH transports.
    types_path: &'static str,
}

const EXAMPLES: [ExampleSpec; 2] = [
    ExampleSpec {
        dir: "kitchen-sink",
        types_path: "kitchen_sink_models",
    },
    ExampleSpec {
        dir: "large-upload",
        types_path: "large_upload_models",
    },
];

/// `(generated subdirectory, file)` pairs committed per example, relative to
/// the example root.
const ARTIFACTS: [(&str, &str); 4] = [
    ("models/generated", "models.rs"),
    ("models/generated", "views.rs"),
    ("client/generated", "client.rs"),
    ("server/generated", "server.rs"),
];

fn examples_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Unique scratch directory per call site invocation (no tempfile crate,
/// mirroring the repo's pid+counter convention).
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "o2r-example-regen-{}-{id}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create scratch dir");
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Runs ONE documented invocation: `oapi-to-rust <example>/openapi.yaml
/// --generate <artifact> [--types-path <types_path>] --output-dir <out>`.
/// The READMEs document these commands against the repository-relative
/// paths shown above; only `--output-dir` is redirected.
fn run_documented_command(spec: &ExampleSpec, generate: &str, out_dir: &Path) {
    let document = examples_root().join(spec.dir).join("openapi.yaml");
    let mut args: Vec<String> = vec![document.to_string_lossy().into_owned()];
    args.push("--generate".into());
    args.push(generate.into());
    if generate != "types" {
        args.push("--types-path".into());
        args.push(spec.types_path.into());
    }
    args.push("--output-dir".into());
    args.push(out_dir.to_string_lossy().into_owned());

    let displayed = format!("oapi-to-rust {}", args.join(" "));
    let output = Command::new(BIN)
        .args(&args)
        .output()
        .unwrap_or_else(|err| panic!("spawn oapi-to-rust for `{displayed}`: {err}"));
    assert!(
        output.status.success(),
        "documented command failed: `{displayed}`\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stderr.is_empty(),
        "documented command emitted diagnostics (none are expected): \
         `{displayed}`\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Byte-compares one regenerated file against its committed counterpart.
fn assert_matches_committed(example_root: &Path, relative: &str, generated: &Path) {
    let committed = example_root.join(relative);
    let expected = fs::read(&committed)
        .unwrap_or_else(|err| panic!("read committed {}: {err}", committed.display()));
    let actual = fs::read(generated)
        .unwrap_or_else(|err| panic!("read regenerated {}: {err}", generated.display()));
    assert_eq!(
        actual,
        expected,
        "`{}` diverges from the committed {}",
        generated.display(),
        committed.display()
    );
}

fn replay_readme_commands(spec: &ExampleSpec) {
    let scratch = Scratch::new(spec.dir);
    // One output directory per README command, mirroring the three
    // destination crates.
    let dirs = [
        ("types", scratch.path.join("models")),
        ("client", scratch.path.join("client")),
        ("server", scratch.path.join("server")),
    ];
    for (generate, out_dir) in &dirs {
        run_documented_command(spec, generate, out_dir);
    }

    // Selection discipline: each transport pass writes ONLY its own file.
    assert!(
        !dirs[1].1.join("models.rs").exists(),
        "client pass wrote models.rs"
    );
    assert!(
        !dirs[1].1.join("views.rs").exists(),
        "client pass wrote views.rs"
    );
    assert!(
        !dirs[1].1.join("server.rs").exists(),
        "client pass wrote server.rs"
    );
    assert!(
        !dirs[2].1.join("models.rs").exists(),
        "server pass wrote models.rs"
    );
    assert!(
        !dirs[2].1.join("views.rs").exists(),
        "server pass wrote views.rs"
    );
    assert!(
        !dirs[2].1.join("client.rs").exists(),
        "server pass wrote client.rs"
    );

    let example_root = examples_root().join(spec.dir);
    let generated_for = |file: &str| {
        if file == "client.rs" {
            &dirs[1].1
        } else if file == "server.rs" {
            &dirs[2].1
        } else {
            &dirs[0].1
        }
    };
    for (relative_dir, file) in ARTIFACTS {
        assert_matches_committed(
            &example_root,
            &format!("{relative_dir}/{file}"),
            &generated_for(file).join(file),
        );
    }
}

#[test]
fn kitchen_sink_readme_commands_reproduce_the_committed_artifacts() {
    replay_readme_commands(&EXAMPLES[0]);
}

#[test]
fn large_upload_readme_commands_reproduce_the_committed_artifacts() {
    replay_readme_commands(&EXAMPLES[1]);
}
