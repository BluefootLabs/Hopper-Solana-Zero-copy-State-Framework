# Hopper audit-readiness and parity dossier — 2026-08-15

## Decision

Hopper is **not yet independently audit-ready**. Its automated verification,
unsafe-code accounting, manifest-driven ABI checks, contention analysis, and
zero-copy design form a strong technical base, but an external review has not
been completed. The 2026-08-16 dependency scan is now vulnerability-clean:
the five inherited host-client vulnerabilities were removed by aligning the
RPC, signing, and Mollusk graphs to Agave 4.2.1. Four unmaintained host/dev
dependencies remain explicitly documented as informational upstream debt, not
unreviewed release blockers.

Run the executable record with:

```text
cargo run -p hopper-cli -- audit-check
cargo run -p hopper-cli -- audit-check --strict --json
```

The first command reports evidence and blockers. The strict form exits nonzero
until every required evidence hash, expiring quality-gate attestation, external
audit report, and known blocker is resolved. The source of truth is
`audit/readiness.json`.

## What “best” must mean

“Best” is not a README adjective. Hopper should claim leadership only where a
reproducible artifact supports it:

- **Safety:** malformed input, account substitution, undersized buffers,
  migrations, aliasing, and raw-surface escape hatches fail closed.
- **Cost:** the same program behavior uses equal or fewer measured CUs and equal
  or smaller deploy binaries on the same toolchain and runtime.
- **Speed:** build, test, simulation, and client-generation latency are measured
  on pinned fixtures; network confirmation time is never claimed as framework
  performance.
- **DX:** init-to-test and init-to-deploy paths are dependency-light, errors name
  the violated invariant, and generated clients preserve every writable/signer
  declaration.
- **Evidence:** benchmark inputs, toolchain, raw results, unsafe inventory,
  dependency audit, and independent report are publishable and reproducible.

Hopper's differentiator should be **machine-checkable intent**: layout,
migration, instruction effects, client account metas, security policies, and
release evidence all derive from declarations that independent tooling can
cross-check.

## Current peer baseline

The relevant stable and active release branches were inspected on 2026-08-15.
Versions below are source-workspace and registry facts, not assumptions based
on old articles. The detailed snapshot, SHAs, audit PRs, and flagship evidence
are in [the zero-copy framework audit](ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md).

| Framework | Current upstream signal | Strength Hopper must match | Hopper-plus direction |
|---|---|---|---|
| Anchor | Stable is **1.1.2**. The v2 line is published as `anchor-lang` 2.0.0-rc.1 and remains self-described Alpha/unaudited: a Pinocchio-based `no_std` rewrite with default zero-copy `Account<T>`, dynamic `Slab`/`PodVec`, typed CPI borrows, and Kani/Miri/fuzz lanes. | Mature ecosystem, high-level constraints, testing/deploy workflow, clients, IDL and SPL breadth; v2 raises the zero-copy and verification baseline. | Preserve lower-level performance while making invariants and migrations independently checkable; add equivalent release provenance, dynamic-state ergonomics, formal evidence, and versioned-envelope support. |
| Blueshift Quasar | Default branch and crates.io remain **0.0.0**, but the active `0.1.0-release` branch is the meaningful comparison target. It is explicitly beta/unaudited and adds typed resize migrations, wire IDL/ABI hashing, stable Rust/Kit/Web3 clients, preview Python/Go/C clients, QuasarSVM, a broad CLI, Kani/Miri/fuzz, and CU budget work. | Familiar macros plus a lean runtime, migration ergonomics, profiling, SPL, clients, and formal checks. | Match simple program ergonomics and safe resize; exceed with effect/write-set verification, generated-client parity checks, and an honest external-audit trail. |
| Pinocchio | Current line **0.11**. Minimal `no_std` program primitives, lazy/custom entrypoints, no-allocator mode, zero-copy views, and small dependency/binary surface. | The efficiency floor and explicit control available to expert authors. | Keep a raw expert path without allowing it to bypass owner, signer, writable, layout, or effect declarations. Benchmark Hopper-generated code against equivalent Pinocchio code. |
| Star Frame | Current workspace line **0.30**. High-performance modular traits, Pinocchio base, zero-copy unsized data and CLI support. | Composable high-performance abstractions and flexible account types. | Show that schema/effect/migration metadata adds measurable safety and tooling without imposing unacceptable CU or binary overhead. |

Primary upstream records:

- [Anchor stable source and tags](https://github.com/otter-sec/anchor), and [v2 alpha source](https://github.com/otter-sec/anchor/tree/anchor-next/lang-v2)
- [Blueshift Quasar default source](https://github.com/blueshift-gg/quasar) and [`0.1.0-release` source](https://github.com/blueshift-gg/quasar/tree/0.1.0-release)
- [Anza Pinocchio source](https://github.com/anza-xyz/pinocchio)
- [Star Frame source](https://github.com/staratlasmeta/star_frame)

## Parity assessment

| Capability | Hopper status | Audit conclusion |
|---|---|---|
| Zero-copy state and segmented layouts | Implemented | Core strength; retain invariant documentation and malformed-input tests. |
| Account constraint/code generation | Implemented | Keep client and on-chain declarations cross-checked; C4 already found and fixed an `epoch_migrate` writable-meta defect. |
| Layout evolution and migration planning | Implemented | Strong differentiator, but external reviewers need end-to-end fixtures and rollback/failure analysis. |
| IDL/schema and multi-language clients | Implemented across Rust, TS, Kotlin, Python, Go, and C emitters | Breadth is strong; generated-code compile tests must remain release gates. |
| SPL integration | Implemented, with legacy surfaces gated | Compare coverage against current Anchor SPL and Quasar SPL instruction-by-instruction before a parity claim. |
| Lifecycle CLI | Init/build/test/deploy/upgrade/close/migrate/dump plus inspection and verification | Good breadth. LiteSVM/Surfpool-equivalent one-command local test ergonomics still need a measured journey audit. |
| Formal and adversarial checking | Kani lanes, unsafe inventory, fuzz targets, verifier/evidence paths, deterministic manifest-derived plans/corpora, and Cicada's no-skip semantic adapter | Cicada's 698-case plan is recomputed against the live manifest and runs actual private business guards under seeded host states in CI. This is host semantic evidence, not 698 SBF transactions; the compiled-SBF lifecycle suite remains the runtime evidence boundary. |
| Contention advisor | Implemented at account/write-lock level | Correctly matches Solana scheduling reality. Never describe disjoint byte ranges inside one pubkey as currently parallel. |
| Versioned transactions | Hopper CLI currently builds legacy envelopes | Gap. v1/4,096 support is upcoming network functionality and requires a newer finalized SDK path; legacy size must fail early today. |
| Independent security audit | Not completed | Critical adoption blocker. |
| Cicada flagship custody and route safety | Internally hardened; 21 host and 22 strict compiled-SBF tests pass against canonical SPL Token, Token-2022, and hostile route fixtures, including exact wrapped-SOL lamport coupling on both canonical processors, rollback of a modeled native-account shortfall, and pre-CPI rejection of conflicting or repeated-writable route aliases. A clean checkout of `3dfceba` reproduced the 167,680-byte ELF (`sha256:ac8ec1d76b4f85a5515dc446536bafccabe1c62b8ff46a13a662971b785da0e9`) and retained the then-current layout-anchor publish-check attestation. That artifact predates the versioned ELF interface binding and does not satisfy the current release gate. | Strong clean historical evidence only, not an independent audit, a current interface-bound artifact, or the required pinned CI artifact runs. Retain the public-cluster/validator-replay, issuer-authority, upgradeable-route, narrow Token-2022, and dynamic downstream-attribution limits in every release claim. |
| Reproducible head-to-head benchmark | Closed for the pinned 2026-08-16 source set: the five-way Hopper/Pinocchio/Quasar/Anchor v2/Star Frame fixture passed successful-state parity checks and all 30 rejection gates from clean commits, then retained a content-addressed archive | Use the fixture-specific 1,578/424 CU Hopper rows and peer rows only with [`audit/framework-matrix-2026-08-16.json`](../audit/framework-matrix-2026-08-16.json). The clean result closes the evidence gap, not the independent-audit gap or a universal ranking. |

## Architecture findings

1. **Keep the declaration graph authoritative.** Account roles, mutability,
   instruction effects, migrations, clients, contention reports, fuzz harnesses,
   and audit evidence should be projections of one manifest rather than parallel
   hand-maintained truths.
2. **Separate SVM constraints from future protocol opportunities.** Solana's
   scheduler still locks by pubkey. C6 should remain an account-set contention
   and transaction-envelope advisor; byte-range scheduling is research, not a
   current performance feature.
3. **Treat raw APIs as audited escape hatches.** Pinocchio demonstrates the value
   of direct control. Hopper can expose it only when the same ownership,
   mutability, aliasing and effect contracts remain machine-visible.
4. **Keep host tooling off the hot path.** Rich schemas, explainers, generated
   clients and evidence tooling belong in the CLI/build pipeline. On-chain code
   should pay only for checks required for correctness.
5. **Make network assumptions explicit.** The dated network baseline records
   Mainnet-live versus planned behavior. Transaction builders must check their
   actual serialized envelope, not infer support from a roadmap date.

## Blocking path to an independent review

| Priority | Deliverable | Exit criterion |
|---|---|---|
| P0 | Dependency advisory refresh/remediation | `cargo audit` (or an equivalent pinned scanner) is current; every advisory is fixed, mitigated with a documented reachability argument, or explicitly accepted by a named owner. |
| P0 | External audit package | Scope, commit SHA, threat model, invariants, build instructions, test/fuzz/Kani commands, prior findings, dependency report, and reproducible artifacts are delivered to an independent reviewer. |
| P0 | Independent audit | Report and remediation commits are public or available to adopters under an explicit disclosure policy. |
| P1 | Current SBF compatibility lane | CI exercises the supported Mainnet Agave toolchain and a forward Agave 4.2 lane without silently replacing the pinned reproducible lane. |
| Closed 2026-08-16 | Clean same-fixture peer benchmark artifact | Hopper `8696640` and benchmark source `af5bc95` produced a fresh-build, 8-sample, 30/30-gate archive retained at benchmark evidence-carrier commit `7ab6a3e`, with ZIP SHA-256 `c64af2460bcbfc0a9a3b8e5a7d8ecdbaa73ff34b7b5d20b0f17e89e44a84f747`; rerun whenever any recorded benchmark-input pin changes. |
| Closed locally; CI attestation pending | C3 manifest-driven fuzz execution | `hopper fuzz generate/check` gates a 698-case Cicada plan and `hopper fuzz run` executes every case without skips through the fail-closed semantic adapter. The adapter distinguishes live-manifest structural probes and actual Cicada host business guards from compiled-SBF transaction evidence. |
| P1 | Transaction v1 readiness | Legacy size checks ship now; v1 construction/decoding lands only with finalized SDK types, feature activation detection, and golden fixtures. |

## Claim policy

Until the blockers close, public language should say “designed for” or cite a
specific measured fixture. Do not say “audited,” “fastest,” “lowest cost,” or
“best” without the corresponding independent report or reproducible result.
That restraint is part of making Hopper credible enough to become the standard.
