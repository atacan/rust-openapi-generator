#!/usr/bin/env bash
# Transport-isolation proofs for the example crates
# (DECISIONS.md D-impl-selective-artifacts; runs in CI as the `isolation` job).
#
# Every check inspects ONLY normal dependency edges (`cargo tree -e normal`),
# so dev-dependencies — kitchen-sink/large-upload clients reach the sibling
# server crate for their #[ignore]-gated smoke tests through dev-deps — can
# never mask a production-graph violation.
#
# Proven here:
#   1. both models crates compile NEITHER transport stack;
#   2. both client crates never compile axum;
#   3. kitchen-sink-server never compiles reqwest;
#   4. large-upload-server's DEFAULT build never compiles reqwest (proxy
#      forwarding is an explicit Cargo-feature opt-in);
#   5. POSITIVE CONTROL: `--features proxy` really does put reqwest into the
#      graph — guards the checks above against passing vacuously.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() {
    echo "::error::${1}" >&2
    exit 1
}

# absent <crate> <forbidden-dependency>
absent() {
    local crate="$1" dependency="$2"
    if cargo tree --locked -p "$crate" -e normal --prefix none \
        | grep -q "^${dependency} v"; then
        fail "${crate}: must NOT compile ${dependency}, but it appears in its normal dependency graph"
    fi
    echo "ok: ${crate} does not compile ${dependency}"
}

# present <crate> <required-dependency> [features]
present() {
    local crate="$1" dependency="$2" features="${3:-}"
    local flag=()
    [[ -n "$features" ]] && flag=(--features "$features")
    if ! cargo tree --locked -p "$crate" -e normal "${flag[@]}" --prefix none \
        | grep -q "^${dependency} v"; then
        fail "${crate} (features: ${features:-<default>}): expected ${dependency} in its normal dependency graph"
    fi
    echo "ok: ${crate} (features: ${features:-<default>}) compiles ${dependency}"
}

echo "== shared models crates compile neither transport stack =="
absent kitchen-sink-models axum
absent kitchen-sink-models reqwest
absent large-upload-models axum
absent large-upload-models reqwest

echo "== client crates never compile the Axum stack =="
absent kitchen-sink-client axum
absent large-upload-client axum

echo "== server crates never compile the Reqwest stack by default =="
absent kitchen-sink-server reqwest
absent large-upload-server reqwest

echo "== positive control: proxy feature opts into reqwest explicitly =="
present large-upload-server reqwest proxy

echo "transport isolation verified across all six example crates"
