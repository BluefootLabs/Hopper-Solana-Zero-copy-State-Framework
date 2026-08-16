# Release Artifact Evidence

Hopper treats ABI agreement, artifact freshness, and source provenance as
separate release properties. A `.so` that still matches a manifest can be stale
relative to the current source tree, so ABI verification alone is not release
evidence.

## Fresh SBF evidence

The `Solana SBF gates` workflow has two required lanes:

1. the pinned Agave v2.3.13 compatibility lane used by existing reproducible
   builds; and
2. an Agave v4.2.1 forward lane using the standalone
   `cargo-build-sbf 4.1.0` compiler, with representative SBPF v0 and v3 builds.

Each lane first proves that its lane-specific release directory does not exist,
then builds Cicada with a manifest-scoped `cargo build-sbf --sbf-out-dir` into
that isolated directory. It regenerates the manifest, runs the binary ABI
`publish-check`, and writes an attestation containing:

- the exact Git commit and clean-tree results before and after `publish-check`;
- Rust, Cargo, and SBF toolchain versions;
- manifest and ELF SHA-256 hashes and byte sizes; and
- proof that the manifest and binary were created after the build marker; and
- the full release-mode `publish-check` result hash, not only a layout scan.

Before the marker is created, the workflow proves the entire lane-specific
output directory does not exist and records both that fact and the exact output
path in the marker. The workflow then uploads the `.so`, manifest, and
attestation together. A previous `target/deploy` artifact cannot enter this
gate, including when a reproducible rebuild is byte-identical to it.

### Current working-tree diagnostic

On 2026-08-16, the post-hardening and post-CPI-revalidation source was built
twice with local `cargo-build-sbf 4.0.0` / platform-tools 1.53 into the isolated
`target/hopper/release/cicada-repro-a` and
`target/hopper/release/cicada-repro-b` directories. Both builds produced the
same 176,832-byte ELF, byte for byte, with SHA-256
`c21e02caafb4346a402b8f8cf86535790af448411c6fa7d12133b478bbf64150`; the
56,457-byte manifest has SHA-256
`dae0e7817bcf9e60a22fe900c1900dcca85afa8e68d0a74a9c27718a1398105d`.
The strict suite passed 21 host tests, 22 compiled-SBF lifecycle/adversarial
tests, and 3 direct canonical-route tests. A fresh manifest generated beside
the first isolated ELF reproduced the recorded 56,457-byte manifest hash. The
full binary-backed publish check passed all three layout anchors, every
program-shape gate, 160 systems tests, and trybuild after the writable-mint,
native-aware lamport, and typed external/interface post-CPI revalidation
closures.

This remains diagnostic evidence because the source tree is uncommitted. The
local host also lacks a Python runtime, so it did not create the JSON
attestation. The required clean-checkout CI lanes must still regenerate and
upload the binary, manifest, and attestation together. `cargo-build-sbf` also
reports that Cicada's combined `cdylib` and host-test `lib` crate types preclude
LTO; the recorded artifact is release-optimized but must not be described as an
LTO build.

## Cicada manifest-fuzz evidence

Cicada's corrected C3 plan contains 698 cases across 17 structural contract
families with commitment
`d3ada6a2a6559b0dc7722c328fa57138c9e060727fe568784495d5fc2e77f9d2`.
The prior 814-case plan included 114 one-byte probes that left one declared
range but entered an adjacent or overlapping authorized range, plus 2 duplicate
union-boundary probes. The generator now emits an escape only when the byte is
outside the instruction's complete authorized union, with adjacent,
overlapping, nested, and duplicate-range regression coverage.

The focused Rust CI lane rebuilds the non-publish
`hopper-cicada-fuzz-adapter`, runs all 698 cases through the real `hopper fuzz
run` process boundary with `--require-invariant cicada-business-semantics`,
permits no skips, and uploads `cicada-semantic-report.json`. The CLI/process
pipeline rebuilds the plan from the live input manifest and requires its
commitment and cases to match the checked-in plan. The adapter then validates
each request and case against Cicada's live `PROGRAM_MANIFEST` and executes
Cicada's actual private claim, execution, duplicate-meta, mint-delegation,
route-bound, and native/non-native lamport floor guards under seeded host
states. Unknown or modified cases and invariant names fail closed.

This report is host semantic evidence, not 698 compiled-SBF transactions. The
compiled-SBF Cicada lifecycle suite remains the transaction/runtime evidence
lane. The doc-hidden host probe module is excluded from Solana builds. The
current post-CPI-revalidation production ELF is the 176,832-byte artifact and
SHA-256
`c21e02caafb4346a402b8f8cf86535790af448411c6fa7d12133b478bbf64150`
recorded above; C3 does not require an alternate feature-built program.

## Publication train evidence

[`release/publish-order.toml`](../release/publish-order.toml) is the
machine-readable order and version map for all 29 public packages. Validate it
without uploading anything:

```sh
python3 scripts/check-publish-train.py \
  --out target/hopper/release/publish-train.json
```

The command fails when the list differs from Cargo metadata, an internal
normal/build dependency points forward, a version requirement is wrong,
crates.io metadata is incomplete, or a packaged file set lacks its manifest or
README. It runs `cargo package --list` for every public package, which validates
the package boundary without requiring unpublished dependencies to exist in the
registry.

After each earlier dependency has been uploaded and indexed, add
`--registry-dry-run`. This invokes only `cargo publish --dry-run`; it never
uploads a crate. If the registry is still missing an earlier dependency, resume
later with `--start-at <package>`.

## Cross-framework benchmark evidence

The sibling `hopper-bench` repository's `run-current-matrix.ps1` now refuses a
dirty Hopper tree, a dirty benchmark tree, and cached-only binaries by default.
Its successful strict run creates a hash manifest and compressed evidence
archive. The manual benchmark workflow runs from fresh checkouts and uploads the
archive only after provenance reports `publishable: true`.

Diagnostic runs remain available with `-Diagnostic`. `-NoBuild` is deliberately
restricted to diagnostic mode so a cached `.so` cannot become release evidence.
