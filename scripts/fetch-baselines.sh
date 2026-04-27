#!/usr/bin/env bash
# Fetch reference framework checkouts used for benchmarks and parity audits.
#
# Replaces the previously bundled `quasar-master.zip` and `pinocchio-main.zip`
# with reproducible git clones at pinned versions. Output lands in
# `bench/external/`, which is gitignored.
#
# Usage:
#   ./scripts/fetch-baselines.sh           # clone or update both
#   ./scripts/fetch-baselines.sh --clean   # remove and re-clone

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXT="$ROOT/bench/external"

PINOCCHIO_URL="https://github.com/anza-xyz/pinocchio.git"
PINOCCHIO_REF="v0.11.0"

QUASAR_URL="https://github.com/quasar-framework/quasar.git"
QUASAR_REF="master"

if [[ "${1:-}" == "--clean" ]]; then
    rm -rf "$EXT"
fi

mkdir -p "$EXT"

clone_or_update() {
    local name="$1" url="$2" ref="$3" dir="$EXT/$1"
    if [[ -d "$dir/.git" ]]; then
        echo "[$name] updating in $dir"
        git -C "$dir" fetch --tags --quiet origin
        git -C "$dir" checkout --quiet "$ref"
    else
        echo "[$name] cloning $url@$ref -> $dir"
        git clone --quiet "$url" "$dir"
        git -C "$dir" checkout --quiet "$ref"
    fi
}

clone_or_update pinocchio "$PINOCCHIO_URL" "$PINOCCHIO_REF"
clone_or_update quasar    "$QUASAR_URL"    "$QUASAR_REF"

echo
echo "Baselines ready under $EXT"
