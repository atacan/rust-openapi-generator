//! Split-workspace compile proof (DECISIONS.md D-impl-selective-artifacts):
//! the architectural contract behind `--generate types|client|server` +
//! `--types-path`, verified with a real Rust toolchain rather than string
//! matching.
//!
//! Scenario A drives the actual `openapi-to-rust` binary (the exact commands
//! from README's split-crate workflow) to fill a three-crate scratch
//! workspace:
//!
//! ```text
//! ws/
//! ├── api-types/    src/lib.rs + generated models.rs views.rs
//!                   deps: serde + openapi-support WITHOUT features
//! ├── api-client/   src/lib.rs + generated client.rs
//!                   deps: api-types + openapi-support/client (+ reqwest…)
//! └── api-server/   src/lib.rs + generated server.rs
//!                   deps: api-types + openapi-support/server (+ axum…)
//! ```
//!
//! The handwritten `lib.rs` files carry COMPILE-ONLY shared-type identity
//! assertions: the server crate implements the generated `Api` trait with
//! parameters annotated as `api_types::models::Account` /
//! `api_types::views::SyncedRecordWrite`, and the client crate calls
//! `create_account` through a binding typed as
//! `api_types::views::AccountWrite`. If either emitter produced its own
//! structurally duplicated types instead of consuming the shared crate,
//! these signatures would stop compiling.
//!
//! After generation the test runs `cargo check --workspace --locked` plus
//! package-specific dependency proofs:
//!
//! - `cargo tree -i <dep>` must FAIL for `reqwest`/`axum`/`hyper`/`tower`
//!   against `api-types`, for `axum` against `api-client`, and for
//!   `reqwest` against `api-server` (a failed inverted lookup proves the
//!   package is absent from that resolved graph);
//! - `cargo tree -e features` must show `openapi-support` enabled with ONLY
//!   the `client` feature in `api-client` and ONLY `server` in `api-server`.
//!
//! Scenario B compiles an external-mode client under the nested IN-CRATE
//! module path `crate::generated::types`, catching import lines that look
//! plausible but do not resolve in a real module tree.
//!
//! # Reproducibility
//!
//! Both scratch workspaces copy a COMMITTED lockfile
//! (`tests/fixtures/split_workspace/{workspace,nested}.Cargo.lock`) and run
//! Cargo with `--locked`, so compiled dependency versions stay pinned (and
//! within the workspace's MSRV of 1.85). If the handwritten manifests in
//! this file change, regenerate the fixtures:
//!
//! ```text
//! O2R_SPLIT_WS_BOOTSTRAP=1 cargo test -p openapi-to-rust-generator --test split_workspace
//! cp target/tmp/split-workspace-split/Cargo.lock \
//!    crates/generator/tests/fixtures/split_workspace/workspace.Cargo.lock
//! cp target/tmp/split-workspace-nested/Cargo.lock \
//!    crates/generator/tests/fixtures/split_workspace/nested.Cargo.lock
//! ```

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_openapi-to-rust");
const FIXTURE_08: &str = "08_views.yaml";
const FIXTURE_LOCKS_DIR: &str = "tests/fixtures/split_workspace";

fn generator_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures_dir() -> PathBuf {
    generator_root().join("fixtures")
}

/// Absolute path of the workspace-local support crate; the scratch manifests
/// reach it as a path dependency so the proof always tests THIS tree.
fn support_crate_dir() -> String {
    generator_root()
        .join("..")
        .join("support")
        .canonicalize()
        .expect("locate crates/support")
        .to_string_lossy()
        .into_owned()
}

/// Stable target directory SHARED by both scratch workspaces across runs:
/// sources are regenerated deterministically while Cargo reuses artifacts,
/// keeping repeated local runs incremental. Concurrent invocations serialize
/// on Cargo's own build-directory lock.
fn shared_target_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("split-workspace-target")
}

fn cargo_exe() -> PathBuf {
    std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

/// Regeneration escape hatch: with `O2R_SPLIT_WS_BOOTSTRAP=1` the test
/// resolves dependencies freely (no committed lockfile, no `--locked`) so
/// fresh fixture lockfiles can be captured from
/// `<target>/tmp/split-workspace-*/Cargo.lock`. Normal runs must NOT set it.
fn bootstrap_mode() -> bool {
    std::env::var_os("O2R_SPLIT_WS_BOOTSTRAP").is_some()
}

/// Version pins applied ONLY during bootstrap, in order, keeping the
/// captured fixture lockfiles within the workspace MSRV of 1.85 (main spec
/// §3.1): Cargo's default resolver still selects newer transitive versions
/// whose OWN `rust-version` exceeds our floor (`url 2.5.8`, `idna_adapter
/// 1.2.2` and the ICU 2.x line need rustc 1.86–1.88), and `cargo check`
/// rejects such a plan. Entries are `(package, precise_version)`; a later
/// entry may depend on an earlier one. Extend this list if a future
/// regeneration trips over another crate.
const BOOTSTRAP_MSRV_PINS: &[(&str, &str)] = &[
    ("url", "2.5.4"),
    ("idna", "1.0.3"),
    ("idna_adapter", "1.2.0"),
];

// ----------------------------------------------------------------------
// Scratch-workspace plumbing
// ----------------------------------------------------------------------

struct ScratchWorkspace {
    root: PathBuf,
    /// Basename of the committed lockfile copied in before every check.
    lock_fixture_name: &'static str,
}

impl ScratchWorkspace {
    /// Creates (wiping any previous run of) `<target>/tmp/<name>`.
    fn new(name: &str, lock_fixture_name: &'static str) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create scratch workspace root");
        Self {
            root,
            lock_fixture_name,
        }
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().expect("relative path has parent"))
            .expect("create scratch subdirectory");
        std::fs::write(&path, contents).expect("write scratch file");
    }

    fn lock_fixture_path(&self) -> PathBuf {
        generator_root()
            .join(FIXTURE_LOCKS_DIR)
            .join(self.lock_fixture_name)
    }

    /// Installs the committed fixture lockfile unless running under
    /// [`bootstrap_mode`] (which leaves resolution free for regeneration).
    fn install_lock(&self) -> bool {
        if bootstrap_mode() {
            eprintln!(
                "BOOTSTRAP: {} resolves dependencies WITHOUT its committed \
                 lockfile",
                self.root.display()
            );
            return false;
        }
        let source = self.lock_fixture_path();
        let contents = std::fs::read_to_string(&source).unwrap_or_else(|err| {
            panic!(
                "read committed lockfile {}: {err} — regenerate it with \
                 `O2R_SPLIT_WS_BOOTSTRAP=1 cargo test -p \
                 openapi-to-rust-generator --test split_workspace`",
                source.display()
            )
        });
        self.write("Cargo.lock", &contents);
        true
    }

    /// Runs the real CLI binary with `--output-dir` inside this workspace.
    fn generate(&self, args: &[&str], out_dir_relative: &str, what: &str) {
        let output = Command::new(BIN)
            .args(args)
            .arg("--output-dir")
            .arg(self.root.join(out_dir_relative))
            .output()
            .expect("spawn openapi-to-rust");
        assert!(
            output.status.success(),
            "{what}: generation must succeed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    /// `cargo check` pinned through the fixture lockfile (`--locked`) except
    /// under [`bootstrap_mode`], which first applies [`BOOTSTRAP_MSRV_PINS`]
    /// so the captured lockfile stays within the workspace MSRV.
    fn check(&self, extra_args: &[&str], what: &str) {
        let locked = self.install_lock();
        if !locked {
            for (package, version) in BOOTSTRAP_MSRV_PINS {
                expect_success(
                    &self.cargo(&["update", "-p", package, "--precise", version]),
                    &format!("bootstrap MSRV pin `{package} = {version}`"),
                );
            }
        }
        let mut args: Vec<&str> = vec!["check"];
        args.extend_from_slice(extra_args);
        if locked {
            args.push("--locked");
        }
        expect_success(&self.cargo(&args), what);
    }

    fn cargo(&self, args: &[&str]) -> Output {
        Command::new(cargo_exe())
            .args(args)
            .current_dir(&self.root)
            .env("CARGO_TARGET_DIR", shared_target_dir())
            .env("CARGO_TERM_COLOR", "never")
            .output()
            .expect("spawn cargo")
    }
}

fn expect_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Proves `package` does NOT depend on `dependency`: an inverted `cargo tree`
/// lookup fails exactly when the resolved graph contains no such node.
fn assert_dependency_absent(workspace: &ScratchWorkspace, package: &str, dependency: &str) {
    let output = workspace.cargo(&["tree", "--locked", "-p", package, "-i", dependency]);
    assert!(
        !output.status.success(),
        "`{package}` must NOT depend on `{dependency}`, but it is in the \
         resolved graph:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}

/// Asserts which `openapi-support` features are enabled for `package`:
/// `wanted` must be on and `forbidden` must stay off.
fn assert_support_feature_isolation(
    workspace: &ScratchWorkspace,
    package: &str,
    wanted: &str,
    forbidden: &str,
) {
    let output = workspace.cargo(&["tree", "--locked", "-e", "features", "-p", package]);
    expect_success(&output, &format!("`cargo tree -e features -p {package}`"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let feature_lines: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("openapi-support feature \""))
        .collect();
    assert!(
        feature_lines
            .iter()
            .any(|line| line.contains(&format!("feature \"{wanted}\""))),
        "`{package}` must enable openapi-support/{wanted}\nfeature lines: \
         {feature_lines:#?}"
    );
    assert!(
        !feature_lines
            .iter()
            .any(|line| line.contains(&format!("feature \"{forbidden}\""))),
        "`{package}` must NOT enable openapi-support/{forbidden} (transport \
         stacks must stay isolated)\nfeature lines: {feature_lines:#?}"
    );
}

// ----------------------------------------------------------------------
// Handwritten manifest / wiring templates (fixture content, NOT generator
// output — normal generation never writes Cargo.toml files)
// ----------------------------------------------------------------------

fn workspace_root_manifest() -> String {
    "\
[workspace]
resolver = \"2\"
members = [\"api-types\", \"api-client\", \"api-server\"]
"
    .to_owned()
}

fn types_crate_manifest() -> String {
    format!(
        "\
[package]
name = \"api-types\"
version = \"0.1.0\"
edition = \"2021\"

[dependencies]
serde = {{ version = \"1\", features = [\"derive\"] }}
openapi-support = {{ path = {support:?} }}
",
        support = support_crate_dir()
    )
}

fn types_lib_rs() -> String {
    "\
//! Scratch fixture crate: normal module wiring over the GENERATED modules.

pub mod models;
pub mod views;
"
    .to_owned()
}

fn client_crate_manifest() -> String {
    format!(
        "\
[package]
name = \"api-client\"
version = \"0.1.0\"
edition = \"2021\"

[dependencies]
api-types = {{ path = \"../api-types\" }}
openapi-support = {{ path = {support:?}, features = [\"client\"] }}
reqwest = {{ version = \"0.12\", default-features = false, features = [\"json\", \"multipart\", \"stream\"] }}
http = \"1\"
mime = \"0.3\"
serde = {{ version = \"1\", features = [\"derive\"] }}
serde_json = \"1\"
",
        support = support_crate_dir()
    )
}

fn client_lib_rs() -> String {
    "\
//! Scratch fixture crate: wiring around the GENERATED client module plus a
//! compile-only proof that it consumes THE shared view type from the
//! `api_types` crate rather than a structurally duplicated local copy — the
//! explicit parameter annotation fails to compile against anything else.

pub mod client;

/// Compile-only: never called, so nothing but type-checking ever runs it.
/// The error type is the support crate's `ClientError`, which the generated
/// module imports but does not re-export.
#[allow(dead_code)]
async fn create_account_takes_the_shared_write_view(
    client: &client::Client,
    write: api_types::views::AccountWrite,
) -> Result<client::CreateAccountResponse, openapi_support::client_error::ClientError> {
    client.create_account(&write).await
}
"
    .to_owned()
}

fn server_crate_manifest() -> String {
    format!(
        "\
[package]
name = \"api-server\"
version = \"0.1.0\"
edition = \"2021\"

[dependencies]
api-types = {{ path = \"../api-types\" }}
openapi-support = {{ path = {support:?}, features = [\"server\"] }}
axum = \"0.8\"
async-trait = \"0.1\"
bytes = \"1\"
http = \"1\"
mime = \"0.3\"
serde = {{ version = \"1\", features = [\"derive\"] }}
serde_json = \"1\"
",
        support = support_crate_dir()
    )
}

fn server_lib_rs() -> String {
    "\
//! Scratch fixture crate: wiring around the GENERATED server module plus a
//! compile-only identity proof — implementing the generated `Api` trait with
//! parameters explicitly typed from the `api_types` crate compiles ONLY when
//! the trait itself is declared against those exact shared types.

pub mod server;

#[allow(dead_code)]
struct DemoApi;

#[async_trait::async_trait]
impl server::Api for DemoApi {
    async fn create_account(
        &self,
        body: api_types::models::Account,
    ) -> server::CreateAccountResponse {
        // Lossless write-view reconstruction pins BOTH the model and its
        // directional view to the single shared definition.
        let _write = api_types::views::AccountWrite::from(&body);
        server::CreateAccountResponse::Created201(api_types::views::AccountRead::from(
            &body,
        ))
    }

    async fn list_audit_entries(
        &self,
        id: String,
    ) -> server::ListAuditEntriesResponse {
        let _ = id;
        unreachable!(\"compile-only proof: never invoked\")
    }

    async fn sync_record(
        &self,
        body: api_types::views::SyncedRecordWrite,
    ) -> server::SyncRecordResponse {
        let _ = body;
        unreachable!(\"compile-only proof: never invoked\")
    }
}
"
    .to_owned()
}

fn nested_app_manifest() -> String {
    format!(
        "\
[package]
name = \"nested-app\"
version = \"0.1.0\"
edition = \"2021\"
# Own workspace root: the scratch tree sits under the generator's target/
# directory, otherwise Cargo would adopt the OUTER repository workspace.
[workspace]

[dependencies]
openapi-support = {{ path = {support:?}, features = [\"client\"] }}
reqwest = {{ version = \"0.12\", default-features = false, features = [\"json\", \"multipart\", \"stream\"] }}
http = \"1\"
mime = \"0.3\"
serde = {{ version = \"1\", features = [\"derive\"] }}
serde_json = \"1\"
",
        support = support_crate_dir()
    )
}

fn nested_app_lib_rs() -> String {
    "\
//! Scratch fixture crate: the shared types AND the client live INSIDE one
//! application crate under `crate::generated::types`, proving the nested
//! in-crate spelling of `--types-path` resolves in a real module tree.

pub mod generated {
    pub mod types {
        pub mod client;
        pub mod models;
        pub mod views;
    }
}
"
    .to_owned()
}

// ----------------------------------------------------------------------
// Scenario A — three-crate split workspace
// ----------------------------------------------------------------------

#[test]
fn split_workspace_compiles_and_isolates_transports() {
    let document = fixtures_dir().join(FIXTURE_08);
    let document = document.to_str().expect("utf-8 fixture path");

    let ws = ScratchWorkspace::new("split-workspace-split", "workspace.Cargo.lock");
    ws.write("Cargo.toml", &workspace_root_manifest());

    // ── api-types: models.rs + views.rs via `--generate types` ──────────
    ws.write("api-types/Cargo.toml", &types_crate_manifest());
    ws.write("api-types/src/lib.rs", &types_lib_rs());
    ws.generate(
        &[document, "--generate", "types"],
        "api-types/src",
        "types-only generation into its own crate",
    );

    // ── api-client: external shared types at crate path `api_types` ─────
    ws.write("api-client/Cargo.toml", &client_crate_manifest());
    ws.write("api-client/src/lib.rs", &client_lib_rs());
    ws.generate(
        &[
            document,
            "--generate",
            "client",
            "--types-path",
            "api_types",
        ],
        "api-client/src",
        "client generation with external types",
    );

    // ── api-server: same shared types crate ─────────────────────────────
    ws.write("api-server/Cargo.toml", &server_crate_manifest());
    ws.write("api-server/src/lib.rs", &server_lib_rs());
    ws.generate(
        &[
            document,
            "--generate",
            "server",
            "--types-path",
            "api_types",
        ],
        "api-server/src",
        "server generation with external types",
    );

    for (member, files) in [
        ("api-types/src", vec!["models.rs", "views.rs"]),
        ("api-client/src", vec!["client.rs"]),
        ("api-server/src", vec!["server.rs"]),
    ] {
        for file in files {
            assert!(
                ws.root.join(member).join(file).is_file(),
                "{member}/{file} must exist after generation"
            );
        }
    }

    ws.check(
        &["--workspace"],
        "cargo check --workspace over the generated split workspace",
    );

    // Dependency edges: both transport crates consume the LOCAL types crate.
    for package in ["api-client", "api-server"] {
        let tree = ws.cargo(&["tree", "--locked", "-p", package]);
        expect_success(&tree, &format!("cargo tree -p {package}"));
        let stdout = String::from_utf8_lossy(&tree.stdout);
        assert!(
            stdout
                .lines()
                .any(|line| line.starts_with("├── api-types v")
                    || line.starts_with("└── api-types v")),
            "{package} -> api-types edge missing:\n{stdout}"
        );
    }

    // Transport isolation by package absence (inverted tree lookups FAIL).
    for dependency in ["reqwest", "axum", "hyper", "tower"] {
        assert_dependency_absent(&ws, "api-types", dependency);
    }
    assert_dependency_absent(&ws, "api-client", "axum");
    assert_dependency_absent(&ws, "api-server", "reqwest");

    // Transport isolation by SUPPORT FEATURE (tokio appears in both stacks,
    // so the feature graph — not package presence — carries the proof).
    assert_support_feature_isolation(&ws, "api-client", "client", "server");
    assert_support_feature_isolation(&ws, "api-server", "server", "client");
}

// ----------------------------------------------------------------------
// Scenario B — nested in-crate module path (`crate::generated::types`)
// ----------------------------------------------------------------------

#[test]
fn nested_in_crate_module_path_compiles() {
    let document = fixtures_dir().join(FIXTURE_08);
    let document = document.to_str().expect("utf-8 fixture path");

    let ws = ScratchWorkspace::new("split-workspace-nested", "nested.Cargo.lock");
    ws.write("Cargo.toml", &nested_app_manifest());
    ws.write("src/lib.rs", &nested_app_lib_rs());
    ws.generate(
        &[document, "--generate", "types"],
        "src/generated/types",
        "types into the nested module directory",
    );
    ws.generate(
        &[
            document,
            "--generate",
            "client",
            "--types-path",
            "crate::generated::types",
        ],
        "src/generated/types",
        "client with nested in-crate types path",
    );

    for file in ["models.rs", "views.rs", "client.rs"] {
        assert!(
            ws.root.join("src/generated/types").join(file).is_file(),
            "src/generated/types/{file} must exist after generation"
        );
    }

    ws.check(&[], "cargo check over the nested-module layout");
}
