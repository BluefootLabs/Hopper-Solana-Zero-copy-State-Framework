# Hopper 0.6 candidate validation — October 9

This archive records the 0.6.0 candidate's implementation checks and measured
execution scope. It is not a crates.io publication receipt, clean hosted-CI
attestation, independent security review, or proof that every framework feature
has executed on devnet. The published framework remains 0.5.0.

## Changes exercised

Instruction metadata follows the decoder through type aliases. Owned bounded
vectors require an exact element width; unrepresentable vector layouts and
manifest-size overflow are rejected at compile time. Bounded instruction
strings reject malformed UTF-8 before account admission. Variable-length
account-tail codecs retain their existing behavior.

Generated Rust, TypeScript, and Python clients execute independent wire-byte,
capacity, element-width and privilege checks. Python also exercises owned
aliases and rejects invalid UTF-8 and wrong fixed widths. C, Go and Kotlin
bounded builders explicitly report unsupported encoding; their generated
refusal paths are inspected by the Rust integration test, not executed by a
native compiler in this pass. Fixed-only builders retain their prior contract.

The API release planner now uses the repository's Rust toolchain for both the
registry baseline and workspace. This avoids false changes caused by comparing
different rustdoc versions. The strict plan targets 25 packages at 0.6.0 and
keeps four unchanged support packages at their published versions. The package
diagnostic passes all 29 metadata, dependency-order and archive-closure checks.
It permits a dirty tree and must not be presented as final publication approval.

## Results

| Check | Recorded outcome |
| --- | --- |
| Workspace tests | 2,508 reported passes, zero failures, 281 ignored |
| Clippy | Entire workspace, all targets, warnings denied: passed |
| Formatting | Passed |
| Dependency audit | Zero known vulnerabilities, four unmaintained-package notices |
| Library API locks | All 28 match |
| SBF v3 matrix | All 53 deployable packages build; no stack-frame diagnostic |
| Borrowed-batch VM | 1,626 instruction cases on each of v0 and v3; 3,252 total |
| Public devnet | 67 finalized transactions: 3 state creations, 9 successful batches, 55 expected refusals |
| Generated clients | Rust, TypeScript and Python execution checks passed |
| Unsafe contracts | 325 Rust files checked; 851 mapped sites, zero unjustified sites |
| Website | 63 served pages, 5,698 internal links/anchors, zero errors |

Ignored tests are not executions. The unsafe map records proof obligations and
test attribution; it does not establish soundness or independent review.

Devnet program:
[AeSdZW4tFTjHhisMEisXVXDMaq96Aktnzy5nSgwHgS8u](https://explorer.solana.com/address/AeSdZW4tFTjHhisMEisXVXDMaq96Aktnzy5nSgwHgS8u?cluster=devnet).
The v3 ELF is 6,592 bytes, SHA-256
`56c5d135c10c1b7d2b6ea03bf9555343ed412f51800227ce411e65bc176a4878`.
The v0 build is 7,472 bytes. Live suites verify the deployed binary before and
after execution, exact errors, return-data provenance and bytes, and complete
fixture-state preservation on refused writes. Each dispatch path starts with
fresh state and ends with count 34 and total 173.

| Fixture workload | Manual borrowed CU | Generated borrowed CU | Owned-alias CU |
| --- | ---: | ---: | ---: |
| Empty | 226 | 255 | 498 |
| Two elements | 269 | 301 | 561 |
| Maximum 32 elements | 871 | 903 | 1,401 |

These are whole-instruction observations, not framework rankings. Borrowed
paths verify that element views reference the original instruction buffer.
The owned-alias path copies its arguments and has a different validation
workload. The fixture accepts any signer and is not a custody example.

## Provenance and diagnostics

`current-source.json` and `source-inputs.tar.gz` retain final candidate inputs.
Quality receipts bind those inputs to the command, result and log bytes.
`source.json` records the source used by live execution. Two subsequent test
cleanups are identified in `post-devnet-change.json`: the macro expansion test
now expects decoder-derived metadata, and two unnecessary fixture clones were
removed. The complete matrix rebuild produces the exact live fixture binary.
`post-matrix-change.json` proves the later clone cleanup by reversing its two
substitutions and checking the original source hash. No on-chain source was
changed by that cleanup.

`all-programs-v3/` retains all 53 logs and public binaries. Reports preserve
their original ignored output paths; archived package directories preserve
file names and hashes. `binary-continuity.json` compares October 8 artifacts:
39 binaries are identical and 14 changed. Historical devnet results must not
be applied to changed binaries. The 22 baseline packages without named live
suites, and untested handler/feature combinations, remain outside complete
live verification.

`diagnostics/` retains the malformed-UTF-8 regression before and after its
implementation fix, the obsolete macro-test failure, and Clippy's two fixture
clone findings. Passing final receipts are separate. The compiled VM runner's
hash and exact tested ELF hashes are recorded in `compiled-vm.json`.

`dependency-database.json` verifies every file in the RustSec snapshot at
revision `550efd3d587a29b2e2c2b21b17a440da4fede999`, observed October 9 UTC.
`peer-heads.json` records current Pinocchio, Pina, Quasar and Anchor v2 heads;
they match the earlier file-level comparison. This is not exhaustive feature
parity or an exclusive zero-copy claim.

`website-commit.json` identifies pushed commit
`418d4f706d311e3607bf683a91f03cac1ac417d3` and verifies it against the tested
input inventory. Desktop and mobile Chromium checks cover keyboard controls,
malformed-input interaction, navigation, reduced motion, overflow and page
errors; screenshots are retained in `website-qa/`. Vercel reports deployment
success, and the home and migration pages return HTTP 200 on the public domain.

Private keys and deployment recovery transcripts are excluded. Existing sealed
archives remain unchanged. Clean hosted SBF CI remains a release requirement;
the prior jobs could not start because of a GitHub account billing lock.
Independent review remains unstarted. BLAKE3 availability, the observed devnet
EpochSchedule inconsistency, transaction-v1 authoring, and complete live feature
coverage remain explicitly tracked rather than inferred from build success.
