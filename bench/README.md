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

3. **Measurement entrypoints** (`measure.sh`, `measure.ps1`, and
   `hopper profile bench`) deploy the bench program to a local validator, run
   every implemented primitive benchmark, parse the bounded CU deltas from
   logs, and compare against baselines.

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
| `proc_macro_typed_dispatch` | 80 | generated `Context<...>` binding + `u64` decode + segment mutation |
| `create_account` CPI | 5000 | System program CreateAccount |
| `token_transfer` CPI | 4000 | SPL Token Transfer |

## Running Locally

### Option A: Docker Desktop (recommended for Windows, no manual validator setup)

```powershell
# Windows: starts the validator container, runs all 20 benchmarks, stops container
.\bench\run-bench-docker.ps1

# Pass extra flags directly to `hopper profile bench`
.\bench\run-bench-docker.ps1 --no-build --out-dir bench\results
```

```bash
# Linux / macOS / WSL
./bench/run-bench-docker.sh
./bench/run-bench-docker.sh --no-build --out-dir bench/results
```

The Docker scripts:
- Pull `anzaxyz/agave:v2.3.13` on first run (override with `SOLANA_IMAGE=...`)
- Generate a dedicated `bench/fixtures/bench-keypair.json` if it doesn't exist
- Wait up to 60 s for the validator to report healthy
- Forward any extra arguments to `hopper profile bench`
- Stop the container in a `finally`/`trap` block regardless of outcome

To switch Solana versions:
```powershell
$env:SOLANA_IMAGE = "anzaxyz/agave:v2.3.13"
.\bench\run-bench-docker.ps1
```

### Option B: Manual validator

```bash
# Start a local validator in a separate terminal
solana-test-validator --reset

# Run the primitive benchmark lab from the workspace root
hopper profile bench
```

The thin wrappers `bench/measure.sh` and `bench/measure.ps1` also delegate to
`hopper profile bench` and are suitable when the validator is already running.

## CI Integration

```yaml
# .github/workflows/bench.yml
- name: Start validator
  run: docker compose -f bench/docker/docker-compose.yml up -d

- name: CU Regression Gate
  run: ./bench/run-bench-docker.sh --fail-on-regression 5

- name: Stop validator
  if: always()
  run: docker compose -f bench/docker/docker-compose.yml down
```

The `--fail-on-regression 5` flag causes the runner to exit with status 1 if
any measurement exceeds its baseline by more than 5%.
