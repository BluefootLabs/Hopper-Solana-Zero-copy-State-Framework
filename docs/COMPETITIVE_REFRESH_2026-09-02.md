# Competitive and network refresh — reverified 2026-09-03

This refresh supersedes time-sensitive claims in
[ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15](ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md)
and
[SOLANA_NETWORK_BASELINE_2026-08-15](SOLANA_NETWORK_BASELINE_2026-08-15.md).
Every count and network observation below is dated. The audit-readiness rule
still applies: do not claim “best”, “fastest”, “safest”, or universal
uniqueness without an independent report or a reproducible fixture.

## 1. Corrections to our own record

| Item | Stale claim | Reverified 2026-09-03 | Action |
| --- | --- | --- | --- |
| Mainnet block CU ceiling | 100,000,000 | **75,000,000** | Derived in `cost_model` |
| Mainnet per-writable-account CU ceiling | 12,000,000 | **30,000,000** | Derived in `cost_model` |
| Nature of both ceilings | Constants | **Slot-time-derived limits** | Modelled as `SlotTimeRegime` |
| Anchor v2 packaging | Not published; no v2 tag | `anchor-lang` **2.0.0-rc.1**, published 2026-08-12; tag `v2.0.0-rc.1` exists | Correct the older audit language |
| Anchor v2 crate name | `anchor-lang-v2` | Renamed to **`anchor-lang`** | Correct the migration and comparison docs |
| Agave stable pin | v4.2.1 | **v4.2.2**, tagged 2026-08-28 | Use v4.2.2 for this refresh |
| Tx account-lock limit | 128 | **64** for legacy and v0 | Describe the pending TxV1-only 64 → 96 proposal precisely |
| Quasar budget gate | Open PR, not shipped | Present on upstream’s `0.1.0-release` branch since 2026-07-14 | Retract the “Hopper shipped first” claim |
| CVLR loop default | Bound 1 is silently unsound | Insufficient unwinding **fails verification** unless optimistic loops are enabled | Keep a performance-risk caveat, not a soundness claim |
| Hopper layout fingerprints | Stable across build profiles | `hopper_layout!` selected SHA-256 or FNV from the resolved feature set | Make SHA-256 unconditional and pin an exact vector under `--no-default-features` |

### ABI correction discovered during this pass

The green-workspace pass exposed a more serious internal issue than a stale
comparison: the same `hopper_layout!` declaration produced a SHA-256 layout ID
with default features and an FNV-derived ID in the Spartan/SBF profile. Cargo
feature unification could therefore change persisted account and manifest ABI
without changing source.

Hopper now always computes layout IDs with its owned const SHA-256
implementation. This is compile-time evaluation at macro call sites, not a
runtime hashing path. A known-vector test is run both with the normal feature
set and `--no-default-features`, and the SBF migration fixture verifies that a
host build and deployed program agree.

## 2. Mainnet cost limits are a function, not constants

[SIMD-0525](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0525-reduce-slot-times.md)
defines a slot-time table that keeps the throughput target at 250M CU/second.
Agave v4.2.2’s
[`runtime/src/slot_params.rs`](https://github.com/anza-xyz/agave/blob/v4.2.2/runtime/src/slot_params.rs)
then composes those base limits with the
[SIMD-0286](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0286-raise-block-limits-to-100M.md)
feature by multiplying **both** ceilings by `100 / 60`.

| Regime | Base account / block | With SIMD-0286 |
| --- | --- | --- |
| 400 ms | 24M / 60M | 40M / 100M |
| 350 ms | 21M / 52.5M | 35M / 87.5M |
| **300 ms** | 18M / 45M | **30M / 75M** |
| 250 ms | 15M / 37.5M | 25M / 62.5M |
| 200 ms | 12M / 30M | 20M / 50M |

Direct finalized-mainnet feature queries rechecked on 2026-09-03 show:

- SIMD-0286 activated at slot 435,888,000 (2026-07-29).
- 350 ms activated at slot 440,208,000 and took effect in epoch 1020
  (2026-08-21).
- 300 ms activated at slot 441,936,000 and took effect in epoch 1024
  (2026-08-28).
- Mainnet was in epoch 1027 and the 300 ms regime remained current.

“100M blocks” was therefore the scaled 400 ms value, superseded when slot time
fell. The old 12M account number was the **unscaled 200 ms** base and was never
the corresponding mainnet value. Hopper now derives both ceilings from
`SlotTimeRegime`; two tests pin Agave’s table, the 250M CU/second invariant,
and the observed 75M/30M pair.

## 3. Network state and concurrency boundary

The following observations were repeated against finalized mainnet on
2026-09-03:

- Active: SIMD-0286, p-token, BLS registration, sBPF v3,
  `limit_instruction_accounts`, minimum loader-v3 extend size, LTDS
  fee-only semantics, and the 300 ms slot regime.
- Transaction v1 is not active. Legacy and v0 transactions remain bounded by
  the existing packet format.
- The five reduced-rent gates are not active. One feature account exists but
  is inactive; four are absent. The rent sysvar remains 6,960
  lamports/byte-year.
- Alpenglow is not active. Agave’s
  [v4.3 schedule](https://github.com/anza-xyz/agave/wiki/v4.3-Release-Schedule)
  gives 2026-09-28 as the start of generic mainnet feature activations, not a
  guaranteed Alpenglow activation date.
- SIMD-0449 direct account pointers are active on devnet and testnet, not
  mainnet.
- CPI stack height remains 5 (the entry invocation plus four nested CPIs).
  SIMD-0268 proposes 9.
- Loader v4 was abandoned; its identifier is the
  `LoaderV4WasAbandoned...` burn address.

### Sub-account locking remains absent

At repository commit `4828b2d`, all 125 proposal files and all 21 GitHub PRs
open on 2026-09-03 were enumerated and searched. No proposal or open PR
declares byte ranges for runtime account locks or introduces sub-account
locking. SIMD-0110 was closed unmerged on 2025-01-14. PR #596 proposes raising
the **TxV1 whole-account** lock count from 64 to 96; it does not change
granularity.

Agave v4.2.2’s
[`accounts-db/src/account_locks.rs`](https://github.com/anza-xyz/agave/blob/v4.2.2/accounts-db/src/account_locks.rs)
tracks locks as `AHashMap<Pubkey, u64>`; the lock key has no byte offset or
range.

The product boundary is firm: **byte-range precision buys no protocol-level
parallelism today.** Hopper’s ranges are useful for borrow enforcement,
auditing, effect comparison, and local conflict analysis—not for claiming
runtime scheduling or fee benefits.

## 4. Competitor state

| Project | Reverified state | Consequence for Hopper |
| --- | --- | --- |
| **Quasar** | Default branch last committed 2026-07-13; release branch last committed 2026-07-26; repository last pushed 2026-08-02. No tags/releases, published framework crates remain 0.0.0, and issue #498 records the broken README install command. | Treat as beta/source research, but credit its shipped release-branch budget gate and stronger verification lane. |
| **Anchor v2** | `anchor-lang` 2.0.0-rc.1 and tag `v2.0.0-rc.1` shipped 2026-08-12. Fourteen commits followed the prior 2026-08-15 snapshot, including sysvar instructions, float rejection, enum IDL, and PDA-signer support for remaining accounts. README still calls v2 Alpha and v1 stable. | Refresh packaging claims; track open ownership-gating PRs #4866/#4867. |
| **Pinocchio** | Released 0.11.2 predates SIMD-0449 parsing. `main` switched its entrypoint to the runtime-provided pointer array without a fallback, while 0449 is not active on mainnet. | Pin 0.11.2 for mainnet fixtures; treat current `main` as unreleased and mainnet-incompatible until the gate activates or compatibility is restored. |
| **pina** | Five releases from 2026-08-19 through 2026-09-01; uses Pinocchio plus `zeropod`, and ships security lints and a static CU profiler. | Strong source-lint ergonomics, but a different axis from runtime effect enforcement. |
| **zeropod** | Current line is 0.3.5; used by pina. Quasar’s release branch does **not** depend on it. | Do not describe it as a shared substrate for Quasar and pina. |
| Star Frame | No default-branch commit after 2026-02-26. | Monitor, do not call universally dormant from `pushed_at` alone. |
| Steel | Last public change observed 2026-06-16; self-described unaudited. | No evidence of a new verification model. |
| Typhoon | Public activity through 2026-07-22 under `aursen-labs`; unaudited. | Track the organization and release line, not volatile download counts. |
| Parallax | Fixture-based LiteSVM testing harness. | Classify as tooling, not a state framework. |
| Light Protocol | Default branch last committed 2026-07-21; repository had later activity on another ref. | Compression protocol, not a direct zero-copy framework comparison. |

Recent Blueshift work is visible in the separate `sbpf` repository. That is
not enough evidence to claim the Quasar team “pivoted,” so this refresh records
the activity without assigning a cause.

### Verification parity

- Quasar’s release branch invokes 183 distinct Miri tests under Tree Borrows.
  Three `miri_extensions` tests are additionally run under the default
  Stacked Borrows model; both lanes use strict provenance. It also runs 87 Kani
  proofs in CI.
- Anchor’s current tree invokes 60 Miri tests under Tree Borrows. It contains
  85 Kani proofs, disabled in CI pending a compatible Kani release.
- Hopper should add Stacked Borrows coverage and strict provenance, then run
  equivalent Kani proofs in CI before turning relative safety into a marketing
  claim.

### Corrected Quasar budget chronology

Quasar issue #468 and PR #496 are still open against the default branch, but
the `0.1.0-release` branch already contains commit `d8cec42` from
2026-07-14. It implements `quasar-budget.toml`, `--write-budget`,
`--assert-budget`, binary and CU totals, per-function checks, and exit status
2 on a budget failure. The release branch also contains code-level CU
assertions and per-example ELF-size tests. Its general workflow does not invoke
`profile --assert-budget`, but the feature itself is implemented.

Accordingly, Hopper must not claim it shipped first. Hopper’s distinct value is
budgeting **declared lock footprint and measured benchmark behavior**, while
Quasar covers static CU and binary-size budgets more broadly.

## 5. Scoped uniqueness finding

Dated GitHub code searches for `"touch map" solana`,
`"write_ranges" solana`, and `"effect manifest" solana`, plus local
searches of Anchor, Quasar, Pinocchio, and pina, found no equivalent Solana
framework implementation. The first two searches returned Hopper results; the
third found no Solana project.

This supports the scoped statement:

> As of 2026-09-03, the searched public Solana repositories did not expose an
> equivalent combination of byte-range write declarations, cumulative touch
> maps, executable effect manifests, and independent effect verification.

It does **not** prove that no private, differently named, or unindexed
implementation exists. The closest mature model is Soroban’s
[host-enforced transaction footprint](https://developers.stellar.org/docs/learn/fundamentals/contract-development/contract-interactions/transaction-simulation),
which declares reads and writes at whole-ledger-entry granularity; Soroban
storage does not support partial-value access.

## 6. Binary-size evidence: archived fact versus local diagnosis

The tracked, content-addressed framework matrix proves the comparable vault
pair:

| Framework | ELF bytes | Evidence |
| --- | ---: | --- |
| Hopper | 9,032 | `audit/framework-matrix-2026-08-16.json` |
| Quasar | 5,784 | Same archive, pinned source and toolchain |
| **Difference** | **3,248** | Reproducible subtraction |

The later guard-off build (7,632 bytes), its 1,400-byte attribution, the 43%
share, the 7,408-byte `.text`, 5,112-byte inlined `entrypoint`, and the
73,408-byte Sentinel figure were recorded only as working-session diagnostics.
No tracked artifact with source hash, toolchain identity, and outputs was found
for those exact values. They must not be promoted as reproducible benchmark
facts until re-run and archived.

What the archive supports today: Quasar’s pinned vault is 3,248 bytes smaller
than Hopper’s pinned vault. What remains to measure: a controlled feature
ablation that isolates the ambient write gate and touch map, records every
input hash, and distinguishes framework machinery from fixture behavior.

Cross-framework CU claims need the same discipline. Anchor’s own benchmark
history mixes a third-party Quasar fork and upstream branches, and historical
rows move with lockfile changes. Quote only like-for-like, pinned fixtures.

## 7. Structural risk: ABIv2

[SIMD-0177](https://github.com/solana-foundation/solana-improvement-documents/pull/177)
remains an open proposal with status Idea. Public prototypes exist in
[`blueshift-gg/abiv2`](https://github.com/blueshift-gg/abiv2) and the
[`mollusk` `abiv2` branch](https://github.com/anza-xyz/mollusk/tree/abiv2).
If adopted, it changes the entrypoint/account interface for every framework,
including Pinocchio’s direct-pointer path.

Do not build product plans on ABIv2’s arrival. Keep Hopper’s entrypoint and
account-decoding layer replaceable, and track the prototypes as compatibility
work—not as evidence of a committed network migration.

## 8. Ranked innovation work

### 1. Release-bound effect diff (medium)

Extend Hopper’s existing executable `ProgramManifest` and release binding;
do not invent a parallel manifest. Compare deployed and candidate releases for:

- new or removed instructions;
- widened write ranges or writable-account sets;
- new lamport permissions;
- new CPI targets;
- changed discriminator and schema bindings.

Publish or reference the signed result through the official
[Program Metadata program](https://github.com/solana-program/program-metadata/blob/main/README.md).
Its documented seeds include `idl` and `security`, while no standardized
`behavior` record is documented upstream. Custom seeds are possible, so a
Hopper convention needs schema/versioning work and upstream coordination.

A mainnet `getProgramAccounts` query returned 1,081 program-owned accounts on
2026-09-03. That volatile number includes multiple account types and buffers;
it is an adoption signal, not a canonical count of published metadata records.
The artifact provides upgrade visibility, not prevention: an upgrade can
declare broader authority, but reviewers can see the expansion before signing.

### 2. Typed field-level state diff (small)

Use layout manifests to render changes such as
`config.paused: 0 → 1 @ byte 114` instead of unrelated base64 blobs.
Surfpool, Mollusk, and replay tools can provide snapshots; Hopper adds the
schema-aware interpretation.

### 3. Ledger-bound behavioral verification (medium–large)

Let Grillo accept a mainnet transaction signature, reconstruct pre/post account
bytes, derive the changed byte set, and check it against the release-bound
manifest. Solana’s
[transaction status metadata](https://solana.com/docs/rpc/json-structures)
contains balances and token balances but not arbitrary pre/post account data,
so this requires deterministic SVM replay, a Geyser/archive sidecar, or an
account-history provider. State that dependency explicitly.

Without an instrumented touch map, the defensible claim is only
`changed ⊆ authorized`. That weaker check is still useful for programs that
publish a compatible third-party manifest.

### 4. CVLR-generated frame conditions (large; spike first)

Generate
[CVLR](https://docs.certora.com/en/latest/docs/solana/spec.html) rules from
declared write ranges: snapshot account bytes, invoke one handler, and assert
that every byte outside the authorized intervals is unchanged. Certora has
published Solana reports for
[p-token](https://www.certora.com/reports/solana-ptoken) and
[stake-pool](https://www.certora.com/reports/solana-stake-pool), while
[`CertoraProver`](https://github.com/Certora/CertoraProver) is GPLv3 and can
be self-hosted.

The default loop bound is one, but insufficient unwinding fails verification
unless the explicitly optimistic loop mode is enabled. The real spike risk is
solver cost for byte-array frame equality and loop-heavy zero-copy code. Gate
the roadmap on one instruction terminating with non-optimistic loops and a
useful counterexample when a forbidden write is injected.

### 5. Verification-lane parity (medium)

Add strict-provenance Miri under Tree and Stacked Borrows where supported, then
add Kani proofs for range bounds, overlap rules, discriminators, and failed
borrow rollback. Publish exact toolchain and proof counts.

### 6. Controlled size attribution (small)

Rebuild one fixed fixture with the ambient write guard, touch map, effect
manifest, and diagnostics independently toggled. Archive ELF, section sizes,
source hashes, lockfile, compiler/Solana toolchain, and commands. This converts
the current diagnostic numbers into evidence and tells us where optimization
actually pays.

## 9. Do not build

- Anything premised on Solana scheduling transactions at byte-range
  granularity.
- A sub-account-locking SIMD without an author, prototype, and runtime design.
- A Hopper symbolic executor or model checker. Generate narrowly scoped rules
  for an existing engine first.
- Another general CU line profiler, forking validator, or state-cheatcode
  suite; integrate Hopper’s manifests with those tools.
- On-chain touch emission by default. It charges every user forever for
  evidence that can usually be produced off-chain.
- A TEE framing for deterministic SVM replay; it introduces another trust
  boundary without improving public reproducibility.

## 10. Reproduction ledger

This refresh used:

- Agave tag `v4.2.2` (`c9c6f328…`) for slot parameters, feature IDs, stack
  depth, loader state, and account-lock types;
- the SIMD repository at `4828b2d`, plus the 21 PRs open through
  2026-09-03;
- finalized feature-account and sysvar queries to mainnet, with devnet/testnet
  checks for the 200/250 ms and direct-pointer gates;
- current default/release branches for Anchor, Quasar, Pinocchio, pina,
  zeropod, Star Frame, Steel, Typhoon, Parallax, and Light;
- the tracked `audit/framework-matrix-2026-08-16.json` archive for the only
  size numbers treated as reproducible facts.

Dates are part of every network, count, release, and repository-activity claim.
Re-run this ledger before carrying those claims into external material.
