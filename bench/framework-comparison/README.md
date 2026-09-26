# Compiled program fixtures

This directory contains Hopper hello/counter workloads and the host-side
`hopper-framework-verifier` crate. The verifier also runs actual-ELF regression
suites for token escrow, treasury, multisig, mint initialization, PDA validation,
and tracked-write behavior.

Run a named suite with its documented ELF environment variables. The suite
checks account state and failures as well as successful execution. The custody
examples link their instructions and test commands from their own READMEs.

The measurement driver is `scripts/bench-framework-comparison.py`. It records
source identity, lockfile and ELF hashes, and validation results. Use a clean
source checkout and `--require-clean` for a source-bound capture. Reused-artifact
runs do not establish a new build's source identity.

Reference fixture attribution is retained in `reference/NOTICE.md`. Historical
measurement data remains research evidence. Product measurements and their
scope are documented in [BENCHMARKS.md](../../BENCHMARKS.md).
