# Hopper Replacement Surface: Pinocchio + Quasar

> **Benchmark posture.** Hopper's release-facing benchmark claims compare
> Hopper and Quasar only. Older "Pinocchio-style" numbers came from a
> Quasar-authored reference vault and are intentionally excluded from launch
> claims. The sibling `hopper-bench` product repo owns the Anza Pinocchio
> target and will publish Pinocchio numbers only when they share the same
> lockfile, SBF toolchain, Mollusk version, seed set, and command line as the
> Hopper and Quasar columns.

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

It does not ship a larger scenario benchmark like a vault or escrow. Hopper's
Pinocchio comparison target therefore lives in the sibling benchmark repo as a
small Anza Pinocchio program with explicit provenance, not as a borrowed
Quasar example.

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
| Quasar `profile/` | Sibling `hopper-bench` repo plus `hopper profile bench` for primitive Hopper-local measurements |

The key design constraint is public-facing: Hopper should not expose separate
"Pinocchio mode" and "Quasar mode" products. Hopper exposes one access model,
one runtime story, and optional escape hatches where lower-level control is
needed.

## Cross-Framework Benchmark Path

Cross-framework measurement is a standalone product surface in the sibling
`hopper-bench` repo. That checkout builds and compares:

- Hopper parity vault from this framework workspace.
- Quasar's vault example from a pinned Quasar source checkout.
- Anza Pinocchio target when the benchmark lockfile includes it.

The output includes deposit CU, withdraw CU, counter CU, delta versus Hopper,
compiled binary size, and unsigned-withdraw safety parity. The shared runner
averages deterministic user seed cases across every included framework so the
comparison does not hinge on one lucky or unlucky PDA bump.

The runner loads compiled SBF binaries into one shared `mollusk-svm` harness
and executes the same scenarios for each:

- authorize: signer + writable + PDA validation only on the same
  `['vault', user]` PDA shape.
- deposit: user signer to `['vault', user]` PDA via system-program transfer
  CPI.
- withdraw: direct lamport mutation from a program-owned `['vault', user]`
  PDA.

That keeps the benchmark apples-to-apples instead of mixing framework overhead
with extra example features like Hopper's init path and zero-copy vault state.

Current release-facing results are the Hopper/Quasar table in
[`../BENCHMARKS.md`](../BENCHMARKS.md). Pinocchio results stay out of this doc
until the sibling benchmark repo publishes a same-provenance Anza Pinocchio
run.

## Hopper Safety And Feature Coverage

### Safety examples

- The sibling benchmark runner verifies unsigned withdraw rejection for every
  framework included in a run.
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
