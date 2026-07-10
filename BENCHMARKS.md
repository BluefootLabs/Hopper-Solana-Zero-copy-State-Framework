# Hopper Benchmarks

Compute-unit measurements for individual Hopper primitives and for
cross-framework parity workloads on Solana.

## How Benchmarks Work

Two measurement methods appear in this document, and they are not
interchangeable:

- **Mollusk net-of-logging (current, 2026-07-07).** The primitive lab runs
  under `mollusk-svm` (validator-free). Each primitive is dispatched between
  two `sol_log_compute_units()` syscalls; the runner records the whole
  instruction's `compute_units_consumed` (**whole-ix**) and the bracketed
  delta minus the empty-bracket overhead measured by a dedicated probe
  instruction (**net**) — the closest estimate of the primitive alone.
- **Validator-log deltas (historical, April 2026).** Earlier tables took the
  raw delta between the two log lines from `solana-test-validator 2.1`
  transaction logs. Those figures are retained below only to mark which
  claims they replace; they are not comparable to the net column and are no
  longer release-facing.

The primitive benchmark program lives in the sibling
[`hopper-bench`](https://github.com/BluefootLabs/hopper-bench) product repo.
From this framework workspace, `hopper profile bench` still knows how to run
the primitive lab; cross-framework orchestration, Docker runners, baselines,
and raw artifacts are owned by the benchmark repo so release docs never drift
from the executable harness.

All CU numbers are toolchain- and runtime-relative: identical Anchor code has
been observed to move 571 → 685 CU between Solana 2.1 and 2.3, and Mollusk CU
parity with mainnet is governed by the configured `SVMFeatureSet`. Compare
numbers only within one provenance block. A refresh on the agave-4.0 Mollusk
stack (0.13.x, SIMD-0339 active) is queued.

## Automation Status

The benchmark program defines instruction discriminators `0..=18` for
primitives, plus dispatch and measurement-overhead probes (`19..=21`) used by
the Mollusk runner. All primitives are covered by the benchmark repo's runner
and by the host `hopper profile bench` path. Release gates consume the
benchmark repo's baselines and artifacts; this framework repo keeps only
lightweight fixtures and historical result snapshots.

## Release-Facing Benchmark Policy

Release-facing comparison tables in this repository must come from one
`hopper-bench` run that uses the same lockfile, SBF toolchain, Mollusk version,
seed set, feature flags, release profile, and command line for every included
framework.

The current vault snapshot includes Hopper, an in-tree Anza Pinocchio target,
Quasar's upstream `examples/vault` target, and an in-tree Anchor comparator.
Quasar's upstream vault exposes only `deposit` and `withdraw`, so
validation-only rows are shown as `n/a` for Quasar instead of being
synthesized by the harness.

## Primitive CU Results (Mollusk, 2026-07-07)

Measured with the `primitive-bench` Mollusk runner (mollusk-svm 0.10.3).
`whole-ix` includes dispatch, fixture checks, and the logging brackets;
`net` subtracts the measured empty-bracket overhead (101 CU, probe disc 21).
The `April 2026` column is the superseded validator-log figure each row
replaces (different method — see above).

| Disc | Operation | Whole-ix CU | Net CU | April 2026 (superseded) | Category |
|------|-----------|------------:|-------:|------------------------:|----------|
| 0 | `check_signer` | 488 | 3 | ~20 | Validation |
| 1 | `check_writable` | 489 | 3 | ~20 | Validation |
| 2 | `check_owner` | 498 | 13 | ~50 | Validation |
| 3 | `Vault::load()` (T1 full check) | 524 | 29 | ~120 | Account loading |
| 4 | `check_keys_eq` | 523 | 14 | ~40 | Validation |
| 5 | `Vault::overlay()` (57 bytes) | 500 | 1 | ~8 | Memory access (Tier A) |
| 6 | `write_header` | 508 | 5 | ~30 | Account init |
| 7 | `zero_init` (57 bytes) | 524 | 21 | ~15 | Account init |
| 8 | `check_account_fast` | 489 | 5 | ~12 | Validation (fast path) |
| 9 | `emit_event` (32-byte payload) | 690 | 240 | ~100 | Events |
| 10 | `TrustProfile::load` (Strict) | 524 | 27 | ~130 | Trust loading |
| 11 | `pod_from_bytes` (57 bytes) | 500 | 1 | ~6 | Memory access (Tier B) |
| 12 | `StateReceipt::begin + commit` | 3291 | 2784 | ~50 | Receipts |
| 13 | `read_layout_id` + compare | 503 | 4 | ~15 | Fingerprint check |
| 14 | `StateSnapshot::capture + diff` | 532 | 26 | ~30 | State tracking |
| 15 | `overlay_mut` + field write | 507 | 4 | ~10 | Memory access (Tier A mut) |
| 16 | `raw_cast_baseline` (unsafe ptr) | 500 | 1 | ~4 | Competitor baseline |
| 17 | `StateReceipt` (enriched fields) | 3292 | 2786 | ~80 | Receipt (all fields) |
| 18 | `receipt + emit` (64B log) | 3649 | 3141 | ~150 | Receipt + event |
| 19 | `proc_macro_typed_dispatch` | 683 | 188 | — | Macro dispatch |

Note on the key-compare rows: all 32-byte key compares were rerouted to
4×u64 word-compare `PartialEq` on 2026-07-07 (the G1 pass). The `check_keys_eq`
14 CU and `check_owner` 13 CU rows above are measured **post-G1**; the April
~40 / ~50 figures are pre-G1 and retired.

Note on the receipt rows: the April validator-log method under-bracketed the
receipt cycle badly. The table-derived split is honest: the snapshot + diff
core is ~26 CU net (disc 14); the full receipt framing and encode dominates
at ~2.8k CU (disc 12/17), and adding emission lands at ~3.1k CU (disc 18).

## Memory Access Tier Comparison

Net CU, Mollusk 2026-07-07 run:

| Tier | Operation | Net CU | What you get |
|------|-----------|-------:|-------------|
| Raw (unsafe) | `raw ptr cast` | 1 | Size check + pointer cast only. **Competitor baseline** |
| B (pod) | `pod_from_bytes` | 1 | Bounds-checked typed view |
| A (safe) | `Vault::overlay()` | 1 | Header + layout_id + bounds check |
| A (mut) | `overlay_mut` + field set | 4 | Mutable overlay + write |
| Full load | `Vault::load()` | 29 | Owner + disc + version + layout_id + size |
| Strict trust | `TrustProfile::load` | 27 | Full cross-program trust validation |

### The Performance Story

**Hopper's safe overlay costs what a raw pointer cast costs.**

This is now a measured claim, not a rounding argument: in the 2026-07-07
Mollusk lab, the raw unsafe cast baseline and Hopper's safe, validated
overlay both measure **1 CU net**. The bounds check, header validation, and
layout-fingerprint verification disappear into the same measured cost as
`*const u8 as *const T`.

For hot paths where accounts are already validated, use Tier A overlay. For
cold paths, use `Vault::load()` at 29 CU net for full protocol-grade
validation (owner + disc + version + layout_id + size). The cost of safety
scales with how much safety you need — and at the overlay tier it is
measured at zero premium.

## Validation Cost Breakdown

Net CU, Mollusk 2026-07-07 run:

| Check | Net CU | Purpose |
|-------|-------:|---------|
| `check_signer` | 3 | Verify account is a signer |
| `check_account_fast` | 5 | Fused fast-path account check |
| `check_writable` | 3 | Verify account is writable |
| `check_owner` | 13 | Compare owner against program_id (post-G1 word compare) |
| `check_keys_eq` | 14 | Compare two account keys (post-G1 word compare) |
| Full T1 load | 29 | All checks: owner + disc + version + layout_id + size |
| Strict trust load | 27 | TrustProfile with all validations |

## Receipt and Tracking Overhead

Net CU, Mollusk 2026-07-07 run:

| Operation | Net CU | Notes |
|-----------|-------:|-------|
| `StateSnapshot::capture + diff` | 26 | Snapshot + diff without receipt framing |
| `read_layout_id` + compare | 4 | 8-byte fingerprint verification |
| `StateReceipt::begin + commit` | 2,784 | Full snapshot + diff + encode cycle |
| `StateReceipt` (enriched) | 2,786 | + phase, compat_impact, validation, migration |
| `receipt + emit` | 3,141 | Full cycle: begin + set + commit + emit |
| `emit_event` (32 bytes) | 240 | Log-based event emission |

A complete audit trail of every state mutation — full enriched receipt plus
emission — measures ~3,141 CU net, about **1.6% of a 200,000 CU instruction
budget**. Lightweight state tracking (snapshot + diff + fingerprint check)
costs ~30 CU. Receipts remain a reasonable default for audit-sensitive state
changes; CU-critical one-shot programs can use the snapshot/diff core alone.

The April validator-log figures for receipts (~50/~80/~150 CU) are retired:
they were captured with a different bracketing method that under-measured
the encode path, and they should not be quoted.

## Competitor-Shaped Baselines

Net CU, Mollusk 2026-07-07 run:

| Framework Style | Equivalent Net CU | What It Does |
|----------------|------------------:|---------------|
| Quasar / raw-cast | 1 | `ptr as *const T`, no validation |
| Steel / podded | 1 | Bounds-checked `Pod` cast |
| **Hopper overlay** | **1** | **Header + layout_id + bounds** |
| Anchor / borsh | ~500–2000 | Deserialization + clone |

The safe overlay and the raw cast are measured at the same net cost. The
validation Hopper adds at this tier is free at measurement resolution; the
difference against Anchor-style deserialization remains orders of magnitude.

## Running Benchmarks

```bash
# Primitive lab from this framework workspace
hopper profile bench

# Cross-framework parity labs from the sibling benchmark checkout
cd ../hopper-bench
./measure.sh all
```

The benchmark lab builds the Hopper benchmark program, provisions
deterministic fixture accounts, executes each primitive under Mollusk,
parses bounded `sol_log_compute_units()` deltas, and emits JSON/CSV/Markdown
artifacts in the benchmark repo's results directory.

Golden baselines, Docker runners, competitor locks, CI thresholds, and the
long-form benchmark roadmap are maintained in the sibling `hopper-bench` repo.

## Framework Parity Benchmark (Vault, 8-seed average)

Measured with the sibling `hopper-bench` Mollusk parity harness on
**2026-07-09** (post-G1 key-compare lowering, the fused single-pass
entrypoint walk, the mutation-complete lamport gate, and the
tag-arithmetic error lowering — see the provenance note under the table
for what each recent change costs or saves). Every included framework used
the same deterministic user seed set, SBF toolchain, runner, and command
line. `n/a` means the upstream comparator does not implement that benchmark
instruction.

A version note on the Anchor column: it is measured against
**anchor-lang 0.31.1**, the comparator this table was locked to.
`anchor-lang` 1.1.2 is the current stable release (a 1.1.2 re-run is queued,
and its binary-size row is expected to shrink), and an unreleased,
Pinocchio-based **Anchor v2 alpha** exists whose in-repo benchmarks land at
Quasar-level CU. Read the Anchor multiples below as measurements of shipped
Anchor 0.31.1/1.x, with the shelf life that implies.

| Scenario | Hopper | Anza Pinocchio | Quasar | Anchor 0.31.1 |
|----------|-------:|---------------:|-------:|--------------:|
| Authorize | **420 CU** | 2512 CU (+2092) | n/a | 5017 CU (+4597) |
| Auth-fail (missing sig) | 66 CU | **41 CU** (−25) | n/a | 2284 CU (+2218) |
| Counter (segment-safe) | **518 CU** | 2539 CU (+2021) | n/a | 5156 CU (+4638) |
| Deposit | **1653 CU** | 3856 CU (+2203) | 1756 CU (+103) | 7150 CU (+5497) |
| Withdraw | **486 CU** | 2548 CU (+2062) | 592 CU (+106) | 5108 CU (+4622) |
| Unsigned withdraw | rejected | rejected | rejected | rejected |
| Binary size (`.so`) | 7.46 KiB | 7.73 KiB | **5.47 KiB** | 190.11 KiB |

> **Full four-way re-measured 2026-07-09** — every column through the same
> mollusk 0.10.3 host runner in one run: Hopper at HEAD, the in-tree Anza
> Pinocchio target, Quasar's own prebuilt vault, and the anchor-lang 0.31
> comparator rebuilt from the same pinned lockfile (its rows reproduced the
> 07-02 values exactly, confirming runner stability). Hopper `.text` is
> 6,016 B (5.88 KiB) of the 7.46 KiB file.
>
> Deltas vs the 2026-07-02 Hopper column, each one a priced decision:
> Withdraw 442 → 486: **+44 CU is the lamport gate (mutation-complete
> write-sets) actually enforcing** on the one lamport-moving instruction —
> a measured safety feature no other column has (the error-lowering's +8
> was recovered by the gate-check fast-out; Deposit's +3 and Auth-fail's
> +5 remain, the price of the 10% `.text` cut). Every Quasar-comparable
> row still wins.
>
> Size: this week a **P0 was found by loading, not building** — the gate's
> `static mut` made every `lamports(...)` program fail the SBF loader's
> no-writable-sections rule; it now lives in the reserved, zero-initialized
> VM heap and Hopper programs carry **zero writable sections**. With ~5 KiB
> of accidentally-linked `core::fmt` also eliminated, the vault's `.so` is
> now **smaller than Pinocchio's (7.46 vs 7.73 KiB) on the identical
> contract**. Quasar's 5.47 KiB still wins this row; the remaining delta is
> increasingly *paid-for* structure (receipts, the byte-range ledger, the
> lamport gate — outlining experiments that "saved" bytes cost +44..+73 CU
> and were rejected; see the in-source measurement notes). We do not
> publish a size lead we have not measured.

The Pinocchio column is built in-tree from the benchmark repo's Anza
Pinocchio target, not borrowed from Quasar's reference sample or an older
"Pinocchio-style" proxy number. The Anchor column is the benchmark repo's
in-tree `anchor-vault` implementing the identical instruction contract
(explicit one-byte-style discriminators via Anchor's `discriminator`
attribute so the harness drives all four programs the same way).

## Router Parity Lab — first three-way numbers (2026-07-07)

This is, to our knowledge, the first published router-class head-to-head
between zero-copy Solana frameworks. The workload (contract:
`hopper-bench/ROUTER_CONTRACT.md` v1) is a multi-hop swap router over a
shared mock-AMM CPI target: 1–3 hops, measured amount forwarding (hop *i+1*
input is the router's measured user-lamport delta from hop *i*, never the
venue-reported figure), and a min-out safety gate exercised at its boundary
on every success row. A framework that lets a min-out violation through is
disqualified from publication; all three passed.

| Row | Hopper | Quasar | Pinocchio (hand-written) |
|---|---:|---:|---:|
| swap_1hop | **1,564 CU** | 1,582 CU (+18) | 1,523 CU (−41) |
| swap_2hop | **3,044 CU** | 3,064 CU (+20) | 2,975 CU (−69) |
| swap_3hop | **4,525 CU** | 4,546 CU (+21) | 4,431 CU (−94) |
| Binary size | **10.05 KiB** | 11.05 KiB | 10.98 KiB |
| min-out gate | rejected | rejected | rejected |

Reading it honestly:

- Hand-written Pinocchio wins the CU rows, as it should — it carries no
  framework surface. Hopper lands within **2.1–2.7%** of it (+41/+69/+94 CU
  across the hops) while carrying full framework validation, state contracts,
  and tooling, and ships the **smallest binary of the three**.
- **Hopper beats Quasar on every row** (−18/−20/−21 CU) with a smaller
  binary. The morning run (pre-entrypoint-fuse:
  `results/router-parity-2026-07-07-threeway/`) had Hopper trailing Quasar
  by +34/+56/+79 CU; the fused single-pass account walk — an
  instruction-level audit finding, fixed and re-measured the same day —
  flipped every row. Both snapshots are kept so the delta is checkable.
- Every framework's measured CU includes one identical mock-AMM invocation
  per hop, so the deltas isolate router-side framework overhead.

Results: `hopper-bench/results/router-parity-2026-07-07-post-ep/` (current)
and `router-parity-2026-07-07-threeway/` (same-day pre-fix snapshot). Vault:
`framework-vaults-2026-07-07-post-ep/`.

### Benchmark provenance checklist

Every parity result published from `hopper-bench` must record:

- Hopper framework commit and benchmark repo commit.
- Quasar source commit or release tag.
- Pinocchio crate versions when the Pinocchio column is included.
- Rust, Solana/Agave SBF, and Mollusk versions.
- Exact feature flags and release profile.
- Exact reproduction command and seed count.

### Current benchmark provenance (2026-07-07 runs)

Shared toolchain for all three 2026-07-07 runs (vault four-way, router
three-way, primitive lab):

| Field | Value |
|---|---|
| Hopper framework checkout | working tree on `fa2bfdb` (post-G1 word-compare, fused single-pass entrypoint, mid-tier CPI) |
| Quasar checkout | `37e8a6b` clean (upstream 2026-06-28) |
| Anchor | `anchor-lang 0.31.1` (crates.io, locked), in-tree `anchor-vault` comparator |
| Rust | rustc 1.96.0 (workspace pin) |
| SBF toolchain | `cargo-build-sbf 4.0.0`, platform-tools `v1.53` |
| SVM harness | `mollusk-svm 0.10.3` |
| Samples | 8 deterministic user seed cases per parity row |

Per-run result files:

| Run | Result files |
|---|---|
| Vault four-way | `hopper-bench/results/framework-vaults-2026-07-07-post-ep/vault-framework-comparison.{json,csv}` |
| Router three-way | `hopper-bench/results/router-parity-2026-07-07-post-review/router-framework-comparison.{json,csv}` |
| Primitive lab | `hopper-bench/results/primitive-bench/primitive-cu.{md,csv}` |

Previous published runs, kept so deltas stay checkable: 2026-07-02
(post-I10, Hopper `1d10d04`): 466/107/564/1713/488 at 7.46 KiB —
functionally identical to the 2026-07-07 row (deposit +1 CU is noise;
G1's win is structural, not vault-visible). 2026-05-25 (Hopper `300797d`,
Quasar `5fda2f5`): Hopper 431/72/551/1669/453 CU at 7.53 KiB; Quasar
deposit 1767 / withdraw 603 at 6.27 KiB — **retired**, see the bisect
section below for why those numbers were un-deployable.

### Performance observations

- **Anchor, measured (2026-07-09):** Hopper is ~11.9× cheaper on
  `authorize`, ~4.3× on `deposit`, ~10.5× on `withdraw`, and the artifact
  is **26× smaller** (7.46 vs 190.11 KiB). Anchor's failure path is also
  expensive: a missing signer costs 2284 CU (8-byte discriminator hash +
  full `try_accounts` before the signer check) vs Hopper's 66 CU. **Shelf-life caveat:** these
  multiples apply to shipped Anchor 0.31.1/1.x. The unreleased Anchor v2
  alpha benchmarks at Quasar-level CU in its own repo, so when it ships,
  "10× cheaper than Anchor" stops being a durable headline for any
  framework. The durable ground is winning within the zero-copy cluster
  (see the router lab above) plus the state/safety/tooling surface no
  Pinocchio-derived framework has (see `docs/THE_MOAT.md`).
- Hopper beats Quasar on **both** upstream Quasar workloads (deposit
  −106 CU, withdraw −150 CU) while carrying its full state-contract
  surface. Quasar publishes no comparative CU benchmark of its own; this
  pinned, provenance-checked matrix is currently the only published
  cross-framework table that includes it.
- Hopper is lower-CU than the in-tree Anza Pinocchio parity target on the
  measured PDA-bearing success paths in this vault contract. Treat that as a
  result for this benchmark, not a universal "faster than Pinocchio" claim —
  the router lab above is the fairer overhead measurement, and there
  hand-written Pinocchio wins by 2.1–2.7%.
- Quasar's upstream vault does not implement `authorize` or `counter_access`,
  so those rows are intentionally absent for Quasar.

### The +13…+44 CU delta vs 2026-05-25, bisected and resolved

Hopper's rows moved between the 2026-05-25 and 2026-07-02 runs (authorize
431→466, auth-fail 72→107, counter 551→564, deposit 1669→1713, withdraw
453→488; binary 7.53→7.46 KiB). An automated `git bisect run` (build the
parity vault at each candidate, measure with the pinned runner; the runner
itself reproduces the May numbers bit-for-bit on the May commit) pinned
the **entire** delta to one commit: `8899e99`, which feature-gated the
`r2` fast entrypoint behind `simd-0321`.

**This is not a regression — it is the removal of an unsound
optimization.** Before `8899e99`, `fast_entrypoint!` unconditionally
read the instruction-data pointer from the SVM's second entrypoint
register. SIMD-0321 — the proposal that populates that register — is
**not activated on any public cluster**; the fast path only worked in
local SVMs that happen to pass `r2`. The May numbers were therefore
~30–40 CU better than any mainnet deployment could actually achieve.
Today's table is the honest, deployable number; the delta is exactly
the account-scanning pass the sound entrypoint performs, and it comes
back the day the SIMD-0321 gate activates (rebuild with the feature).

Two corollaries the bisect proved along the way:

- **I10 (fused signer/writable validation), I7 (touch maps), and I12
  (write policies) cost 0 CU** — the parity vault measures identically
  at the pre-I10 commit `411790f` and at current head, across all five
  scenarios and binary size. The earlier suspicion of I10's
  failure-path fallback was wrong.
- The widened auth-fail gap to Pinocchio (−31 → −66 CU) is the same
  entrypoint story: their 41 CU rejection is measured with their
  scanning entrypoint too, so the honest comparison is 66 vs 41 (as of
  2026-07-09; 61 pre-error-lowering) — the scanning pass plus Hopper's
  dispatch reaching the fused check. Still the only vault row Pinocchio
  wins.

### Reading the Pinocchio deltas honestly

The large CU gaps on `deposit`, `withdraw`, `authorize`, and
`counter_access` are **mostly a PDA-strategy difference, not a substrate
difference**. The in-tree Pinocchio target uses idiomatic
`find_program_address` (a bump search that can cost ~1,500–2,500 CU),
whereas the Hopper parity vault verifies a **stored canonical bump** with
a single `create_program_address` (~200 CU). A Pinocchio program that
also stored its bump would land close to Hopper on these rows — the
router lab above, where the Pinocchio comparator is hand-optimized, shows
exactly that shape.

So the accurate claim is **"Hopper is fast by default"**: the cheap PDA
path is the one the macros and conventions steer you toward, while raw
Pinocchio leaves that optimization to the author. It is not "raw
Pinocchio is slower."

### Architecture and DX observations

- Verify-only PDA avoids `sol_curve_validate_point` by comparing hashes directly
  against the known PDA address. This is the stored-bump path described above.
- The fast entrypoint receives instruction data via the second SVM register
  (`r2`). This depends on **SIMD-0321**, whose feature gate is not yet active on
  public clusters, so it is opt-in behind the `simd-0321` cargo feature; with
  the feature off, `fast_entrypoint!` is the standard scanning entrypoint and
  these numbers are measured without the `r2` shortcut. `hopper feature-gate`
  compares a build's configuration against the live cluster gate account.
- **Tuned intrinsics (`hopper-builtins`, experimental, opt-in):** the
  `crates/hopper-builtins` crate overrides `memcmp`/`bcmp`/`memcpy`/`memset`
  with ordering-correct word-wise routines. Vault CU is identical with the
  feature on (+0.41 KiB size); the value is runtime-length memory ops, which
  LLVM lowers to `bcmp` — a symbol platform-tools does not even provide.
  Linking currently requires
  `RUSTFLAGS="-C link-arg=--allow-multiple-definition"`. For contrast:
  Quasar's vendored `solana-compiler-builtins` overrides only `memcmp`, is
  gated on `target_arch = "bpf"` so it is inert on the standard
  `cargo build-sbf` route, and `rust-lld` refuses its strong-vs-strong symbol
  collision on that route.
- Hopper's claim is not "raw Pinocchio is slower." The claim is that Hopper
  packages low-overhead account access with framework validation, schema,
  lifecycle, CPI, and CLI tooling.

### Where Pinocchio is still the right choice

Use raw Pinocchio directly when the target program should remain a minimal
manual substrate with no framework-owned account lifecycle, schema, CLI, or
validation layer. Hopper is the framework-layer option when those surfaces are
worth carrying.

The parity vault source is at
[`examples/hopper-parity-vault`](examples/hopper-parity-vault/src/lib.rs).
The cross-framework runners live in the sibling `hopper-bench` repo.


## CU Budget Reference

Per-account framework cost from the Mollusk net rows above: full validated
load 29 CU + overlay access ~1 CU + fingerprint re-check 4 CU ≈ **~35 CU per
account**. Lightweight state tracking (snapshot + diff) adds ~26 CU; a full
emitted receipt adds ~3.1k CU where an audit trail is wanted.

| Scenario | Typical CU | Hopper overhead (loads + overlays + diff) |
|----------|-----------|-------------------------------------------|
| Simple transfer (1 account) | ~5,000 | ~60 CU |
| DeFi swap (3 accounts) | ~50,000 | ~130 CU |
| Complex instruction (6 accounts) | ~150,000 | ~240 CU |

In all scenarios, core Hopper overhead is well under 1% of the instruction
budget; opting into full receipt emission adds ~1.6% of a 200k budget.


## Deploy-cost economics

Rent-exempt deploy cost follows `(elf_bytes + 128) × 6,960` lamports
(formula verified against a live mainnet program-account balance; see
`docs/audit/GAP_CLOSURE_AND_INNOVATION_2026.md`, section 2). Applied to the
2026-07-07 vault matrix and the devnet counter:

| Artifact | Size | Rent at deploy |
|---|---:|---:|
| Hopper counter (2026-07-09 build) | 3,736 B | **≈ 0.027 SOL** |
| Hopper counter (devnet artifact, 07-07) | 4,688 B | ≈ 0.034 SOL |
| Hopper vault (2026-07-09) | 7.46 KiB | ≈ 0.054 SOL |
| Quasar vault | 5.47 KiB | ≈ 0.040 SOL |
| Pinocchio vault | 7.73 KiB | ≈ 0.056 SOL |
| Anchor 0.31.1 vault | 190.11 KiB | **≈ 1.356 SOL (~25× Hopper)** |

Unlike CU tables, deploy rent does not churn with runtime versions: it is a
durable cost axis, and the Anchor-class artifact is the outlier on it.


## Devnet deployment evidence (this pass)

Built with `cargo build-sbf` on the Anza toolchain and deployed to devnet
from authority `HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT`. Bytecode
sizes are the on-disk `.so` artifacts that were deployed.

| Example | SBF `.so` bytes | Devnet program id |
|---------|----------------:|-------------------|
| counter | 4 688 | `D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F` |
| escrow | 18 736 | `5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN` |
| versioned-state (migration) | 25 664 | `EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt` |
| orderbook | 18 408 | `CK3XYYsbFducx9UEEWWLGAVnSAhGkMtM1TKLe8PDP6dJ` |
| virtual-state | 23 240 | `6MmtjcdZuGZyceETKB2pstfSZ8Pv5r72U7dZrBCzgehz` |

The 4 688-byte counter was the headline size claim as deployed; the same
example rebuilt at HEAD (2026-07-09, after the writable-section P0 fix and
the `core::fmt`/error-lowering size work) measures **3,736 bytes ≈ 0.027
SOL** — a complete, deployable zero-copy program — `#[account]` layout,
`#[derive(Accounts)]` context with a `has_one` constraint, single-byte
dispatch, and a checked mutation — in under 4 KiB of bytecode. The devnet
program id above still runs the 4,688-byte 07-07 artifact until
redeployed.

### Measured on-chain compute

The escrow `make` instruction (self-initializes a fresh `Escrow` account
via the `init` lifecycle, then writes four typed fields) consumed
**1 761 CU** on devnet, decoded from the confirmed transaction by
`hopper explain` against the checked-in escrow manifest.

### Bytecode size vs the competitor sources

Compared against the extracted competitor trees at
`competitors/pinocchio_src/` and `competitors/quasar_src/` (read-only,
not vendored). Hopper's per-program bytecode is dominated by the
framework's account-loading and validation surface; the counter figure
above shows that surface compiles down to a few KiB rather than the tens
of KiB an Anchor-style framework adds. The cross-framework CU runner that
produces same-lockfile, same-toolchain comparison rows lives in the
sibling `hopper-bench` repo, as described in the release-facing benchmark
policy above; the numbers in this section are the artifacts and on-chain
measurements produced directly in this devnet pass.
