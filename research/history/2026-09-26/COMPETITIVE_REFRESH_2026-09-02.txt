# Competitive and network refresh: reverified 2026-09-06

> **Superseded in part on 2026-09-19.** Transaction v1 is active on
> mainnet-beta, rent is 5,080 lamports per byte, and the release-bound
> authority diff ranked below has shipped. See
> [COMPETITIVE_REFRESH_2026-09-19.md](COMPETITIVE_REFRESH_2026-09-19.md).

This refresh supersedes time-sensitive claims in
[ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15](ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md)
and
[SOLANA_NETWORK_BASELINE_2026-08-15](SOLANA_NETWORK_BASELINE_2026-08-15.md).
Every count and network observation below is dated. The audit-readiness rule
still applies: do not claim “best”, “fastest”, “safest”, or universal
uniqueness without an independent report or a reproducible fixture.

The product statement that survives this pass is concrete:

> **Solana locks accounts. Hopper governs bytes.** Declare what an instruction
> may mutate; Hopper checks the range before the borrow and records what it
> acquired; Grillo separately recomputes the resulting effect relation.

This 0.3 workspace implements **Declare → Enforce → Observe → Verify**. A
release-bound effect-authority **Compare** step is ranked work below. Mainnet scheduling
remains whole-account; none of these byte-level controls creates sub-account
parallelism or a byte-level fee discount.

## 1. Corrections to our own record

| Item | Stale claim | Reverified 2026-09-06 | Action |
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
| Mainnet rent reserve | 6,960 lamports per byte | **6,333 lamports per byte** after SIMD-0437 step 1 activated 2026-09-03 | Read the live Rent sysvar/RPC; date every SOL quote |
| Solana frame-condition whitespace | Nobody proves absence of mutation | QEDGen/qedsvm proves narrow selected compiled paths and untouched fields | Integrate external proof engines; differentiate on enforced runtime authority and cumulative evidence |

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

### Client-generation corrections discovered during the polish pass

Compiling Hopper's own compact, headered, `Address`, and Cicada fixtures found
four more fail-open or unusable edges, fixed in `f8939a3`:

- Python's flat emitter placed `from __future__` imports after executable code
  and split `build_<instruction>.ACCOUNT_ORDER` across a newline; dataclass
  constants were also instance fields. Generated modules now use `ClassVar`,
  keep metadata assignments intact, and contain no mid-file future imports.
- Python, Go, C, and off-chain Rust compact readers incorrectly tried to read a
  header fingerprint from bytes `4..12`. All six SDK targets now use the same
  boundary: headered layouts compare `LAYOUT_ID`; fixed compact layouts require
  exact size plus discriminator; compact-dynamic layouts require their minimum
  prefix and accept a longer application-defined tail. Both compact forms take
  the fingerprint from manifest/IDL metadata.
- The Rust client emitter now maps Hopper `Address`, typed addresses, and wire
  scalars to matching compilable types, and preserves unknown fixed-size values
  as byte arrays of their declared size instead of zero-length placeholders.
- A bare `SCREAMING_SNAKE_CASE` seed such as Cicada's `CONFIG_SEED` can name a
  Rust constant whose bytes are absent from the manifest. It now remains
  caller-provided instead of being misclassified as an account and used to
  derive an invalid PDA.

The focused lane passes 251 executable `hopper-schema` tests (204 unit plus 47
integration/property), formatting, and clippy with warnings denied. Seven
generated Rust/C artifacts compiled with
warnings denied. Python and Go runtimes were unavailable on the audit host, so
those outputs received generator regression tests and source review rather
than an external interpreter/compiler claim.

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

The following observations were repeated against finalized mainnet through
2026-09-06:

- Active: SIMD-0286, p-token, BLS registration, sBPF v3,
  `limit_instruction_accounts`, minimum loader-v3 extend size, LTDS
  fee-only semantics, and the 300 ms slot regime.
- Transaction v1 is not active. Legacy and v0 transactions remain bounded by
  the existing packet format.
- SIMD-0437 reduced-rent step 1 activated at slot 444,096,000 on
  2026-09-03. Mainnet now uses a 6,333-lamport-per-byte rent-exempt reserve
  coefficient with SIMD-0194's `1.0` wire threshold marker. The four later
  reduction gates were absent in the finalized 2026-09-06 query.
- Alpenglow is not active. Agave’s
  [v4.3 schedule](https://github.com/anza-xyz/agave/wiki/v4.3-Release-Schedule)
  gives 2026-09-28 as the start of generic mainnet feature activations, not a
  guaranteed Alpenglow activation date.
- SIMD-0449 direct account pointers are active on devnet and testnet, not
  mainnet.
- Account Data Direct Mapping is likewise active on devnet/testnet but remains
  under **Pending Mainnet Beta Activation** in Anza's feature-gate schedule as
  rechecked 2026-09-06. Mainnet therefore still uses the serialized/copy path;
  Hopper's observed-length CoW cost helper is forward-looking/test-cluster
  guidance, not a current Mainnet CU model.
- CPI stack height remains 5 (the entry invocation plus four nested CPIs).
  SIMD-0268 proposes 9.
- Loader v4 was abandoned; its identifier is the
  `LoaderV4WasAbandoned...` burn address.

### Sub-account locking remains absent

At repository commit `4828b2d`, 125 proposal Markdown files and the 22 GitHub
PRs open on 2026-09-03 were enumerated and searched. A repeat query found 22
open PRs on 2026-09-06; new PR #621 is unrelated to account locking. No file
or open PR
declares byte ranges for runtime account locks or introduces sub-account
locking. SIMD-0110 was closed unmerged on 2025-01-14 and proposed exponential
fees for **whole write-locked accounts**, not sub-account locks. PR #596
proposes raising the **TxV1 whole-account** lock count from 64 to 96; it does
not change granularity.

Agave v4.2.2’s
[`accounts-db/src/account_locks.rs`](https://github.com/anza-xyz/agave/blob/v4.2.2/accounts-db/src/account_locks.rs)
tracks locks as `AHashMap<Pubkey, u64>`; the lock key has no byte offset or
range.

The product boundary is firm: **byte-range precision buys no protocol-level
parallelism today.** Hopper’s ranges are useful for borrow enforcement,
auditing, effect comparison, and local conflict analysis; not for claiming
runtime scheduling or fee benefits.

## 4. Competitor state

| Project | Reverified state | Consequence for Hopper |
| --- | --- | --- |
| **Quasar** | Default pin `b0de7db` (2026-07-13), release pin `0361701` (2026-07-26). No tags/releases; published framework crates remain 0.0.0, and issue #498 records the broken README install command. Its release line includes static CU/binary budgets plus a deeper Miri/Kani lane. | Treat as beta/source research; credit the size and verification leads. Hopper's distinction is enforced range authority plus cumulative evidence, not “shipping budgets first.” |
| **Anchor** | Stable v1.2.0 shipped 2026-09-04. The v2 tree reached `38bb553` on 2026-09-05; latest published `anchor-lang` v2 remains 2.0.0-rc.1 from 2026-08-12 while its README calls v2 Alpha. Recent commits include behavior and correctness fixes, including tail-slab minimum-length handling. | Anchor leads ecosystem maturity. Keep the historical benchmark labeled as a pinned pre-RC snapshot, not current v2. |
| **QEDGen / qedsvm** | `solana-skills` v2.49.0 / `bf7f968`; qedsvm v0.12.0 / `99bd5ed`. A `.qedspec` effect can lower to a versioned descriptor, resolve IDL offsets, check a selected compiled path, and discharge a sorry-free Lean theorem. Ratchet also diffs Anchor/Quasar IDLs. Coverage remains narrow: selected paths, bounded traces, limited mutations, and no general real-CPI proof. | Retract “nobody proves absence of mutation” and generic “nobody diffs behavior.” Build a manifest bridge instead of another prover. Hopper retains the runtime gate, cumulative tracked-borrow evidence, an ELF commitment to the manifest-projected declaration, and separate effect recomputation, within the limits below. |
| **Pinocchio** | Released 0.11.2 predates SIMD-0449 parsing. `main` switched its entrypoint to the runtime-provided pointer array without a fallback, while 0449 is not active on mainnet. | Pin 0.11.2 for mainnet fixtures; treat current `main` as unreleased and mainnet-incompatible until the gate activates or compatibility is restored. |
| **pina** | Pin `4b5eeeb` on 2026-09-06; 0.12.2 published 2026-09-01. It uses Pinocchio plus `zeropod` and ships Codama/client, fuzz, coverage, mutation, Surfpool, security-lint, and budget workflows. Mutable loading still grants a whole typed account. | Strong tooling ergonomics, but a different axis from range authority and effect evidence. |
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

- A local enumeration of Quasar's pinned release branch found 183 Miri tests
  under Tree Borrows and 87 Kani proof functions exercised by CI. Three
  `miri_extensions` tests are additionally invoked under the default Stacked
  Borrows model; both Miri lanes enable strict provenance. Treat these as
  reproducible test-list counts, not audit-equivalent proof totals.
- A local enumeration of Anchor's 2026-09-05 tree found 60 Miri tests under
  Tree Borrows and 85 Kani proof functions. Its Kani CI lane remains disabled
  pending a compatible Kani release.
- zeropod's current tree also contains 100 syntactic Kani proof functions and
  runs Kani in CI. It has no equivalent range/effect model.
- Hopper should add Stacked Borrows coverage and strict provenance, then run
  equivalent Kani proofs in CI before turning relative safety into a marketing
  claim.

### QEDGen changes the formal-verification comparison

QEDGen's current descriptor pipeline overlaps two ideas that earlier Hopper
research described as empty whitespace. It can lower a constrained `.qedspec`
effect into a versioned `RefinementDescriptor`, resolve fields to byte offsets,
check one selected compiled path in an SBF artifact, and generate a sorry-free
Lean theorem that frames untouched fields. Its Ratchet integration also applies
seven program-level and twenty account-level upgrade rules to Anchor and Quasar
IDLs.

Primary sources: the pinned
[`descriptor.rs`](https://github.com/QEDGen/solana-skills/blob/bf7f968661d114b373c412d8614130c148d144b0/crates/qedgen/src/descriptor.rs),
qedsvm's
[`REFINEMENT_DESCRIPTOR.md`](https://github.com/QEDGen/qedsvm/blob/99bd5ede85374adc7fc5c835c2432ecf4e123fd1/docs/REFINEMENT_DESCRIPTOR.md),
its explicit
[`COVERAGE.md`](https://github.com/QEDGen/qedsvm/blob/99bd5ede85374adc7fc5c835c2432ecf4e123fd1/docs/COVERAGE.md),
and QEDGen's pinned
[`ratchet.rs`](https://github.com/QEDGen/solana-skills/blob/bf7f968661d114b373c412d8614130c148d144b0/crates/qedgen/src/verify/ratchet.rs).

That is real proof work, but its current automatic envelope is deliberately
narrow: one selected path rather than whole-CFG coverage, bounded concrete loop
traces, a small set of `u64` update forms and field shapes, and termination at
real CPI or unsupported syscalls. The correct response is integration, not a
uniqueness claim. Hopper can export its declared range authority to qedsvm and
CVLR, carry the separate ELF commitment alongside it, then publish which
handlers, paths, loops, and syscalls the external engine actually covered.

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
`"write_ranges" solana`, and `"effect manifest" solana`, plus source-pinned
searches of Anchor, Quasar, Pinocchio, pina, QEDGen/qedsvm, zeropod, Star
Frame, Steel, Typhoon, Light Protocol, Parallax, and LiteSVM, found no
equivalent implementation of Hopper's complete loop.

This supports the scoped statement:

> As of 2026-09-06, across the named public repositories and pinned revisions,
> no implementation was found that combines runtime-enforced byte-range write
> authorization, cumulative tracked-borrow evidence, an ELF-embedded commitment
> to a manifest-projected interface/effect declaration, and a separate verifier
> that recomputes effects against that declared contract.

It does **not** prove that no private, differently named, or unindexed
implementation exists. The ELF record commits to the projected declaration; it
does not embed the manifest, prove handler control flow, authenticate a
deployment, establish artifact freshness, or make Grillo consume the ELF.
Grillo currently verifies caller-supplied evidence and identity fields, not
ledger provenance. QEDGen/qedsvm overlaps selected-path frame proofs and
upgrade analysis, but not Hopper's runtime authorization or cumulative touch
evidence. The closest mature host-enforced declaration model is Soroban’s
[host-enforced transaction footprint](https://developers.stellar.org/docs/learn/fundamentals/contract-development/contract-interactions/transaction-simulation),
which declares reads and writes at whole-ledger-entry granularity, including
transitive calls, and whose simulation returns modified-entry diffs. Soroban
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

## 7. Cicada and Grillo deployment economics

Deployment rent is refundable principal, not a burned protocol fee. For a
fresh upgradeable-loader-v3 deployment with ELF length `B` and reserved
`max_len`, query the target cluster's live
`getMinimumBalanceForRentExemption` function `R`:

```text
permanent Program account     = R(36)
permanent ProgramData account = R(45 + max_len)
permanent total               = R(36) + R(45 + max_len)
Buffer account rent floor     = R(37 + B)
```

The stock Solana CLI funds its temporary Buffer at the **ProgramData
requirement**, not merely the Buffer floor. Loader v3 drains that full balance
back to the payer immediately before allocating ProgramData. It is recycled
working capital, not a second permanent charge. Transaction and optional
priority fees are separate.

The final 2026-09-06 dirty-tree diagnostic build of Cicada is 165,944 bytes,
SHA-256
`7ee1247f704b6feb42cfc499b9bcdb30b79b4f83bc4de599cbe389b685c2defb`,
with 152,528 bytes of `.text`. Two isolated rebuilds plus the working target
artifact, three matching copies total, were byte-identical, and
strict release verification found interface commitment
`98a0eaf0cf78b13881426c0894e4fd521b7250e7f419389b180662a3b08d1976`
plus all three layout anchors. Because the shared source tree was dirty, this
is reproducible diagnostic evidence, not a clean-commit release attestation.

At Mainnet slot 444,767,908 with `max_len = B`:

| Component | Data bytes | Lamports | SOL | Treatment |
| --- | ---: | ---: | ---: | --- |
| Program | 36 | 1,038,612 | 0.001038612 | permanent |
| ProgramData | 165,989 | 1,052,018,961 | 1.052018961 | permanent |
| **Cicada loader-v3 total** | n/a | **1,053,057,573** | **1.053057573** | refundable principal; fees excluded |
| Buffer rent floor | 165,981 | 1,051,968,297 | 1.051968297 | transient account floor |
| Stock CLI Buffer funding | 165,981 | 1,052,018,961 | 1.052018961 | drained and reused for ProgramData |
| Config | 112 | 1,519,920 | 0.001519920 | one per instance |
| Shard (20 slots) | 9,080 | 58,314,264 | 0.058314264 | one minimum; each additional shard repeats it |
| SourceLease | 128 | 1,621,248 | 0.001621248 | refundable when reclaimed |

A minimally usable Cicada instance, program, ProgramData, Config, and one
Shard, therefore locks **1.112891757 SOL**, plus fees and any active leases.
At the pinned Agave default of 1,012 program bytes per write transaction, this
ELF implies about 164 writes plus Buffer creation and final deploy. An
illustrative default base-signature fee is about 0.000840000 SOL, but the exact
fee must come from `getFeeForMessage` because signer layout, retries, priority
price, and cluster conditions vary.

`hopper deploy --dry-run --cluster <cluster>` now builds or reuses the exact
artifact, queries live rent, records the slot at which the RPC reads began,
reports permanent and recycled principal separately, and sends no transaction.

Grillo is an offline host library/CLI with no Solana entrypoint or `cdylib`.
Its on-chain deployment cost is **0 SOL / not applicable**; only the operator's
off-chain compute and storage exist. The two workspace packages are versioned
0.1.0 but were not observed on crates.io on 2026-09-06. Effect ABI v0.2 is an
experimental evidence/schema surface, not a published Grillo crate release.

## 8. Structural risk: ABIv2

[SIMD-0177](https://github.com/solana-foundation/solana-improvement-documents/pull/177)
remains an open proposal with status Idea. Public prototypes exist in
[`blueshift-gg/abiv2`](https://github.com/blueshift-gg/abiv2) and the
[`mollusk` `abiv2` branch](https://github.com/anza-xyz/mollusk/tree/abiv2).
If adopted for ABI-v2/sBPFv4 targets, it changes virtual layout, entry
registers, and metadata syscalls for every framework, including Pinocchio's
direct-pointer path. It would not instantly invalidate already deployed legacy
ABI binaries.

Do not build product plans on ABIv2’s arrival. Keep Hopper’s entrypoint and
account-decoding layer replaceable, and track the prototypes as compatibility
work; not as evidence of a committed network migration.

## 9. Ranked innovation work

### 1. External proof bridge and coverage certificate (medium)

Export Hopper's existing layout and range authority into both qedsvm
`RefinementDescriptor` inputs and CVLR frame rules. Hopper supplies the runtime
gate and observed touch set; an external engine proves that selected compiled
paths respect the same authority. Do not duplicate either symbolic engine.

Every release should publish a proof-coverage certificate beside the binary and
manifest hashes: verifier/version, covered handlers and paths, unsupported
paths/syscalls, loop bounds and optimism mode, plus observed runtime coverage.
A selected-path theorem is valuable when its boundary is machine-readable.

### 2. Release-bound effect-authority diff (medium)

Compose with QEDGen Ratchet for existing IDL/layout compatibility rather than
claiming generic behavior-diff whitespace. Hopper's distinct upgrade review is
the authority delta from its ELF-committed, manifest-projected declaration:

- new or removed instructions;
- widened write ranges or writable-account sets;
- new lamport permissions;
- new CPI targets; and
- changed artifact, discriminator, schema, and proof-coverage bindings.

Publish or reference the signed result through the official
[Program Metadata program](https://github.com/solana-program/program-metadata/blob/main/README.md).
Its documented seeds include `idl` and `security`; no standardized behavior or
effect record is documented. Custom seeds are possible, so a Hopper convention
needs a versioned body, release binding, and upstream coordination. This gives
upgrade signers visibility into expanded authority; it does not prevent them
from approving it.

### 3. Parallax/LiteSVM adapter plus typed field diff (small)

Consume Parallax `AccountChange { before, after }` or LiteSVM snapshots now,
derive `changed ⊆ authorized`, and render changes such as
`config.paused: 0 → 1 @ byte 114` instead of unrelated base64 blobs. This is
useful for any compatible manifest producer before historical-signature replay
exists. When no instrumented touch map exists, do not claim the stronger
`changed ⊆ acquired ⊆ authorized` relation.

### 4. Ledger-bound Grillo producer (medium–large)

Let Grillo accept a mainnet transaction signature and obtain invocation-entry
and invocation-exit state, rather than trusting a supplied provenance label.
Solana's
[transaction status metadata](https://solana.com/docs/rpc/json-structures)
contains balances and token balances but not arbitrary pre/post account data,
so this requires deterministic SVM replay, a Geyser/archive sidecar, or an
account-history provider. Transaction-wide snapshots are insufficient for a
v0.2 PASS because sibling instructions can reverse a mutation. State that
dependency explicitly.

### 5. CVLR frame-condition spike (medium)

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
unless explicitly optimistic loops are enabled. Gate this spike on one
instruction terminating with non-optimistic loops and producing a useful
counterexample when a forbidden write is injected.

### 6. Verification-lane parity (medium)

Add strict-provenance Miri under Tree and Stacked Borrows where supported, then
add Kani proofs for range bounds, overlap rules, discriminators, failed-borrow
rollback, and `MIN_DATA_LEN` edge cases inspired by Anchor's current tail-slab
fix. Publish exact toolchain, enumeration command, and captured proof counts.

### 7. Controlled size attribution (small)

Rebuild one fixed fixture with the ambient write guard, touch map, effect
manifest, and diagnostics independently toggled. Archive ELF, section sizes,
source hashes, lockfile, compiler/Solana toolchain, and commands. This converts
the current 1,400-byte working-session diagnosis into evidence and identifies
where optimization actually pays.

## 10. Do not build

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

## 11. Reproduction ledger

This refresh used:

- Agave `d25f148` (2026-09-05), with v4.2.2 retained where a released tag was
  required, for slot parameters, feature IDs, stack depth, loader state,
  deployment flow, and account-lock types;
- the SIMD repository at `4828b2d`, the 21 PRs open on 2026-09-03, and the 22
  open-PR repeat query on 2026-09-06;
- finalized feature-account and sysvar queries to mainnet, with devnet/testnet
  checks for the 200/250 ms and direct-pointer gates;
- source-pinned default/release branches for Anchor, Quasar, Pinocchio, pina,
  zeropod, Star Frame, Steel, Typhoon, Parallax, LiteSVM, Light Protocol,
  QEDGen/qedsvm, Soroban, and Certora;
- Mainnet rent queries at slots 444,762,799, 444,767,908, and 444,822,656 for the final
  165,944-byte Cicada diagnostic artifact;
- the tracked `audit/framework-matrix-2026-08-16.json` archive for the only
  size numbers treated as reproducible facts.

Dates are part of every network, count, release, and repository-activity claim.
Re-run this ledger before carrying those claims into external material.
