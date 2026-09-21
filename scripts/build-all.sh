#!/usr/bin/env bash
# Builds every tool in the workspace in release mode and copies the
# resulting binaries into bin/, so tools/<name> and bin/<name> always
# stay in sync as new tools are added under tools/.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

cargo build --release --workspace

mkdir -p bin

for tool_dir in tools/*/; do
    name="$(basename "$tool_dir")"
    cp "target/release/$name" "bin/$name"
    echo "Copied $name to bin/$name"
done
