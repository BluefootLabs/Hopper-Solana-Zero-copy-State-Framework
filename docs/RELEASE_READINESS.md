# Release readiness

Hopper 0.5.0 is published. The checked borrowed-argument APIs and receipt-backed
readiness checks are unreleased. This page describes the current working tree;
it does not attest a clean committed release candidate.

The next package train targets **0.6.0**, including breaking generated-client
behavior described in the [migration guide](MIGRATION_0_6.md). Package manifests
and dependency floors are prepared; this is not a registry publication claim.
October 9 changes preserve owned-container and scalar aliases, add checked
Python bounded builders, and make C/Go/Kotlin refuse unsupported bounded inputs.

The October 8 working changes add checked borrowed batches and repair Rust
client generation for bounded instruction arguments. The October 7 archives
remain evidence for their recorded source, not automatic approval of these
new changes. See [the batch contract](BORROWED_SLICES.md) and its executable
fixture for the added validation surface. Independent review and clean hosted
SBF CI remain release requirements; a new feature does not close either gate.

## Dossier reconciliation

The October 2 starting report contained 27 blocking findings and one tracked
warning. Most were stale evidence or expired check attestations, rather than
27 newly discovered code vulnerabilities. The prior report remains in the
[borrowed-arguments archive](../audit/borrowed-args-2026-10-02/readiness.json).
The current manifest is [audit/readiness.json](../audit/readiness.json).

The refreshed dossier closes **25 of the 27 blocking findings**. Independent
review and the exact clean-source SBF CI runs remain open. One nonblocking
transaction-v1 implementation item remains tracked.

| Recorded baseline, October 2 | Result |
| --- | --- |
| Full workspace tests | 2,486 reported passes, zero failures, 278 ignored |
| Workspace Clippy, all targets | Passed with warnings denied |
| Formatting | Passed |
| Fresh RustSec database | Zero vulnerabilities; four unmaintained informational advisories |
| Public API locks | All 28 library snapshots match |
| Unsafe contracts and map | 324 Rust files across 29 public packages; 855 mapped sites |
| Cicada host semantic cases | 698 passed, zero skips |
| Borrowed-argument devnet rerun | 20 finalized transactions, including 14 expected refusals |
| Website | Production build and 5,109 internal links/anchors across 59 served pages passed; browser interaction QA unavailable |

[Archived receipts and logs](../audit/readiness-2026-10-02/README.md) identify
the exact source and scope. Workspace counts include ignored tests and do not
imply that every compiled fixture executed. The token-lab tests used the
explicit archived 0.5 v3 ELF. Those historical gate receipts bound 733 text
inputs with CRLF/LF normalization; binaries and execution logs are byte-hashed.

The subsequent documentation review adds Markdown to the gate inventory because
Rust tests can compile it through `include_str!`. The
[documentation-input receipts](../audit/documentation-inputs-2026-10-02/README.md)
record the refreshed checks. The current manifest selects the active receipts;
the earlier archive remains unchanged. Markdown additions, edits, and removals
now invalidate recorded results just like source changes.

The [October 7 verification](../audit/borrowed-dispatch-2026-10-07/README.md)
covers borrowed `&MyArgs` handler dispatch, exact fixed-byte client encoding,
the updated manual/generated Solana fixture, and the refreshed network and
framework-source observations. These additions are unreleased. The current
manifest selects the latest complete gate receipts; historical archives retain
their original input inventory and workload.

The subsequent function-lab work adds a deployable probe program and a build
gate that discovers every workspace `cdylib` package. Its independent expected
values cover hashes, memory operations, arithmetic, return data, receipt logs,
crypto calls and account borrowing. Building all programs remains distinct
from executing all their handlers. The
[function-lab guide](../bench/function-lab/README.md) names optional calls and
the [network baseline](SOLANA_NETWORK_BASELINE.md) records live BLAKE3 failure
and an EpochSchedule inconsistency. These findings must not be advertised as
passing cluster compatibility. See the
[execution archive](../audit/full-devnet-2026-10-07/README.md) for measured scope.

- Evidence hashes are refreshed after checking the referenced contracts,
  source, toolchains, workflows, manifests, and retained receipts.
- The missing August planning documents are replaced by this release dossier
  and the [source-pinned framework comparison](FRAMEWORK_BOUNDARIES.md).
- The obsolete 0.3 publication requirement is closed by the
  [0.5 publication evidence](RELEASE_0_5_VALIDATION.md): 28 new indexed versions,
  downloaded archive verification, and a registry-only consumer. This does not
  publish or pre-approve the next release.
- Readiness schema 2 requires local gate execution receipts (receipt schema 1). These
  bind command, result, log, and current Rust/configuration/script/Markdown inputs.
  They are local evidence, not authenticated hosted CI results.
- The [network baseline](SOLANA_NETWORK_BASELINE.md) now records transaction v1
  as active. Hopper's legacy-only transaction authoring remains a tracked gap.

## Remaining release requirements

**Independent review:** no independent reviewer is engaged and no independent
security review has started. The [scope](../audit/EXTERNAL_AUDIT_SCOPE.md)
continues to apply. A passing internal check cannot close this requirement.

**Clean-source SBF CI:** both required jobs in
[run 36812193185](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/actions/runs/36812193185)
failed to start because GitHub reported an account billing lock. The required
Agave 2.3.13 and 4.2.1-forward lane artifacts still need to be produced from the
final clean source and archived. Existing local v0/v3 execution and devnet
receipts do not replace those exact attestations.

The next package version and publication must follow the public API comparison
and package train against the final source. No new version is published by this
readiness refresh.

The October 2 default-feature API locks differed from verified publication commit
`0a8827e177f225574f2285a58875d50e8059ffe7` by one default trait method,
`Pod::validate_value`. Generated macro behavior and dependency version floors
also need to be included in the final release plan. The October 7 changes add a
generated borrowed-argument decoder and stricter client encoding; they must be
included in that comparison before publication. The live registry planner
now fails explicitly when network access is unavailable; an inaccessible
registry must never be interpreted as proof that the packages are unpublished.
The later runtime changes also raise the hash-slice bound from 16 to 20,000,
share the native hash implementation, validate modular-exponentiation inputs,
and repair SBF entrypoint exports and receipt logging. They are unreleased;
existing registry artifacts are not evidence for these changes.
Epoch arithmetic also gains overflow saturation and a zero-divisor guard,
with independent boundary vectors in the compiled and deployable fixture.

## Evidence boundaries

The [borrowed argument fixture](../bench/borrowed-args/README.md) tests malformed
representations, exact lengths, bounded tails, account permissions, overflow,
return-data provenance, and unchanged state on refused writes. Its account
accepts any signer; it is not a custody authorization example.

The unsafe ledger and test map identify review obligations. Neither test counts
nor mapped unsafe sites establish soundness. Historical benchmark rows retain
their original source and workload labels. Website illustrations are explanatory
models and do not run Rust or submit transactions in the browser.
