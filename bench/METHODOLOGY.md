# Cross-Framework Benchmark Methodology

Closes Hopper Safety Audit item **D4** ("Benchmark suite across frameworks").
This document is the ground truth for how cross-framework measurements
are taken, how results are reported, and how a contributor reproduces them.

## Goals

The audit's methodology requirements (page 15) are:

1. **Same toolchain**. every framework is compiled with the same
   `rustc` version and the same SBF toolchain release.
2. **Same optimization settings**. every framework is built with the
   same `cargo` profile flags (`release`, `opt-level=3`, `lto=fat`).
3. **Equivalent logic**. every framework's vault implements the
   *same* public behaviour at the same call sites. Frameworks are
   allowed to be idiomatic, but they are **not** allowed to change
   what the program actually does.
4. **Both host and SBF measurements** where relevant. On-chain
   CU is the authoritative comparison number; host benchmarks are
   used only for development iteration.

## Frameworks measured

| Framework | Binary source | Required |
|---|---|---|
| **Hopper** | `target/deploy/hopper_parity_vault.so` (this repo) | Yes. baseline |
| **Quasar** | `$quasar_root/target/deploy/quasar_vault.so` | Optional, skipped if missing |
| **Pinocchio-style** | `$quasar_root/target/deploy/pinocchio_vault.so` | Optional, skipped if missing |
| **Anchor** | `$anchor_root/target/deploy/anchor_vault.so` | Optional, skipped if missing |

A framework without a built `.so` is silently skipped in the emitted
report so partial runs are valid during development. CI requires all
four to be present for a release cut.

## Shared vault contract

Every framework's vault implements the same four instructions with
identical storage semantics. This is the hard rule: behavioral
equivalence is what makes the compute-unit delta meaningful.

| Instruction | Behaviour | Accounts |
|---|---|---|
| `authorize` | Gate: require signer authority matches the vault's recorded authority | `[vault (ro), authority (signer)]` |
| `counter_access` | Read: increment a stored `u64` counter, return the new value | `[vault (mut), authority (signer)]` |
| `deposit` | Financial: move `lamports` from payer to vault, increment balance counter | `[vault (mut), payer (signer, mut), system_program]` |
| `withdraw` | Financial: move `lamports` from vault to user, require signer match | `[vault (mut), authority (signer), user (mut)]` |

The vault state layout is:

```text
// 8 bytes counter + 32 bytes authority = 40 bytes body
#[repr(C)]
struct Vault {
    counter: u64,         // LE
    authority: [u8; 32],
}
```

Under the 16-byte Hopper header the total account size is 56 bytes
(8 + 32 + 16 header = 56). Competitor frameworks are free to use
their own header scheme but the *payload* semantics must match.

## Seven measured workloads (audit page 15)

Every workload is executed once per sample and the mean is reported.
Default `samples = 128`.

| Workload | What it exercises |
|---|---|
| Counter increment with one small state account | Baseline framework overhead |
| Two non-overlapping segment writes in one account | Hopper's segment-lease innovation |
| Sequential read-then-write of same segment | Exposes sticky-vs-RAII borrow design |
| PDA create + initialise + write | DX + lifecycle overhead |
| 64 KiB account scan (zero-copy read) | Fast-path advantage at scale |
| 1 MiB account scan (zero-copy read) | Extreme-size behaviour |
| Cross-program foreign-field read | Lens / ABI verification cost |

The first four map directly to the `authorize` / `counter_access` /
`deposit` / `withdraw` instructions. Workloads 5–7 live in the
`bench/hopper-bench` on-chain program (they don't need cross-framework
equivalents because every framework reduces to raw byte reads for
them).

## What is measured

Per framework, per instruction:

- **Compute units**. read from `sol_log_compute_units` deltas.
- **Binary size**. from the `.so` file on disk, both in bytes and KiB.
- **Stack frame size**. extracted from `llvm-objdump --section=.text`.
- **Unsigned-withdraw rejection**. a safety correctness check: the
  `withdraw` instruction *must* reject when the signer constraint is
  violated. Any framework that accepts it fails this row.

Results are emitted as a JSON report at `bench/results/cross_framework.json`
plus a CSV at `bench/results/cross_framework.csv` for spreadsheet use.
The Hopper row is the baseline (delta = 0); every other row reports
`cu_delta = framework_cu - hopper_cu` so the direction of the
comparison is unambiguous.

## Pinning

The benchmark output is reproducible iff every node in the chain is
pinned:

| Component | Where pinned |
|---|---|
| rustc version | `rust-toolchain.toml` (repo root) |
| SBF toolchain | `bench/Dockerfile` or contributor's `solana --version` output |
| cargo profile | `bench/framework-vault-bench/Cargo.toml` `[profile.release]` |
| mollusk-svm | `bench/framework-vault-bench/Cargo.toml` version constraint |
| competitor commits | `bench/competitors.lock` (new; records git SHAs) |

Contributors should snapshot their `solana --version` and the
upstream competitor commits into `bench/competitors.lock` before
publishing any cross-framework numbers.

## Running the cross-framework bench

```bash
# Build all four vaults.
cargo build-sbf -p hopper-parity-vault
(cd $QUASAR_ROOT && cargo build-sbf -p quasar-vault)
(cd $QUASAR_ROOT && cargo build-sbf -p pinocchio-vault)
(cd $ANCHOR_ROOT && anchor build -p anchor-vault)

# Run the shared harness.
cargo run -p framework-vault-bench --release -- \
    --quasar-root $QUASAR_ROOT \
    --anchor-root $ANCHOR_ROOT \
    --out bench/results/

# Inspect.
jq '.benchmarks[] | {framework, authorize_cu, deposit_cu, binary_size_kib}' \
    bench/results/cross_framework.json
```

The docker wrapper at `bench/docker/run-cross-framework.sh` encapsulates
all of the above for CI reproducibility.

## Adding a new framework

1. Build the framework's vault `.so` with the shared contract above.
2. Add an entry to `ProgramSpec` in `bench/framework-vault-bench/src/main.rs`
   with the framework name, program ID, and binary path.
3. Run the harness. the new framework auto-appears in the report
   with `cu_delta` computed against the Hopper baseline.
4. Document the new competitor's commit SHA in `bench/competitors.lock`.

## Safety-correctness gate

Any framework that fails `unsigned_withdraw_rejected` is recorded in
the report but **excluded** from CU-delta totals. A framework that's
faster because it skipped a safety check is not a meaningful
comparison. the report flags this explicitly so readers know.

## Interpreting CU deltas

Small deltas (under 50 CU) are within run-to-run noise on mollusk.
Differences under 100 CU are uninteresting for most protocol decisions.
Differences at the 500+ CU level reflect genuine framework-level
overhead. The audit's explicit expectation is that Hopper lands
within ~10% of Pinocchio-style raw-substrate implementations while
offering the safety and DX Anchor provides. the cross-framework
report is how that claim is validated.
