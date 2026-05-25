# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

> **Release status.** Hopper `0.2.1` is the current public release line for the
> Hopper framework, CLI, and companion crates. APIs are still young, and the
> release surface is documented, release-checked, and scoped to the APIs
> exercised by this repository.

Hopper is a fast zero-copy Solana framework: write programs like Anchor or
Quasar, while Hopper verifies the bytes before it casts them. Start with
familiar account and context ergonomics, then opt into upgradeable state
contracts, segment-level borrows, receipts, policy graphs, and schema manifests
when the program needs that power.

The public claim is precise: **Anchor/Quasar-class DX, Hopper-grade
safety/state contracts, Pinocchio-class raw control.**

The framework path and the systems path share the same runtime. `hopper-lang`,
imported as `hopper`, is the front door: `use hopper::prelude::*`,
`#[hopper::account]`, typed account wrappers, and the
`hopper::{account,cpi,token,system}` facade modules. `hopper-systems` is the
advanced state architecture underneath it.

The repository keeps framework crates, first-party examples, and release
tooling together. Independent products such as the benchmark suite and SVM
harness live separately so release claims stay reproducible and easy to audit.

## What Hopper provides

- `no_std` / `no_alloc` framework crates for on-chain programs.
- A focused framework facade: `#[hopper::account]`, `#[hopper::program]`,
  `#[derive(Accounts)]`, `Account<'info, T>`, `Signer<'info>`, `Program<'info, P>`.
- Zero-copy typed account access over fixed-layout account bytes.
- Closure-scoped `with` / `with_mut` account access for direct state mutation
  while Hopper owns validation and borrow lifetimes.
- Layout fingerprints and versioned headers for account compatibility checks.
- Segment-aware access helpers for field-level borrow tracking behind
  `hopper::systems::*`.
- Token-2022 extension constraints that scan TLV data without leaving the
  zero-copy path.
- Optional proc macros for faster authoring; the core framework remains usable
  without proc macros.
- Progressive modules: `hopper::account`, `hopper::cpi`, and `hopper::token`
  for app code; `hopper::systems::*` plus `hopper::{layout,segment,receipt}`
  for explicit lower-layer access.
- Hopper Native by default for low-overhead account access with framework
  safety/DX, with explicit legacy Pinocchio and `solana-program` compatibility
  modes quarantined behind opt-in features.
- Schema, IDL, manager, and CLI tooling for inspecting and explaining account
  layouts.

## Release Status

- Main framework package: `hopper-lang = "0.2.1"`; import it as
  `hopper` with `hopper = { package = "hopper-lang", version = "0.2.1" }`.
- Version-pinned docs.rs target: <https://docs.rs/crate/hopper-lang/0.2.1>.
- CLI install: `cargo install hopper-cli`.
- Public companion crate targets include `hopper-native`, `hopper-runtime`,
  `hopper-systems`, `hopper-derive`, `hopper-schema`, `hopper-solana`, `hopper-token`,
  `hopper-token-2022`, `hopper-associated-token`, `hopper-system`,
  `hopper-memo`, `hopper-finance`, `hopper-lending`, `hopper-staking`,
  `hopper-vesting`, `hopper-distribute`, `hopper-multisig`, `hopper-anchor`,
  `hopper-manager`, and `hopper-sdk`, all at `0.2.1`.
- The current same-provenance vault benchmark snapshot is documented in
  [BENCHMARKS.md](BENCHMARKS.md); regenerate it from the separate
  [hopper-bench](https://github.com/BluefootLabs/hopper-bench) repo before
  changing launch or benchmark claims.
- Generated client targets in this release are TypeScript, Kotlin, Python, Go,
  C header-only, off-chain Rust, Codama-shaped JSON, and Anchor-shaped IDL JSON.
  Generated readers assert Hopper layout IDs before decoding account bytes.
- Security-sensitive users should review [AUDIT.md](AUDIT.md) and
  [docs/UNSAFE_INVARIANTS.md](docs/UNSAFE_INVARIANTS.md) before deployment.

## Hopper in 30 seconds

Write the account shape, derive the account context, and keep mutation behind
`ctx.accounts.*`:

```rust
#[program(profile = "tiny")]
mod counter_program {
  use super::*;

  #[instruction(0)]
  pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
    ctx.accounts
      .counter
      .with_mut(|counter| counter.value.checked_add_assign(1))
  }
}
```

That is the core Hopper pitch: Quasar-like handler shape, but the account has
already passed owner, role, discriminator, version, and layout-fingerprint
checks before the typed borrow reaches the closure.

## Quick Start

Add the published framework package under the Rust crate name `hopper`:

```sh
cargo add hopper-lang --rename hopper --features proc-macros
```

Equivalent `Cargo.toml` entry:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.2.1", features = ["proc-macros"] }
```

For SBF programs that want the same explicit feature shape used by `hopper init`:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.2.1", default-features = false, features = ["hopper-native-backend", "proc-macros"] }
```

Install the CLI:

```sh
cargo install hopper-cli
hopper init my-program --template minimal --yes
```

For local development inside this repository:

```toml
[dependencies]
hopper = { path = "../Hopper-Solana-Zero-copy-State-Framework", package = "hopper-lang", features = ["proc-macros"] }
```

Public package links:

- Framework crate: <https://crates.io/crates/hopper-lang>
- Framework docs: <https://docs.rs/crate/hopper-lang/0.2.1>
- CLI crate: <https://crates.io/crates/hopper-cli>
- CLI docs: <https://docs.rs/crate/hopper-cli/0.2.1>
- Website and docs entry point: <https://hopperzero.dev>

Minimal framework example:

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[derive(Accounts)]
pub struct Increment<'info> {
    #[account(mut, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

#[program]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        ctx.accounts
            .counter
            .with_mut(|counter| counter.value.checked_add_assign(1))
    }
}
```

Initialization uses the same surface. After `ctx.init_vault()?`, mutate the
fresh account through `ctx.accounts` and the generated `set_inner(...)` helper:

```rust
ctx.accounts
    .vault
    .with_mut_after_init(|vault| vault.set_inner(*ctx.accounts.payer.key(), 0, 0))?;
```

## Documentation map

- [docs/README.md](docs/README.md): current docs front door.
- [docs/FIRST_FIVE_MINUTES.md](docs/FIRST_FIVE_MINUTES.md): counter, vault, dynamic multisig, token transfer, and raw escape hatch through the `ctx.accounts.*` path.
- [docs/GETTING_STARTED_SERIOUS.md](docs/GETTING_STARTED_SERIOUS.md): source-first setup and first serious program flow.
- [docs/HOPPER_LAYERS.md](docs/HOPPER_LAYERS.md): framework mode, structured state, systems mode, and Anchor/Quasar/Hopper mental mapping.
- [docs/WRITING_HOPPER_PROGRAMS.md](docs/WRITING_HOPPER_PROGRAMS.md): Hopper authoring patterns and program structure.
- [docs/PROFILING.md](docs/PROFILING.md): `hopper profile elf`, binary profile artifacts, and reproducible benchmark commands.
- [docs/PROTOCOL_GRADE_EXAMPLES.md](docs/PROTOCOL_GRADE_EXAMPLES.md): receipt indexing, compatibility reports, migration plans, typed cross-program reads, and segment leases.
- [docs/POLICY_GUARANTEES.md](docs/POLICY_GUARANTEES.md): capability policy, sealed/raw/hybrid access, and the policy-vault example.
- [docs/MIGRATION_FROM_ANCHOR.md](docs/MIGRATION_FROM_ANCHOR.md): Anchor-to-Hopper migration notes.
- [docs/MIGRATION_FROM_QUASAR.md](docs/MIGRATION_FROM_QUASAR.md): Quasar-to-Hopper migration notes.
- [docs/HOPPER_VS_QUASAR.md](docs/HOPPER_VS_QUASAR.md): simple side-by-side positioning: Quasar casts; Hopper checks the cast.
- [docs/PORT_QUASAR_IN_20_MINUTES.md](docs/PORT_QUASAR_IN_20_MINUTES.md): hands-on bounded-tail vault/multisig port guide using pretty `#[hopper::account]` fields.
- [docs/DYNAMIC_TAILS_FROM_QUASAR.md](docs/DYNAMIC_TAILS_FROM_QUASAR.md): mapping Quasar bounded dynamic fields to Hopper fixed-body + compact dynamic-tail layouts.
- [docs/DYNAMIC_FIELDS_QUASAR_TO_HOPPER.md](docs/DYNAMIC_FIELDS_QUASAR_TO_HOPPER.md): side-by-side bounded dynamic field migration, including Hopper's compact-tail contract.
- [docs/TOKEN_2022_GUIDE.md](docs/TOKEN_2022_GUIDE.md): zero-copy Token-2022 extension policy examples and account constraint syntax.
- [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md): lifecycle, schema, client, profiling, Solana compatibility gates, generated Actions/mobile/test scaffolds, and manager command reference.

## Progressive Use Model

Hopper is layered so users do not have to learn the systems surface first:

1. Framework mode: `use hopper::prelude::*`, `#[account]`, `#[program]`,
   typed wrappers, PDA helpers, token modules, and guard macros.
2. Structured state: keep `#[account]` and add bounded dynamic fields such as
  `String<'a, 32>` or `Vec<'a, Address, 10>`. Hopper lowers them into the
  same fixed-body + compact-tail layout as explicit `#[hopper::dynamic_account]`.
  Use `TailStr<'a>` or `TailBytes<'a>` only when a protocol deliberately wants a
  named final field to consume the remaining tail payload.
3. Systems mode: add `hopper::systems::*`, `hopper::segment`,
   `hopper::receipt`, `hopper::policy`, `hopper::migration`, and
   `hopper::interface` for field leasing, audit trails, upgrades, and
   cross-program layout contracts.
4. Substrate mode: use `hopper::substrate` when a program needs direct Hopper
  Native tools such as account views, CU budget probes, hashes, PDA helpers,
  raw input parsing, memory helpers, and syscalls.

## Advanced Access Tiers

Framework handlers normally use `ctx.accounts.*` plus `get()` / `get_mut()` on
typed wrappers. Reach for these lower-level access tiers only when a protocol
explicitly needs systems-mode control:

1. `segment_ref_typed` / generated field accessors - default hot path for
  field-level borrow leasing.
2. `get` / `get_mut` on `Account<'info, T>` - validated whole-layout access.
3. `segment_ref_const` / dynamic `segment_ref` - advanced runtime-selected
  segment access.
4. `raw_ref` / `raw_mut` - unsafe typed escape hatch.
5. `as_mut_ptr` - full raw pointer escape for policy-controlled raw mode.

For variable-length account data, use Quasar-style bounded fields directly in
`#[account]`:

```ignore
#[hopper::account(discriminator = 10, version = 1)]
pub struct Multisig<'a> {
  pub threshold: u64,
  pub label: String<'a, 32>,
  pub signers: Vec<'a, Address, 10>,
}
```

The source stays pretty, but the wire truth stays explicit: fixed body, `u32`
tail length, compact tail payload. `Address` / `Pubkey` vectors keep the
borrowed zero-copy view path; other `T: TailElement` vectors use
`HopperVec<T, N>` through the same codec/editor path. Use
`#[hopper::dynamic_account]` with `#[tail(...)]` when you want the systems-mode
tail shape spelled out in source. For Quasar-style bare final tails, spell the
last field as `TailStr<'a>` or `TailBytes<'a>`; Hopper fingerprints it as
`tail_str` or `tail_bytes` and leaves the bytes unprefixed inside the tail
payload. Named extension segments remain the right tool for larger repeated
regions that need independent borrow tracking or migration metadata.

Handlers with variable account tails use generated remaining-account accessors:
`ctx.remaining_accounts()` is strict and duplicate-rejecting,
`ctx.remaining_accounts_passthrough()` preserves duplicates when a protocol
needs that, and `ctx.remaining_accounts().signers::<N>()?` validates bounded
multisig-style signer lists without allocation.

## Repository layout

| Path | Purpose |
|---|---|
| `.` (`hopper-lang`) | Main framework API imported as `hopper`: accounts, programs, CPI, PDA helpers, prelude. |
| `crates/hopper-runtime` | Internal runtime: account views, borrow tracking, CPI helpers, backend compatibility. |
| `crates/hopper-core` (`hopper-systems`) | Advanced state architecture: ABI types, headers, layout contracts, segments, policies, receipts. |
| `crates/hopper-macros` | Declarative macro surface. |
| `crates/hopper-macros-proc` (`hopper-derive`) | Proc-macro authoring layer. |
| `crates/hopper-native` | Native low-level backend used by Hopper by default. |
| `crates/hopper-schema` | Schema, IDL, Codama projection, and layout manifest support. |
| `crates/hopper-system` | Hopper-owned system-program helpers. |
| `crates/hopper-solana` | Solana interop helpers. |
| `crates/hopper-spl` | SPL Token, Token-2022, ATA, and Metaplex helper crates. |
| `crates/hopper-manager` | Manifest-driven account inspection library. |
| `crates/hopper-sdk` | Client-side SDK surface. |
| `tools/hopper-cli` | `hopper` CLI for linting, schema export, account inspection, and profiling. |
| `examples` | Example Hopper programs. |
| `examples/hopper-tail-lab` | Devnet-ready dynamic-tail lab covering bounded fields, `TailStr`, `TailBytes`, init, and account wrappers. |
| `docs` | Design notes, unsafe invariants, and audit/recovery notes. |

The obsolete split repositories were folded back into this workspace with
subtree history preserved and then archived/private on GitHub.

Sibling product repos:

- [hopper-bench](https://github.com/BluefootLabs/hopper-bench): benchmark harness and CU regression lab.
- [hopper-svm](https://github.com/BluefootLabs/hopper-svm): in-process Solana execution harness for Hopper test authors.

## Backend features

Hopper Native is the default backend.

```toml
# Default backend from source
hopper = { package = "hopper-lang", version = "0.2.1" }

# Legacy Pinocchio migration/benchmark compatibility only
hopper = { package = "hopper-lang", version = "0.2.1", default-features = false, features = ["legacy-pinocchio-compat"] }

# solana-program compatibility backend
hopper = { package = "hopper-lang", version = "0.2.1", default-features = false, features = ["solana-program-backend"] }
```

Only one backend should be enabled for a program build.

`legacy-pinocchio-compat` is not Hopper's native execution story. It exists for
migration tests and compatibility benchmarking. New programs should use the
default Hopper Native backend.

## Tooling

Useful development commands:

```sh
cargo metadata --no-deps --format-version 1
cargo test -p hopper-cli cmd::lint::tests -- --nocapture
cargo test -p hopper-lang --features proc-macros,metaplex --test constant_integration -- --nocapture
cargo test -p hopper-lang --features proc-macros,metaplex --test metaplex_context_integration -- --nocapture
```

The CLI source lives in `tools/hopper-cli`. It supports lifecycle commands,
linting, `solana-check`, schema/IDL export, manifest inspection, account
decoding, client generation, Solana Actions scaffolds, mobile bindings,
security test matrices, manager workflows, and profile helpers.

## Examples

Framework-first examples:

- [examples/hopper-counter](examples/hopper-counter): minimal `#[derive(Accounts)]`, `Ctx<T>`, and `ctx.accounts.*` flow.
- [examples/hopper-vault](examples/hopper-vault): SOL vault using typed wrappers, `set_inner`, checked wire helpers, and a System Program transfer helper for deposits.
- [examples/hopper-escrow](examples/hopper-escrow): token-escrow shape using the same account facade.
- [examples/quasar-port-20-min](examples/quasar-port-20-min): Quasar-style bounded dynamic account port with Hopper's dynamic-tail guarantees.
- [examples/hopper-devnet-audit](examples/hopper-devnet-audit): deployable devnet audit program covering dynamic tails, contexts, segments, proof chains, Token-2022 policy scans, field capabilities, and substrate probes.
- [examples/hopper-argus-guard](examples/hopper-argus-guard): Argus-style risk guard subsystem proof with checked exposure accounting and authority-bound state.

Systems-mode examples:

- [examples/hopper-proc-vault](examples/hopper-proc-vault): generated/lowered account access for teams that want to inspect the macro output shape.
- [examples/hopper-policy-vault](examples/hopper-policy-vault): strict, sealed, raw, and hybrid handlers side by side.
- [examples/hopper-showcase](examples/hopper-showcase): broad feature tour across framework and systems layers.

Raw and benchmark examples:

- [examples/hopper-parity-vault](examples/hopper-parity-vault): apples-to-apples benchmark target with intentionally low-level lamport mutation.
- [examples/hopper-token-2022-vault](examples/hopper-token-2022-vault) and [examples/hopper-token-2022-ata](examples/hopper-token-2022-ata): Token-2022 low-level validation and CPI examples.

For in-process tests, use the sibling [hopper-svm](https://github.com/BluefootLabs/hopper-svm) repo as a dev-dependency.

## Benchmarks

The benchmark suite is maintained as a separate product repo:

https://github.com/BluefootLabs/hopper-bench

Do not copy old benchmark numbers from this README. Regenerate numbers from the
benchmark repo before publishing performance claims.

The current same-provenance vault snapshot includes Hopper, the in-tree Anza
Pinocchio target, and Quasar's upstream vault target. Quasar implements only
the financial `deposit` / `withdraw` rows, so validation-only rows are marked
`n/a` rather than synthesized. See [BENCHMARKS.md](BENCHMARKS.md) for the table
and provenance.

Treat that table as a historical release-candidate vault measurement, not proof
that Hopper is broadly faster than Quasar. Re-run the benchmark repo at current
Hopper and Quasar heads before publishing fresh performance language.

Current positioning: **Anchor/Quasar-class DX, Hopper-grade safety/state
contracts, Pinocchio-class raw control.** Treat benchmark rows as measurements
of that vault contract, not a universal raw-substrate ranking.

Canonical reproduction command:

```powershell
cd ../hopper-bench
.\compare-framework-vaults.ps1 -HopperRoot ..\Hopper-Solana-Zero-copy-State-Framework -QuasarRoot <path-to-quasar> -OutDir results\framework-vaults
```

### Where Pinocchio Is Still The Right Choice

Use raw Pinocchio directly when a program wants the smallest possible substrate,
manual account validation, and no framework-level schema, lifecycle, or tooling
surface. Hopper is the framework-layer option for teams that want the same
low-level access model plus explicit safety and developer ergonomics.

## Safety posture

Hopper uses `unsafe` at the boundary where account bytes become typed views.
The framework keeps those boundaries small and documented, but this is still a
zero-copy framework and should be reviewed like one.

See:

- `docs/UNSAFE_INVARIANTS.md`
- `AUDIT.md`
- `crates/hopper-core/tests/unsafe_boundary_tests.rs`
- `crates/hopper-core/tests/overlay_equivalence_tests.rs`

## Support

Hopper is open-source Solana infrastructure. Public-goods support and donations
can be sent to `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

Donation URI: <solana:F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT?label=solanadevdao.sol>

## License

Licensed under either of:

- MIT license (`LICENSE-MIT`)
- Apache License, Version 2.0 (`LICENSE-APACHE`)

at your option.
