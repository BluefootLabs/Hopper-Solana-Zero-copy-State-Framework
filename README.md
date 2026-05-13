# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

> **Release status.** Hopper `0.1.0` is the first public release line for the
> Hopper framework, CLI, and companion crates. APIs are still young, and the
> release surface is documented, release-checked, and scoped to the APIs
> exercised by this repository.

Hopper is a fast zero-copy Solana framework. Start with familiar account and
context ergonomics, then opt into upgradeable state contracts, segment-level
borrows, receipts, policy graphs, and schema manifests when the program needs
that power.

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
- Layout fingerprints and versioned headers for account compatibility checks.
- Segment-aware access helpers for field-level borrow tracking behind
  `hopper::systems::*`.
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

- Main framework package: `hopper-lang = "0.1.0"`; import it as
  `hopper` with `hopper = { package = "hopper-lang", version = "0.1.0" }`.
- Version-pinned docs.rs target: <https://docs.rs/crate/hopper-lang/0.1.0>.
- CLI install: `cargo install hopper-cli`.
- Public companion crate targets include `hopper-native`, `hopper-runtime`,
  `hopper-systems`, `hopper-derive`, `hopper-schema`, `hopper-solana`, `hopper-token`,
  `hopper-token-2022`, `hopper-associated-token`, `hopper-system`,
  `hopper-memo`, `hopper-finance`, `hopper-lending`, `hopper-staking`,
  `hopper-vesting`, `hopper-distribute`, `hopper-multisig`, `hopper-anchor`,
  `hopper-manager`, and `hopper-sdk`, all at `0.1.0`.
- Benchmark numbers must be regenerated from the separate
  [hopper-bench](https://github.com/BluefootLabs/hopper-bench) repo before any
  launch or comparison claim.
- Security-sensitive users should review [AUDIT.md](AUDIT.md) and
  [docs/UNSAFE_INVARIANTS.md](docs/UNSAFE_INVARIANTS.md) before deployment.

## Quick Start

Add the published framework package under the Rust crate name `hopper`:

```sh
cargo add hopper-lang --rename hopper --features proc-macros
```

Equivalent `Cargo.toml` entry:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.1.0", features = ["proc-macros"] }
```

For SBF programs that want the same explicit feature shape used by `hopper init`:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.1.0", default-features = false, features = ["hopper-native-backend", "proc-macros"] }
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
- Framework docs: <https://docs.rs/crate/hopper-lang/0.1.0>
- CLI crate: <https://crates.io/crates/hopper-cli>
- CLI docs: <https://docs.rs/crate/hopper-cli/0.1.0>
- Website and docs entry point: <https://hopperzero.dev>

Minimal framework example:

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(disc = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[accounts]
pub struct Increment {
    #[account(mut)]
    pub counter: Counter,

    #[signer]
    pub authority: AccountView,
}

#[program]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Context<Increment>) -> ProgramResult {
        let authority = *ctx.authority_account()?.address();
        let mut counter = ctx.counter_load_mut()?;

        require_keys_eq!(counter.authority, authority, ProgramError::IncorrectAuthority);

        let next = counter
            .value
            .get()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        counter.value = WireU64::new(next);
        Ok(())
    }
}
```

## Documentation map

- [docs/GETTING_STARTED_SERIOUS.md](docs/GETTING_STARTED_SERIOUS.md): source-first setup and first serious program flow.
- [docs/HOPPER_LAYERS.md](docs/HOPPER_LAYERS.md): framework mode, structured state, systems mode, and Anchor/Quasar/Hopper mental mapping.
- [docs/WRITING_HOPPER_PROGRAMS.md](docs/WRITING_HOPPER_PROGRAMS.md): Hopper authoring patterns and program structure.
- [docs/POLICY_GUARANTEES.md](docs/POLICY_GUARANTEES.md): capability policy, sealed/raw/hybrid access, and the policy-vault example.
- [docs/MIGRATION_FROM_ANCHOR.md](docs/MIGRATION_FROM_ANCHOR.md): Anchor-to-Hopper migration notes.
- [docs/MIGRATION_FROM_QUASAR.md](docs/MIGRATION_FROM_QUASAR.md): Quasar-to-Hopper migration notes.
- [docs/PORT_QUASAR_IN_20_MINUTES.md](docs/PORT_QUASAR_IN_20_MINUTES.md): hands-on bounded-tail vault/multisig port guide using `#[hopper::dynamic_account]`.
- [docs/DYNAMIC_TAILS_FROM_QUASAR.md](docs/DYNAMIC_TAILS_FROM_QUASAR.md): mapping Quasar bounded dynamic fields to Hopper fixed-body + compact dynamic-tail layouts.
- [docs/QUASAR_PINOCCHIO_REPLACEMENT.md](docs/QUASAR_PINOCCHIO_REPLACEMENT.md): what Hopper replaces from Quasar/Pinocchio and what benchmark claims still require same-provenance proof.
- [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md): lifecycle, schema, client, profiling, and manager command reference.
- [docs/PUBLICATION_AUDIT.md](docs/PUBLICATION_AUDIT.md): crate-by-crate publication and competitive-readiness audit.

## Progressive Use Model

Hopper is layered so users do not have to learn the systems surface first:

1. Framework mode: `use hopper::prelude::*`, `#[account]`, `#[program]`,
   typed wrappers, PDA helpers, token modules, and guard macros.
2. Structured state: add `hopper::layout`, `hopper::schema`,
   `#[hopper::dynamic_account]`, explicit dynamic tails, and generated
   manifests when account compatibility matters.
3. Systems mode: add `hopper::systems::*`, `hopper::segment`,
   `hopper::receipt`, `hopper::policy`, `hopper::migration`, and
   `hopper::interface` for field leasing, audit trails, upgrades, and
   cross-program layout contracts.

## Advanced Access Tiers

Framework handlers normally use generated context accessors or `load` /
`load_mut`. Reach for these lower-level access tiers when a protocol explicitly
needs systems-mode control:

1. `segment_ref_typed` / generated field accessors - default hot path for
  field-level borrow leasing.
2. `load` / `load_mut` - validated whole-layout access.
3. `segment_ref_const` / dynamic `segment_ref` - advanced runtime-selected
  segment access.
4. `raw_ref` / `raw_mut` - unsafe typed escape hatch.
5. `as_mut_ptr` - full raw pointer escape for policy-controlled raw mode.

For variable-length account data, opt into structured state with
`#[hopper::dynamic_account]` for Quasar-style bounded `String` and
`Vec<Address>` fields, or use `#[hopper::state(dynamic_tail = T)]` plus
`hopper_dynamic_fields!` for explicit custom tails. Named extension segments
remain the right tool for larger repeated regions that need independent borrow
tracking or migration metadata.

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
hopper = { package = "hopper-lang", version = "0.1.0" }

# Legacy Pinocchio migration/benchmark compatibility only
hopper = { package = "hopper-lang", version = "0.1.0", default-features = false, features = ["legacy-pinocchio-compat"] }

# solana-program compatibility backend
hopper = { package = "hopper-lang", version = "0.1.0", default-features = false, features = ["solana-program-backend"] }
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
linting, schema/IDL export, manifest inspection, account decoding, client
generation, manager workflows, and profile helpers.

Start with `examples/hopper-counter`, then move to `examples/hopper-vault` or
`examples/hopper-proc-vault` for the app framework path. Move to
`examples/hopper-policy-vault` for strict, sealed, raw, and hybrid handlers side
by side. For in-process tests, use the sibling
[hopper-svm](https://github.com/BluefootLabs/hopper-svm) repo as a
dev-dependency.

## Benchmarks

The benchmark suite is maintained as a separate product repo:

https://github.com/BluefootLabs/hopper-bench

Do not copy old benchmark numbers from this README. Regenerate numbers from the
benchmark repo before publishing performance claims.

Release-facing performance claims are Hopper-vs-Quasar only until the Anza
Pinocchio target is measured from the same `hopper-bench` lockfile, SBF
toolchain, Mollusk version, seed set, feature flags, release profile, and
command line as the Hopper and Quasar columns.

Current positioning: Hopper targets low-overhead account access while adding
framework safety, schema, lifecycle, CPI, and CLI tooling. Direct Pinocchio
comparison claims wait for a same-provenance Pinocchio benchmark column.

Canonical reproduction command:

```sh
cd ../hopper-bench
./measure.sh all
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
