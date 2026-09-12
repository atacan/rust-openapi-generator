# AGENTS.md — instructions for coding agents working in this repo

## Before committing, pushing, or opening/updating a PR

CI (`.github/workflows/ci.yml`) gates every PR. Run these locally first —
never push while any of them fail:

1. Formatting:
   - `cargo fmt --check`
   - If it fails, run `cargo fmt`, then re-check.
2. Lints (warnings denied):
   - `cargo clippy --workspace --all-targets --locked -- -D warnings`
   - Plus the feature-matrix variants from the `clippy` job in `ci.yml`:
     - `cargo clippy -p openapi-support --all-targets --features client,server --locked -- -D warnings`
     - `cargo clippy -p openapi-support --all-targets --features client,client-gzip --locked -- -D warnings`
     - `cargo clippy -p large-upload-server --all-targets --features proxy --locked -- -D warnings`
3. Tests:
   - `cargo test --workspace --locked`
   - Do NOT run the `#[ignore]`-gated long-haul proofs (excluded by default; keep it that way).

## Notes

- `ci.yml` is the source of truth for the gate list; if it changes, mirror the change here.
- MSRV is Rust 1.85 (`rust-version` in the workspace `Cargo.toml`); avoid APIs or lints newer than that.
- Generation must stay byte-deterministic (spec §50 test 39): never hand-edit
  `crates/generator/tests/snapshots`, `crates/conformance`, or
  `examples/kitchen-sink` / `examples/large-upload` — the `determinism` job
  regenerates them and fails on any diff.
