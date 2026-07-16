#!/usr/bin/env sh
set -eu

# Miri lane over Hopper's aliasing core, under Tree Borrows.
#
# Scope: the modules where every `&`/`&mut` handed to user code is
# manufactured from raw loader-input pointers — the segment borrow
# ledger (+ the instruction-ambient touch log), the write-policy gate
# and its ambient lamport store, the native-boundary transmutes, and
# the account borrow registry. These are the surfaces where an aliasing
# bug would be invisible to normal tests and fatal in production.
#
# Deterministic unit tests only: the proptest modules are excluded
# because proptest's failure-persistence writes regression files, which
# Miri's isolation (correctly) refuses; the same generators run at full
# case counts in the normal lanes.
#
# Tree Borrows (`-Zmiri-tree-borrows`) is the aliasing model Quasar's
# miri lane runs under; matching it keeps the comparison honest.
#
# First run builds a Miri sysroot (one-time, a few minutes). Steady
# state is well under a minute.

repo="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required to run the Hopper Miri lane. Install Rust first." >&2
  exit 1
fi

if ! rustup component list --toolchain nightly 2>/dev/null | grep -q '^miri.*(installed)'; then
  echo "Miri is not installed on the nightly toolchain." >&2
  echo "Install it with: rustup component add miri rust-src --toolchain nightly" >&2
  exit 1
fi

MIRIFLAGS='-Zmiri-tree-borrows' cargo +nightly miri test \
  -p hopper-runtime --lib --features touch-map -- \
  segment_borrow::tests \
  segment_borrow::touch_map_tests \
  write_policy::tests \
  context::write_policy_tests \
  native_boundary \
  borrow_registry::tests
