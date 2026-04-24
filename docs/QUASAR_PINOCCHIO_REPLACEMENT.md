# Hopper Replacement Surface: Pinocchio + Quasar

> **Historical note.** This document describes the pre-R2 state when the
> cross-framework bench loaded its Pinocchio baseline from Quasar's
> `examples/pinocchio-vault`. After audit recommendation R2 (see
> [`../AUDIT.md`](../AUDIT.md)) the Pinocchio baseline is built in-tree from
> [`../bench/pinocchio-vault`](../bench/pinocchio-vault/src/lib.rs) using
> Anza's own `pinocchio = "0.10"`. The rationale captured below is retained
> for historical context; specific CU numbers cited in this file refer to
> the deprecated Quasar-authored reference vault, not the current Anza
> Pinocchio baseline.

This note records what the extracted upstream sources actually contain and how
Hopper maps those surfaces into one unified system.

## What The Extracted Repos Contain

### Pinocchio (`pinocchio-main.zip`)

The upstream Pinocchio repo is intentionally narrow:

- `sdk/` for entrypoint, allocation, panic, CPI, and low-level account access
- `programs/system`
- `programs/token`
- `programs/token-2022`
- `programs/associated-token-account`

It does not ship a larger scenario benchmark like a vault or escrow. For a
Pinocchio-style scenario target we currently use Quasar's
`examples/pinocchio-vault`, which is built directly against Pinocchio.

### Quasar (`quasar-master.zip`)

The Quasar repo spans both language ergonomics and tooling:

- `lang/` for the main framework surface
- `derive/` for proc macros
- `spl/` for SPL CPI helpers and account wrappers
- `cli/` for `quasar init/build/test/deploy/profile/dump`
- `profile/` for tracked CU profiling
- `examples/` for `vault`, `escrow`, `multisig`, and `pinocchio-vault`
- `tests/programs/*` for safety and constraint regression suites

## Hopper Mapping

| Upstream surface | Hopper replacement |
| --- | --- |
| Pinocchio `sdk/` entrypoint / allocator / raw account access | `crates/hopper-native`, `crates/hopper-runtime`, root `hopper` macros |
| Pinocchio `programs/system` | `crates/hopper-system` |
| Pinocchio `programs/token` | `crates/hopper-token` |
| Pinocchio `programs/token-2022` | `crates/hopper-token-2022` |
| Pinocchio `programs/associated-token-account` | `crates/hopper-associated-token` |
| Quasar `lang/` + `derive/` | root `hopper`, `crates/hopper-macros`, `crates/hopper-macros-proc` |
| Quasar `spl/` | Hopper companion crates plus Hopper-owned CPI wrappers |
| Quasar `cli/` | `tools/hopper-cli` |
| Quasar `profile/` | `bench/`, `hopper profile bench`, and `bench/compare-framework-vaults.ps1` |

The key design constraint is public-facing: Hopper should not expose separate
"Pinocchio mode" and "Quasar mode" products. Hopper exposes one access model,
one runtime story, and optional escape hatches where lower-level control is
needed.

## Cross-Framework Benchmark Path

The repo now includes `bench/compare-framework-vaults.ps1` plus the shared host
runner in `bench/framework-vault-bench`.

It builds and compares:

- `hopper-parity-vault`
- Quasar `examples/vault`
- Quasar `examples/pinocchio-vault`

The output includes:

- deposit CU
- withdraw CU
- delta versus Hopper
- compiled binary size
- unsigned withdraw safety parity

The shared runner averages 8 deterministic user seed cases across all three
frameworks so the comparison does not hinge on one lucky or unlucky PDA bump.

The runner loads all three compiled SBF binaries into one shared `mollusk-svm`
harness and executes the same scenarios for each:

- authorize: signer + writable + PDA validation only on the same `['vault', user]` PDA shape
- deposit: user signer to `['vault', user]` PDA via system-program transfer CPI
- withdraw: direct lamport mutation from a program-owned `['vault', user]` PDA

That keeps the benchmark apples-to-apples instead of mixing framework overhead
with extra example features like Hopper's init path and zero-copy vault state.

Latest verified averaged result on the extracted archives:

- Hopper parity: authorize `823` CU, auth-fail `122` CU, counter `933` CU, deposit `2051` CU, withdraw `851` CU, binary `7.66` KiB
- Quasar: authorize `585` CU, auth-fail `66` CU, counter `607` CU, deposit `1768` CU, withdraw `605` CU, binary `8.36` KiB
- Pinocchio-style: authorize `2543` CU, auth-fail `74` CU, counter `2575` CU, deposit `3763` CU, withdraw `2567` CU, binary `10.13` KiB

That result means Hopper still clearly beats the Pinocchio-style vault while
staying much closer to Quasar, and the latest Hopper-side gain is still
framework-owned: Hopper Native now exposes a direct native PDA derivation and
verification path that Hopper Runtime can use without extra conversion churn.
That pulls the current authorize gap down to `238` CU (`823` vs `585`) and the
missing-signature gap to `56` CU (`122` vs `66`).

The new counter path is the more important strategic benchmark. All three
targets validate the same vault PDA and mutate the same raw
`[authority:32][counter:8]` state layout, but Hopper does the read/write via
`segment_ref` + `segment_mut` while Quasar and the Pinocchio-style target use
direct byte access. Hopper's current segment-safe path is `326` CU behind
Quasar (`933` vs `607`), which is the clearest performance target if Hopper
is going to claim a stronger state model without conceding too much runtime
cost.

Example:

```powershell
.\bench\compare-framework-vaults.ps1 -QuasarRoot d:\tmp\framework-sources\quasar-master\quasar-master
```

## Hopper Safety And Feature Coverage

### Safety examples

- `bench/framework-vault-bench` now verifies unsigned withdraw rejection for
  Hopper, Quasar, and the Pinocchio-style target under the same runner.
- `docs/SAFE_COMPOSITION.md` captures the broader safety model.
- `docs/UNSAFE_INVENTORY.md` tracks explicit escape hatches.

### Feature examples

- `examples/hopper-vault` for the minimal unified Hopper feature surface
- `examples/hopper-parity-vault` for fair cross-framework vault benchmarking
- `examples/hopper-escrow` for typed multi-instruction state flow
- `examples/hopper-showcase` for the broad language surface
- `examples/hopper-virtual-state` for virtualized state patterns
- `examples/hopper-migration` for layout/version migration
- `examples/hopper-token-2022-vault` for Hopper-owned Token-2022 + ATA flow
- `examples/cross-program-read` for inter-program state access

## Devnet Follow-Up

The next natural step is not a new architecture layer; it is an operational
workflow:

1. pick one scenario program (`hopper-vault` or `hopper-token-2022-vault`)
2. build it with `hopper build`
3. deploy it with `hopper deploy`
4. reuse the manifest/`hopper explain` flow for inspection and client output

That is the shortest path to proving Hopper can cover Pinocchio's low-level
deployment story and Quasar's developer-experience story with the same system.
