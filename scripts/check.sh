#!/usr/bin/env bash
# Runs every check CI runs: formatting, lints and tests.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== cargo fmt =="
cargo fmt --all -- --check

echo "== cargo clippy =="
cargo clippy --workspace --all-targets -- -D warnings

echo "== cargo test =="
cargo test --workspace

echo "all checks passed"
