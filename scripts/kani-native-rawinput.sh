#!/usr/bin/env sh
set -eu

repo="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required to run Hopper Kani proofs. Install Rust first." >&2
  exit 1
fi

if ! cargo kani --version >/dev/null 2>&1; then
  echo "cargo-kani is not installed on PATH." >&2
  echo "Install it with: cargo install --locked kani-verifier" >&2
  echo "Then run initial setup with: cargo kani setup" >&2
  exit 1
fi

if [ "${1:-}" = "--setup" ]; then
  cargo kani setup
fi

cargo kani -p hopper-native
