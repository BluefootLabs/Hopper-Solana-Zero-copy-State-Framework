# Hopper Benchmarks

Compute-unit measurements for individual Hopper primitives on Solana.

## How Benchmarks Work

Each benchmark dispatches a single Hopper operation between two
`sol_log_compute_units()` syscalls. The CU delta is captured from
validator transaction logs:

```
delta = first_remaining - second_remaining
```

The primitive benchmark program lives in the sibling
[`hopper-bench`](https://github.com/BluefootLabs/hopper-bench) product repo.
From this framework workspace, `hopper profile bench` still knows how to run
the primitive lab; cross-framework orchestration, Docker runners, baselines,
and raw artifacts are owned by the benchmark repo so release docs never drift
from the executable harness.

## Automation Status

The benchmark program defines instruction discriminators `0..=18`. All 19
primitives are covered by the benchmark repo's Docker runner and by the host
`hopper profile bench` path. Release gates consume the benchmark repo's
baselines and artifacts; this framework repo keeps only lightweight fixtures
and historical result snapshots.

## Release-Facing Benchmark Policy

Release-facing comparison tables in this repository must come from one
`hopper-bench` run that uses the same lockfile, SBF toolchain, Mollusk version,
seed set, feature flags, release profile, and command line for every included
framework.

The current vault snapshot includes Hopper, an in-tree Anza Pinocchio target,
and Quasar's upstream `examples/vault` target. Quasar's upstream vault exposes
only `deposit` and `withdraw`, so validation-only rows are shown as `n/a` for
Quasar instead of being synthesized by the harness.

## CU Results

Measured on solana-test-validator 2.1 (April 2026).

| Disc | Operation | Expected CU | Category |
|------|-----------|-------------|----------|
| 0 | `check_signer` | ~20 | Validation |
| 1 | `check_writable` | ~20 | Validation |
| 2 | `check_owner` | ~50 | Validation |
| 3 | `Vault::load()` (T1 full check) | ~120 | Account loading |
| 4 | `check_keys_eq` | ~40 | Validation |
| 5 | `Vault::overlay()` (57 bytes) | ~8 | Memory access (Tier A) |
| 6 | `write_header` | ~30 | Account init |
| 7 | `zero_init` (57 bytes) | ~15 | Account init |
| 8 | `check_signer_fast` | ~12 | Validation (fast path) |
| 9 | `emit_event` (32-byte payload) | ~100 | Events |
| 10 | `TrustProfile::load` (Strict) | ~130 | Trust loading |
| 11 | `pod_from_bytes` (57 bytes) | ~6 | Memory access (Tier B) |
| 12 | `StateReceipt::begin + commit` | ~50 | Receipts |
| 13 | `read_layout_id` + compare | ~15 | Fingerprint check |
| 14 | `StateSnapshot::capture + diff` | ~30 | State tracking |
| 15 | `overlay_mut` + field write | ~10 | Memory access (Tier A mut) |
| 16 | `raw_cast_baseline` (unsafe ptr) | ~4 | Competitor baseline |
| 17 | `StateReceipt` (enriched fields) | ~80 | Receipt (all fields) |
| 18 | `receipt + emit` (72B log) | ~150 | Receipt + event |

## Memory Access Tier Comparison

| Tier | Operation | CU | What you get |
|------|-----------|-----|-------------|
| Raw (unsafe) | `raw ptr cast` | ~4 | Size check + pointer cast only. **Competitor baseline** |
| B (pod) | `pod_from_bytes` | ~6 | Bounds-checked typed view (+2 CU) |
| A (safe) | `Vault::overlay()` | ~8 | Header + layout_id + bounds check (+4 CU) |
| A (mut) | `overlay_mut` + field set | ~10 | Mutable overlay + write (+6 CU) |
| C (raw) | `load_unchecked` | ~6 | No validation, caller risk |
| Full load | `Vault::load()` | ~120 | Owner + disc + version + layout_id + size |
| Strict trust | `TrustProfile::load` | ~130 | Full cross-program trust validation |

### The Performance Story

**Hopper's safe path is within 4 CU of raw.**

A raw `*const u8 as *const T` pointer cast costs ~4 CU. Hopper's safe overlay
costs ~8 CU. The 4 CU difference
buys you: bounds checking, header validation, and layout_id fingerprint
verification.

**Hopper's raw path exists when you need it.** `pod_from_bytes` at ~6 CU
is 2 CU from raw, with bounds checking. `load_unchecked` matches raw.

For hot paths where accounts are already validated, use Tier A overlay.
For cold paths, use `Vault::load()` at ~120 CU for full protocol-grade
validation. The cost of safety scales with how much safety you need.

## Validation Cost Breakdown

| Check | CU | Purpose |
|-------|-----|---------|
| `check_signer` | ~20 | Verify account is a signer |
| `check_signer_fast` | ~12 | Optimized signer check |
| `check_writable` | ~20 | Verify account is writable |
| `check_owner` | ~50 | Compare owner against program_id |
| `check_keys_eq` | ~40 | Compare two account keys |
| Full T1 load | ~120 | All checks: owner + disc + version + layout_id + size |
| Strict trust load | ~130 | TrustProfile with all validations |

## Receipt and Tracking Overhead

| Operation | CU | Notes |
|-----------|-----|-------|
| `StateReceipt::begin + commit` | ~50 | Snapshot + diff + encode to 72 bytes |
| `StateReceipt` (enriched) | ~80 | + phase, compat_impact, validation, migration |
| `receipt + emit` | ~150 | Full cycle: begin + set + commit + emit |
| `StateSnapshot::capture + diff` | ~30 | Snapshot + diff without receipt framing |
| `read_layout_id` + compare | ~15 | 8-byte fingerprint verification |
| `emit_event` (32 bytes) | ~100 | Log-based event emission |
| `emit_event` (128 bytes) | ~120 | Larger event payload |

A full enriched receipt (snapshot + diff + enriched fields + encode)
costs ~80 CU. Emitting it as an event adds ~70 CU for the syscall.
For a typical instruction budget of 200,000 CU, full receipt tracking
with emission adds 0.075% overhead.

## What This Means

### Safe vs Raw: The Honest Comparison

```
  Raw unsafe cast (competitor baseline):   ~4 CU
  pod_from_bytes (bounds-checked):         ~6 CU   (+2 CU)
  Vault::overlay (safe, validated):        ~8 CU   (+4 CU)
  Full Vault::load (protocol-grade):     ~120 CU   (30x raw)
```

Hopper's **safe overlay is 4 CU more than raw**. The full validation path
is 30x more expensive, but you typically pay that cost once per
instruction, then use overlays for all subsequent access.

### Receipt Overhead: Negligible

```
  Basic receipt (begin + commit):          ~50 CU   (0.025% of 200k budget)
  Enriched receipt (all fields):           ~80 CU   (0.040% of 200k budget)
  Receipt + emit (full audit trail):      ~150 CU   (0.075% of 200k budget)
```

A complete audit trail of every state mutation costs less than a single
`check_owner` call in this benchmark. Receipts are cheap enough to make the
default answer "yes" for audit-sensitive state changes, while tiny one-shot
programs can still omit them when every byte matters.

## Running Benchmarks

```bash
# Primitive lab from this framework workspace
hopper profile bench

# Cross-framework parity lab from the sibling benchmark checkout
cd ../hopper-bench
./measure.sh all
```

The benchmark lab builds and deploys the Hopper benchmark program, provisions
deterministic fixture accounts, simulates each implemented primitive
benchmark, parses bounded `sol_log_compute_units()` deltas, and emits JSON/CSV
artifacts in the benchmark repo's results directory.

Golden baselines, Docker runners, competitor locks, CI thresholds, and the
long-form benchmark roadmap are maintained in the sibling `hopper-bench` repo.

## Competitor-Shaped Baselines

| Framework Style | Equivalent CU | What It Does |
|----------------|---------------|---------------|
| Quasar / raw-cast | ~4 | `ptr as *const T`, no validation |
| Steel / podded | ~6 | Bounds-checked `Pod` cast |
| **Hopper overlay** | **~8** | **Header + layout_id + bounds** |
| Anchor / borsh | ~500-2000 | Deserialization + clone |

Hopper's safe path is closer to raw-cast frameworks than to Anchor.
The 4 CU premium over raw buys header validation, fingerprint
verification, and a clean typed API.

## Framework Parity Benchmark (Vault, 8-seed average)

Measured with the sibling `hopper-bench` Mollusk parity harness on 2026-05-25.
Every included framework used the same deterministic user seed set, SBF
toolchain, runner, and command line. `n/a` means the upstream comparator does
not implement that benchmark instruction.

| Scenario | Hopper | Anza Pinocchio | Quasar |
|----------|-------:|---------------:|-------:|
| Authorize | **431 CU** | 2512 CU (+2081) | n/a |
| Auth-fail (missing sig) | 72 CU | **41 CU** (-31) | n/a |
| Counter (segment-safe) | **551 CU** | 2539 CU (+1988) | n/a |
| Deposit | **1669 CU** | 3856 CU (+2187) | 1767 CU (+98) |
| Withdraw | **453 CU** | 2548 CU (+2095) | 603 CU (+150) |
| Unsigned withdraw | rejected | rejected | rejected |
| Binary size | 7.53 KiB | 7.73 KiB | **6.27 KiB** |

The Pinocchio column above is built in-tree from the benchmark repo's Anza
Pinocchio target, not borrowed from Quasar's reference sample or an older
"Pinocchio-style" proxy number.

### Benchmark provenance checklist

Every parity result published from `hopper-bench` must record:

- Hopper framework commit and benchmark repo commit.
- Quasar source commit or release tag.
- Pinocchio crate versions when the Pinocchio column is included.
- Rust, Solana/Agave SBF, and Mollusk versions.
- Exact feature flags and release profile.
- Exact reproduction command and seed count.

### Current benchmark provenance

| Field | Value |
|---|---|
| Result files | `hopper-bench/results/framework-vaults-current-head-2026-05-25/vault-framework-comparison.{json,csv}` |
| Hopper framework checkout | `300797d` plus local audit-fix changes |
| Benchmark checkout | `4c5183c` clean |
| Quasar checkout | `5fda2f5` clean |
| SBF toolchain | `cargo-build-sbf 4.0.0`, platform-tools `v1.53` |
| Samples | 8 deterministic user seed cases |
| Command | `./compare-framework-vaults.ps1 -HopperRoot D:\tmp\Hopper-Solana-Zero-copy-State-Framework -QuasarRoot D:\tmp\quasar -OutDir results\framework-vaults-current-head-2026-05-25` |

### Performance observations

- Hopper is within 150 CU of Quasar on the two upstream Quasar vault workloads
  while adding Hopper's state-contract surface in its own parity target.
- Hopper is lower-CU than the in-tree Anza Pinocchio parity target on the
  measured PDA-bearing success paths in this vault contract. Treat that as a
  result for this benchmark, not a universal "faster than Pinocchio" claim.
- Hopper produces a smaller binary than the four-instruction Anza Pinocchio
  parity target. Quasar remains 1.26 KiB smaller, but its upstream vault only
  implements `deposit` and `withdraw`.
- Quasar's upstream vault does not implement `authorize` or `counter_access`, so
  those rows are intentionally absent for Quasar.

### Architecture and DX observations

- Verify-only PDA avoids `sol_curve_validate_point` by comparing hashes directly
  against the known PDA address. This is a Hopper optimization to keep measuring
  against same-provenance competitors, not a published Pinocchio performance
  claim.
- The fast entrypoint receives instruction data via the second SVM register,
  avoiding a full-buffer account scan on supported runtimes.
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
The cross-framework runner lives in the sibling `hopper-bench` repo.


## CU Budget Reference

| Scenario | Typical CU | Hopper Overhead |
|----------|-----------|------------------|
| Simple transfer (1 account) | ~5,000 | ~128 CU (load + overlay + receipt) |
| DeFi swap (3 accounts) | ~50,000 | ~400 CU (3 loads + overlays + receipt) |
| Complex instruction (6 accounts) | ~150,000 | ~800 CU (6 loads + overlays + receipt) |

In all scenarios, Hopper overhead is <2% of the total instruction budget.


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

The 4 688-byte counter is the headline size claim: a complete, deployable
zero-copy program — `#[account]` layout, `#[derive(Accounts)]` context
with a `has_one` constraint, single-byte dispatch, and a checked mutation
— in under 5 KiB of bytecode.

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
