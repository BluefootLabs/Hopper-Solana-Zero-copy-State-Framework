# Hopper CU Benchmark Lab

Compute-unit measurements and regression baselines for Hopper framework primitives.

## Architecture

```text
bench/
├── README.md                 ← This file
├── cu_baselines.toml         ← Golden CU baselines (CI gate thresholds)
├── compare-framework-vaults.ps1 ← Build wrapper for the fair vault comparison
├── framework-vault-bench/    ← Shared Mollusk runner for Hopper/Pinocchio/Quasar vaults
├── pinocchio-vault/          ← In-tree Anza Pinocchio raw-substrate baseline (R2)
│   ├── Cargo.toml
│   └── src/main.rs           ← Shared scenario runner and report writer
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
| --- | ---: | --- |
| `check_signer` | 20 | Fast-path header compare |
| `check_writable` | 20 | Fast-path header compare |
| `check_owner` | 50 | 32-byte key compare |
| `check_account` (Tier 1) | 120 | owner + disc + version + layout_id + size |
| `check_keys_eq` | 40 | 4×u64 short-circuit compare |
| `verify_pda` (with bump) | 200 | create_program_address syscall |
| `verify_pda_cached` | 200 | BUMP_OFFSET optimization path |
| `find_and_verify_pda` | 544 | Hopper Native fast PDA path (`sol_sha256` + `sol_curve_validate_point`) |
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

### Option C: Cross-framework vault comparison

Builds the minimal in-tree vault programs (Hopper + Pinocchio) and runs them
through one shared Mollusk harness. Quasar and Anchor are optional and only
included when you pass a local checkout root.

- Hopper `examples/hopper-parity-vault` (always built in-tree)
- Pinocchio `bench/pinocchio-vault` (always built in-tree; Anza `pinocchio = "0.10"`)
- Quasar `examples/vault` (optional; requires `-QuasarRoot`)
- Anchor `programs/anchor-vault` (optional; requires `-AnchorRoot`)

```powershell
# Minimal: Hopper vs Pinocchio only.
.\bench\compare-framework-vaults.ps1

# Include Quasar:
.\bench\compare-framework-vaults.ps1 -QuasarRoot d:\tmp\framework-sources\quasar-master\quasar-master
```

The `-QuasarRoot` argument is optional and points to an extracted Quasar
repository checkout (for example, a local mirror of the upstream Quasar
repo). Hopper deliberately does **not** vendor Quasar sources. Pre-R2 this
argument was mandatory because the Pinocchio baseline was loaded from
Quasar's `examples/pinocchio-vault`. That third-party indirection is gone;
the Pinocchio baseline is now built in-tree (see R2 in [AUDIT.md](../AUDIT.md)).

This flow:

- builds `hopper-parity-vault`
- builds the in-tree `pinocchio-vault`
- builds Quasar `examples/vault` (only if `-QuasarRoot` was supplied)
- runs every built binary under one shared `mollusk-svm` runner
- averages 8 shared deterministic user seed cases across every framework present
- uses one authorize scenario: signer + writable + PDA validation on the same `['vault', user]` PDA shape with no CPI or lamport mutation
- uses one counter-access scenario: the same `['vault', user]` PDA plus a raw `[authority:32][counter:8]` data region that is validated and incremented without CPI or lamport mutation
- uses one deposit scenario: system CPI transfer into the same `['vault', user]` PDA shape
- uses one withdraw scenario: direct lamport mutation from the same program-owned PDA shape
- checks unsigned withdraw rejection for every binary
- measures the missing-signature failure cost on the authorize path for every binary
- writes JSON and CSV metrics under `bench/results/framework-vaults`

The comparison is scenario-level rather than primitive-level, so it complements
`hopper profile bench` instead of replacing it. The dedicated parity target is
intentional: `examples/hopper-vault` remains the richer Hopper feature demo,
while `examples/hopper-parity-vault` is the fair benchmark target.

Latest verified averaged result (pre-R2 — Quasar-authored Pinocchio reference):

- Hopper parity: authorize `823` CU, auth-fail `122` CU, counter `993` CU, deposit `2050` CU, withdraw `851` CU, binary `8.30` KiB
- Quasar: authorize `585` CU, auth-fail `66` CU, counter `607` CU, deposit `1768` CU, withdraw `605` CU, binary `8.36` KiB
- Pinocchio-style (deprecated column): authorize `2543` CU, auth-fail `74` CU, counter `2575` CU, deposit `3763` CU, withdraw `2567` CU, binary `10.13` KiB

The "Pinocchio-style" row above is historical. After R2 the Pinocchio column
is built in-tree from `bench/pinocchio-vault` using Anza's own crates, and
numbers will be refreshed on the next bench run. Expect Hopper's lead over
idiomatic Pinocchio to be a few hundred CU on PDA-bearing instructions
(the verify-only sha256 PDA path), not the ~2000 CU shown above — the old
gap was against a non-optimised reference sample, not against Pinocchio in
the shape Pinocchio users actually ship.

The main Hopper win in this pass is framework-owned: Hopper Native now uses a
direct native PDA verification/search path and Hopper Runtime routes those
checks without paying avoidable runtime-address conversion overhead. That cuts
the parity vault materially versus the previous baseline and lands the current
authorize gap at `238` CU (`823` vs `585` against Quasar) with the missing-signature gap at
`56` CU (`122` vs `66` against Quasar).

The counter-access scenario is the honest safety-model benchmark: every
framework mutates the same raw `[authority:32][counter:8]` state region on
the same vault PDA shape, but Hopper does it through `segment_ref` +
`segment_mut` while Quasar and the Pinocchio baseline slice raw bytes
directly. That puts the current Hopper segment-safe path `386` CU behind
Quasar (`993` vs `607`), which is the clearest remaining performance target
on Hopper's unique state model rather than on CPI-heavy flows.

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
