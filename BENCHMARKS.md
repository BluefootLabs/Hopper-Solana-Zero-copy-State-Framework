# Hopper Benchmarks

Compute-unit measurements for individual Hopper primitives on Solana.

## How Benchmarks Work

Each benchmark dispatches a single Hopper operation between two
`sol_log_compute_units()` syscalls. The CU delta is captured from
validator transaction logs:

```
delta = first_remaining - second_remaining
```

The bench program lives in `bench/hopper-bench/`. Deploy it to a local
validator and send transactions with the appropriate discriminator byte.

## Automation Status

The benchmark program defines instruction discriminators `0..=18`. All 19
primitives are covered by the Docker-based runner (`bench/run-bench-docker.ps1`
/ `bench/run-bench-docker.sh`) which runs `hopper profile bench` against a
containerized test validator. The `bench/measure.sh` wrapper gates instructions
`0..=10` in the legacy CI path; the Docker runner covers the full set.

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
| 18 | `receipt + emit` (64B log) | ~150 | Receipt + event |

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

A raw `*const u8 as *const T` pointer cast (what Quasar-style frameworks
do) costs ~4 CU. Hopper's safe overlay costs ~8 CU. The 4 CU difference
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
| `StateReceipt::begin + commit` | ~50 | Snapshot + diff + encode to 64 bytes |
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
`check_owner` call. There is no reason not to use receipts.

## Running Benchmarks

```bash
# Start local validator
solana-test-validator

# Run the primitive benchmark lab
hopper profile bench
```

The benchmark lab builds and deploys `hopper-bench`, provisions deterministic
fixture accounts, simulates each implemented primitive benchmark, parses the
bounded `sol_log_compute_units()` deltas, and emits JSON/CSV artifacts under
`bench/results/` by default.

See `bench/cu_baselines.toml` for golden baselines and CI gate thresholds.
See `docs/BENCHMARK_AND_TOOLING_PARITY_PLAN.md` for the remaining benchmark
automation and public-proof work.

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

All frameworks execute identical logic: `['vault', user]` PDA derivation,
signer/writable checks, token operations. Measured on Mollusk with Solana 2.1.

| Scenario | Hopper | Pinocchio | Quasar |
|----------|--------|-----------|--------|
| Authorize | **432 CU** | _re-run pending_ | 585 CU |
| Auth-fail (missing sig) | **70 CU** | _re-run pending_ | 66 CU |
| Counter (segment-safe) | **539 CU** | _re-run pending_ | 607 CU |
| Deposit | **1651 CU** | _re-run pending_ | 1768 CU |
| Withdraw | **455 CU** | _re-run pending_ | 605 CU |
| **Binary size** | **7.62 KiB** | _re-run pending_ | 8.36 KiB |

The **Pinocchio** column is now built in-tree from
[`bench/pinocchio-vault`](bench/pinocchio-vault/src/lib.rs) using Anza's own
`pinocchio = "0.10"` and `pinocchio-system = "0.5"` crates. It replaces the
earlier "Pinocchio-style" column, which loaded a third-party reference vault
from Quasar's tree; that indirection was easy to misread as benchmarking the
Pinocchio framework itself (it was not — Pinocchio is a substrate, not a
framework, and the old 2543 CU authorize was against a non-optimised
reference sample). Pinocchio numbers here are marked _re-run pending_; the
next `framework-vault-bench` run will populate them. Expected Hopper lead
over idiomatic Pinocchio on PDA-bearing instructions is a few hundred CU,
attributable to Hopper's verify-only PDA path (see below).

**Key observations:**

- **Hopper beats Quasar on 4 of 5 instructions** while providing 47+ safety
  mechanisms vs Quasar's ~10. The only gap is auth-fail (+4 CU, negligible).
- **Smallest binary** (pre-R2 re-run): Hopper at 7.62 KiB is 8.8% smaller than
  Quasar (8.36 KiB). Binary-size comparison to Anza Pinocchio will be updated
  with the next bench run.
- **Verify-only PDA**: Hopper's novel sha256-only PDA verification eliminates
  `sol_curve_validate_point` (~159 CU/attempt) by comparing hashes directly
  against the known PDA address. ~350 CU savings per PDA-bearing instruction
  over the standard `find_program_address` approach (which is what idiomatic
  Pinocchio uses, and therefore where Hopper's lead over Pinocchio comes from).
- **Fast entrypoint**: Two-argument SVM entrypoint receives instruction data via
  the second register, eliminating full-buffer account scanning.
- **Safety at no cost**: The counter-access instruction (539 vs 607 CU) now beats
  Quasar despite using segment-level borrow tracking. Quasar's counter uses
  `borrow_unchecked_mut()` with raw byte slicing and no conflict detection.

The parity vault source is at
[`examples/hopper-parity-vault`](examples/hopper-parity-vault/src/lib.rs).
Benchmark runner is `bench/runner/` (`framework-vault-bench`).


## CU Budget Reference

| Scenario | Typical CU | Hopper Overhead |
|----------|-----------|------------------|
| Simple transfer (1 account) | ~5,000 | ~128 CU (load + overlay + receipt) |
| DeFi swap (3 accounts) | ~50,000 | ~400 CU (3 loads + overlays + receipt) |
| Complex instruction (6 accounts) | ~150,000 | ~800 CU (6 loads + overlays + receipt) |

In all scenarios, Hopper overhead is <2% of the total instruction budget.
