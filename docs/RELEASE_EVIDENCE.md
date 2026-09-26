# Release Artifact Evidence

Hopper treats declared-interface binding, artifact freshness, and source provenance as
separate release properties. A `.so` that still matches a manifest can be stale
relative to the current source tree, so interface verification alone is not release
evidence.

## Fresh SBF evidence

The `Solana SBF gates` workflow has two required lanes:

1. the pinned Agave v2.3.13 compatibility lane used by existing reproducible
   builds; and
2. an Agave v4.2.1 forward lane using the standalone
   `cargo-build-sbf 4.1.0` compiler, with representative SBPF v0 and v3 builds.

Each lane first proves that its lane-specific release directory does not exist,
then builds Cicada with a manifest-scoped `cargo build-sbf --sbf-out-dir` into
that isolated directory. It regenerates the manifest, runs the ELF
interface-binding `publish-check`, and writes an attestation containing:

- the exact Git commit and clean-tree results before and after `publish-check`;
- Rust, Cargo, and SBF toolchain versions;
- manifest and ELF SHA-256 hashes and byte sizes;
- proof that the manifest and binary were created after the build marker; and
- the full release-mode `publish-check` result hash, including the exact
  versioned interface commitment rather than only a layout scan.

Before the marker is created, the workflow proves the entire lane-specific
output directory does not exist and records both that fact and the exact output
path in the marker. The workflow then uploads the `.so`, manifest, and
attestation together. A previous `target/deploy` artifact cannot enter this
gate, including when a reproducible rebuild is byte-identical to it.

### Clean 2026-09-23 local attestation

The [Cicada evidence archive](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/cicada-sbf-2026-09-23)
records a fresh isolated build from clean commit `685deff6`, using
`cargo-build-sbf 4.1.0`, platform-tools 1.54 and SBPF v0. Its 157,672-byte ELF
has SHA-256 `09d71f5ada51cce3946545a242a63f00037c1bcd442490d9bae9a7459342e02b`.
The exact interface-binding publish check passed, and all 23 compiled lifecycle
tests passed against that ELF and the recorded route fixtures. The archive
contains the manifest, attestation and command transcripts with checksums.

This is local build and execution evidence. Cicada was not deployed by this
run, and this does not establish third-party AMM integration or authenticated
upgradeable-route artifact policy. Hosted workflows for this source could not
start because GitHub reported an account billing lock; their status is not a
passing CI result.

### Historical 2026-09-06 working-tree diagnostic

After the Cicada revision-overflow hardening, two isolated builds with
`cargo-build-sbf 4.1.0`, platform-tools 1.54, and Rust/Cargo 1.96.0 produced
the same artifact as `target/deploy/hopper_cicada.so`:

- ELF: 165,944 bytes, SHA-256
  `7ee1247f704b6feb42cfc499b9bcdb30b79b4f83bc4de599cbe389b685c2defb`;
- `.text`: 152,528 bytes;
- generated manifest: 57,783 bytes, SHA-256
  `9ac3ca294376909200030d6794d6afc343de7076440bbd741bb10d87c0dcb241`;
- interface commitment:
  `98a0eaf0cf78b13881426c0894e4fd521b7250e7f419389b180662a3b08d1976`.

The manifest grew by 93 bytes when its three layouts began emitting the
explicit `hasDynamicTail: false` field. A fresh SBF rebuild remained
byte-identical, and `hopper verify --strict --release` found the unchanged
commitment and all three layout anchors. `hopper publish-check --full` passed.
Cicada's 25 host tests and
23 compiled lifecycle tests also passed. This remains diagnostic evidence:
the shared tree had hundreds of pre-existing status entries, the generated manifest was untracked at
the start, and this Windows host had no Python runtime for the normalized
attestation script. It must not replace the required clean committed CI
attestation. The build also repeats the warning that a combined `cdylib` +
`lib` crate type prevents LTO.

### Historical working-tree diagnostic

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
then-current binary-backed publish check passed all three layout anchors, every
program-shape gate, 160 systems tests, and trybuild after the writable-mint,
native-aware lamport, and typed external/interface post-CPI revalidation
closures.

This is retained only as historical diagnostic evidence. That verifier checked
raw layout-anchor presence; the artifact predates the versioned ELF
interface-binding record and cannot satisfy the current `--release` gate. It
also predates the later Cicada repeated-writable-route-alias admission fix and
was not built from a clean committed checkout, so it does not attest the
current source. The local
host also lacked a Python runtime, so it did not create the JSON attestation.
The required clean-checkout CI lanes must still regenerate and upload the
current binary, manifest, and attestation together.
`cargo-build-sbf` also reported that Cicada's combined `cdylib` and host-test
`lib` crate types preclude LTO; the recorded artifact is release-optimized but
must not be described as an LTO build.

### Pre-binding clean local attestation

After the repeated-writable-route-alias closure was committed at
`3dfceba4a8f7b98ff0e355aa1965e7e9509023b2`, a separate clean checkout built
Cicada with `cargo-build-sbf 4.1.0` and platform-tools v1.54 into an isolated
output directory proven absent before the build. The resulting 167,680-byte ELF
has SHA-256
`ac8ec1d76b4f85a5515dc446536bafccabe1c62b8ff46a13a662971b785da0e9`,
and the generated 56,457-byte manifest has SHA-256
`dae0e7817bcf9e60a22fe900c1900dcca85afa8e68d0a74a9c27718a1398105d`.
The attestation records a clean tree before and after the then-current
binary-backed `hopper publish-check --full`; all 3 layout anchors, program-shape,
documentation, feature, token, client, fuzz, artifact, Solana-shape, 160
systems-test, and trybuild gates passed. The same source passed 21 of 21 host
tests, 22 of 22 strict compiled-SBF lifecycle/adversarial tests, and targeted
all-target clippy with warnings denied before the clean build.

The normalized attestation is retained at
[`audit/cicada-sbf-attestation-2026-08-16.json`](../audit/cicada-sbf-attestation-2026-08-16.json).
This closes binary freshness and legacy layout-anchor presence for commit
`3dfceba`; it does not close the current release-interface binding because that
artifact predates the versioned record. The required pinned Agave v2.3.13 and
Agave v4.2.1-forward CI lanes must rebuild the final source and retain their
ELF, generated manifest, exact interface commitment, and attestation together
before release.

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
lane. The doc-hidden host probe module is excluded from Solana builds. C3 does
not require an alternate feature-built program.

## Publication train evidence

The [September 23 publication archive](../audit/registry-publication-2026-09-23/README.md)
records all 29 indexed packages, downloaded archive checksums, per-package
clean source commits, registry dry runs, and compiled execution of a fresh
registry-only consumer. The included final-source Cicada attestation reproduces
the 157,672-byte executable described above.

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
uploads a crate. Under pinned Cargo 1.96, publish dry runs stage their archive
in `target/package/tmp-crate`; this is separate from `cargo package` output.
The verifier removes the expected staged file before the command and hashes
the newly produced archive, preventing stale package-preflight bytes from
being recorded as registry dry-run evidence.

If the registry is still missing an earlier dependency, resume
later with `--start-at <package>`.

## Program execution evidence

Dated program measurements and reproduction requirements are recorded in
[program measurements](../BENCHMARKS.md). Validate exact account state, balances,
refusals, rollback, and the deployed artifact when assessing a release.
