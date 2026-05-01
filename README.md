# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

> **Beta / pre-release.** Hopper is under active development and has not
> received an external security audit. APIs may change. Use at your own risk.
> Hopper is not published to crates.io or docs.rs yet; use a git or path
> dependency until the first public release is cut.

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
- Hopper Native by default, with explicit legacy Pinocchio and
  `solana-program` compatibility modes quarantined behind opt-in features.
- Schema, IDL, manager, and CLI tooling for inspecting and explaining account
  layouts.

## Status

Hopper is not release-stable yet.

- Use from source via a git or path dependency.
- Do not rely on public API stability until the first tagged release.
- Benchmark numbers should be regenerated from the separate
  [hopper-bench](https://github.com/BluefootLabs/hopper-bench) repo before any
  launch or comparison claim.
- Security-sensitive users should treat Hopper as unaudited until an external
  audit is complete.

## Quick start from source

```toml
[dependencies]
hopper = { git = "https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework", features = ["proc-macros"] }
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
- [docs/QUASAR_PINOCCHIO_REPLACEMENT.md](docs/QUASAR_PINOCCHIO_REPLACEMENT.md): why Hopper Native replaces a backend-first Pinocchio story.
- [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md): lifecycle, schema, client, profiling, and manager command reference.

## Access model

Use Hopper's access tiers deliberately:

1. `segment_ref_typed` / generated field accessors — default hot path for
  field-level borrow leasing.
2. `load` / `load_mut` — validated whole-layout access.
3. `segment_ref_const` / dynamic `segment_ref` — advanced runtime-selected
  segment access.
4. `raw_ref` / `raw_mut` — unsafe typed escape hatch.
5. `as_mut_ptr` — full raw pointer escape for policy-controlled raw mode.

For variable-length account data, use inline dynamic fields for small bounded
payloads and named extension segments for larger/repeated regions that need
independent borrow tracking or migration metadata.

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
# Default backend
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

Do not claim Pinocchio performance parity or wins until the R2 Pinocchio column
has been regenerated from the standalone benchmark repo.

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
