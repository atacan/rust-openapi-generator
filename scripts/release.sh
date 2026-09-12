#!/usr/bin/env bash
# Prepare and publish a stable oapi-to-rust patch or minor release.
#
# Usage: scripts/release.sh patch|minor
#
# The script makes the release commit and annotated tag only after every local
# release gate passes. Pushing the tag starts .github/workflows/release.yml,
# which publishes the binaries and updates the Homebrew tap.
set -euo pipefail

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  printf 'usage: %s patch|minor\n' "${0##*/}" >&2
  exit 2
}

[[ $# -eq 1 ]] || usage
case "$1" in
  patch|minor) bump="$1" ;;
  *) usage ;;
esac

cd "$(dirname "$0")/.."

# Untracked files are harmless because the commit below names every file it
# stages explicitly. Tracked changes are not: a release must start from an
# already-reviewed main branch.
[[ -z "$(git status --porcelain --untracked-files=no)" ]] \
  || fail 'commit or stash tracked changes before releasing'

git fetch origin main --tags
[[ "$(git branch --show-current)" == main ]] || fail 'releases must start on main'
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] \
  || fail 'local main is not identical to origin/main'

current_version="$(awk -F'"' '/^version = / { print $2; exit }' crates/generator/Cargo.toml)"
[[ "$current_version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] \
  || fail "crates/generator/Cargo.toml has an invalid version: ${current_version}"
current_tag="v${current_version}"
git rev-parse --verify --quiet "refs/tags/${current_tag}" >/dev/null \
  || fail "current version tag ${current_tag} does not exist"
git merge-base --is-ancestor "$current_tag" HEAD \
  || fail "current version tag ${current_tag} is not an ancestor of main"

IFS=. read -r major minor patch <<<"$current_version"
case "$bump" in
  patch) next_version="${major}.${minor}.$((patch + 1))" ;;
  minor) next_version="${major}.$((minor + 1)).0" ;;
esac
next_tag="v${next_version}"
git rev-parse --verify --quiet "refs/tags/${next_tag}" >/dev/null \
  && fail "release tag ${next_tag} already exists"

release_notes="$(git log --no-merges --format='- %s' "${current_tag}..HEAD")"
[[ -n "$release_notes" ]] || fail "no commits exist after ${current_tag}; refusing an empty release"

CURRENT_VERSION="$current_version" NEXT_VERSION="$next_version" \
RELEASE_DATE="$(date +%F)" RELEASE_NOTES="$release_notes" python3 - <<'PY'
from pathlib import Path
import os

current = os.environ["CURRENT_VERSION"]
next_version = os.environ["NEXT_VERSION"]
release_date = os.environ["RELEASE_DATE"]
notes = os.environ["RELEASE_NOTES"]

cargo_toml = Path("crates/generator/Cargo.toml")
contents = cargo_toml.read_text()
old = f'version = "{current}"'
if contents.count(old) != 1:
    raise SystemExit(f"expected exactly one {old!r} in {cargo_toml}")
cargo_toml.write_text(contents.replace(old, f'version = "{next_version}"', 1))

changelog = Path("CHANGELOG.md")
contents = changelog.read_text()
marker = "## Unreleased\n\n"
if marker not in contents:
    raise SystemExit(f"expected {marker!r} in {changelog}")
entry = f"## v{next_version} — {release_date}\n\n### Changes\n\n{notes}\n\n"
changelog.write_text(contents.replace(marker, marker + entry, 1))
PY

# Regenerate the package's lockfile entry before invoking the locked gates.
cargo check --package openapi-to-rust-generator

echo '== formatting =='
cargo fmt --check
echo '== workspace clippy =='
cargo clippy --workspace --all-targets --locked -- -D warnings
echo '== feature-matrix clippy =='
cargo clippy -p openapi-support --all-targets --features client,server --locked -- -D warnings
cargo clippy -p openapi-support --all-targets --features client,client-gzip --locked -- -D warnings
cargo clippy -p large-upload-server --all-targets --features proxy --locked -- -D warnings
echo '== workspace tests =='
cargo test --workspace --locked
git diff --check

expected_files=$'CHANGELOG.md\nCargo.lock\ncrates/generator/Cargo.toml'
actual_files="$(git diff --name-only | LC_ALL=C sort)"
[[ "$actual_files" == "$expected_files" ]] \
  || fail "release preparation changed unexpected files:\n${actual_files}"

printf '\nPreparing %s from %s:\n' "$next_tag" "$current_tag"
git diff -- CHANGELOG.md Cargo.lock crates/generator/Cargo.toml

git add CHANGELOG.md Cargo.lock crates/generator/Cargo.toml
git commit -m "chore: prepare ${next_tag} release"
git push origin main

# Do not create the release tag until the remote branch is proven to contain
# every release-controlled version update. This protects against a partial or
# misdirected push and keeps the tag, binary version, lockfile, and changelog
# tied to the same commit on GitHub.
git fetch origin main
remote_generator_version="$(
  git show origin/main:crates/generator/Cargo.toml |
    awk -F'"' '/^version = / { print $2; exit }'
)"
[[ "$remote_generator_version" == "$next_version" ]] \
  || fail "origin/main generator version is ${remote_generator_version:-missing}, expected ${next_version}"
remote_lock_version="$(
  git show origin/main:Cargo.lock |
    awk -v package='openapi-to-rust-generator' '
      $0 == "name = \"" package "\"" { found = 1; next }
      found && /^version = / { gsub(/\"/, "", $3); print $3; exit }
    '
)"
[[ "$remote_lock_version" == "$next_version" ]] \
  || fail "origin/main lockfile version is ${remote_lock_version:-missing}, expected ${next_version}"
git show origin/main:CHANGELOG.md | grep -Fqx "## ${next_tag} — $(date +%F)" \
  || fail "origin/main changelog has no entry for ${next_tag}"

git tag -a "$next_tag" -m "oapi-to-rust ${next_tag}"
git push origin "$next_tag"

printf '\nPublished %s. Follow the release at:\n' "$next_tag"
printf 'https://github.com/atacan/rust-openapi-generator/actions/workflows/release.yml\n'
