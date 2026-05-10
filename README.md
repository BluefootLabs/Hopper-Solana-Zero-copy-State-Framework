# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

> **Release status.** Hopper `0.1.0` is the first public release line for the
> Hopper framework, CLI, and companion crates. APIs are still young, and the
> release surface is documented, benchmark-provenanced, and gated by the checks
> in this repository. Verify crates.io package ownership before using registry
> install commands for the root `hopper` package.

Hopper is a zero-copy state framework for Solana programs. It maps typed,
fixed-layout views onto account bytes without a serialization round trip, while
keeping the byte layout inspectable through headers, layout fingerprints,
schema manifests, and CLI tooling.

The repository now follows the Quasar-style product layout: framework-internal
crates live together in this main repo, while independent products such as the
benchmark suite and SVM harness live separately.

## What Hopper provides

- `no_std` / `no_alloc` framework crates for on-chain programs.
- Zero-copy typed account access over fixed-layout account bytes.
- Layout fingerprints and versioned headers for account compatibility checks.
- Segment-aware access helpers for field-level borrow tracking.
- Optional proc macros for faster authoring; the core framework remains usable
  without proc macros.
- Hopper Native by default, targeting Pinocchio-class performance with
  framework safety/DX, with explicit legacy Pinocchio and `solana-program`
  compatibility modes quarantined behind opt-in features.
- Schema, IDL, manager, and CLI tooling for inspecting and explaining account
  layouts.

## Release Status

- Framework version target: `hopper = "0.1.0"`.
- Version-pinned docs.rs target: <https://docs.rs/crate/hopper/0.1.0>.
- Source-backed CLI install: `cargo install --path tools/hopper-cli` from this
  repository.
- Public companion crate targets include `hopper-native`, `hopper-runtime`,
  `hopper-core`, `hopper-schema`, `hopper-solana`, `hopper-token`,
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

```toml
[dependencies]
hopper = { git = "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework", features = ["proc-macros"] }
```

Install the CLI:

```sh
git clone https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework hopper
cd hopper
cargo install --path tools/hopper-cli
```

For local development inside this repository:

```toml
[dependencies]
hopper = { path = "../Hopper-Solana-Zero-copy-State-Framework", features = ["proc-macros"] }
```

Minimal layout example:

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    pub authority: TypedAddress<Authority>,
    pub balance: WireU64,
    pub bump: u8,
}

#[hopper::program]
mod vault {
    use super::*;

    #[instruction(1)]
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
        let mut balance = ctx.vault_balance_mut()?;
        *balance = WireU64::new(balance.get() + amount);
        Ok(())
    }
}
```

## Documentation map

- [docs/GETTING_STARTED_SERIOUS.md](docs/GETTING_STARTED_SERIOUS.md): source-first setup and first serious program flow.
- [docs/WRITING_HOPPER_PROGRAMS.md](docs/WRITING_HOPPER_PROGRAMS.md): Hopper authoring patterns and program structure.
- [docs/POLICY_GUARANTEES.md](docs/POLICY_GUARANTEES.md): capability policy, sealed/raw/hybrid access, and the policy-vault example.
- [docs/MIGRATION_FROM_ANCHOR.md](docs/MIGRATION_FROM_ANCHOR.md): Anchor-to-Hopper migration notes.
- [docs/MIGRATION_FROM_QUASAR.md](docs/MIGRATION_FROM_QUASAR.md): Quasar-to-Hopper migration notes.
- [docs/DYNAMIC_TAILS_FROM_QUASAR.md](docs/DYNAMIC_TAILS_FROM_QUASAR.md): mapping Quasar bounded dynamic fields to Hopper fixed-body + dynamic-tail layouts.
- [docs/QUASAR_PINOCCHIO_REPLACEMENT.md](docs/QUASAR_PINOCCHIO_REPLACEMENT.md): what Hopper replaces from Quasar/Pinocchio and what benchmark claims still require same-provenance proof.
- [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md): lifecycle, schema, client, profiling, and manager command reference.

## Access model

Use Hopper's access tiers deliberately:

1. `segment_ref_typed` / generated field accessors - default hot path for
  field-level borrow leasing.
2. `load` / `load_mut` - validated whole-layout access.
3. `segment_ref_const` / dynamic `segment_ref` - advanced runtime-selected
  segment access.
4. `raw_ref` / `raw_mut` - unsafe typed escape hatch.
5. `as_mut_ptr` - full raw pointer escape for policy-controlled raw mode.

For variable-length account data, use `#[hopper::state(dynamic_tail = T)]` for
small bounded payloads attached to one fixed layout, and named extension
segments for larger/repeated regions that need independent borrow tracking or
migration metadata.

## Repository layout

| Path | Purpose |
|---|---|
| `crates/hopper-runtime` | Runtime account views, borrow tracking, CPI helpers, backend compatibility. |
| `crates/hopper-core` | ABI types, account headers, layout contracts, checks, collections, receipts. |
| `crates/hopper-macros` | Declarative macro surface. |
| `crates/hopper-macros-proc` | Optional proc-macro authoring layer. |
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
hopper = { git = "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework" }

# Legacy Pinocchio migration/benchmark compatibility only
hopper = { git = "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework", default-features = false, features = ["legacy-pinocchio-compat"] }

# solana-program compatibility backend
hopper = { git = "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework", default-features = false, features = ["solana-program-backend"] }
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
cargo test -p hopper --features proc-macros,metaplex --test constant_integration -- --nocapture
cargo test -p hopper --features proc-macros,metaplex --test metaplex_context_integration -- --nocapture
```

The CLI source lives in `tools/hopper-cli`. It supports lifecycle commands,
linting, schema/IDL export, manifest inspection, account decoding, client
generation, manager workflows, and profile helpers.

Start with `examples/hopper-policy-vault` to see strict, sealed, raw, and
hybrid handlers side by side. For in-process tests, use the sibling
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

Current positioning: Hopper targets Pinocchio-class performance and access
shape while adding framework safety, schema, lifecycle, CPI, and CLI tooling.
That is an architecture/DX claim until a same-provenance Pinocchio column is
published.

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

## License

Licensed under either of:

- MIT license (`LICENSE-MIT`)
- Apache License, Version 2.0 (`LICENSE-APACHE`)

at your option.
