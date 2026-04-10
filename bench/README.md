# Hopper CU Benchmark Lab

Compute-unit measurements and regression baselines for Hopper framework primitives.

## Architecture

```
bench/
├── README.md                 ← This file
├── cu_baselines.toml         ← Golden CU baselines (CI gate thresholds)
├── hopper-bench/             ← On-chain benchmark program
│   ├── Cargo.toml
│   └── src/lib.rs            ← Per-primitive CU measurement entry points
└── measure.sh                ← CI script: deploy, measure, compare baselines
```

## How It Works

1. **On-chain program** (`hopper-bench/`) exposes instruction entry points that
   exercise one Hopper primitive per instruction. The program uses
   `sol_log_compute_units()` before and after each operation to capture CU deltas.

2. **Golden baselines** (`cu_baselines.toml`) define per-operation CU budgets.
   CI jobs fail if any measurement exceeds its budget by more than 5%.

3. **Measurement script** (`measure.sh`) deploys the bench program to a local
   validator, sends benchmark transactions, parses logs, and compares against
   baselines.

## Measured Primitives

| Operation | Baseline (CU) | Notes |
|---|---:|---|
| `check_signer` | 20 | Fast-path header compare |
| `check_writable` | 20 | Fast-path header compare |
| `check_owner` | 50 | 32-byte key compare |
| `check_account` (Tier 1) | 120 | owner + disc + version + layout_id + size |
| `check_keys_eq` | 40 | 4×u64 short-circuit compare |
| `verify_pda` (with bump) | 200 | create_program_address syscall |
| `verify_pda_cached` | 200 | BUMP_OFFSET optimization path |
| `find_and_verify_pda` | 1500 | find_program_address syscall |
| `check_account_fast` | 12 | Batched u32 header comparison |
| `overlay` (57-byte layout) | 8 | Pointer cast, size check |
| `write_header` | 30 | 16-byte header write |
| `zero_init` (57 bytes) | 15 | memset zero |
| `emit_event` (32 bytes) | 100 | sol_log_data syscall |
| `TrustProfile::load` (Strict) | 130 | owner + layout_id + size + sentinel |
| `create_account` CPI | 5000 | System program CreateAccount |
| `token_transfer` CPI | 4000 | SPL Token Transfer |

## Running Locally

```bash
# Start a local validator
solana-test-validator --reset &

# Build the bench program
cd bench/hopper-bench
cargo build-sbf

# Deploy and measure
cd ..
./measure.sh
```

## CI Integration

```yaml
# .github/workflows/bench.yml
- name: CU Regression Gate
  run: |
    cd bench
    ./measure.sh --ci --fail-on-regression=5
```

The `--fail-on-regression=5` flag causes the script to exit with status 1 if
any measurement exceeds its baseline by more than 5%.
