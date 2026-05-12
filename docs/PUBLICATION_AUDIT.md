# Hopper Publication and Competitive-Readiness Audit

This audit is a release-facing checklist for Hopper as a Solana zero-copy state framework. It is intentionally conservative: each statement is tied to a crate, document, test, or CLI gate in this repository. Competitive comparisons are scope comparisons, not live benchmark claims.

## Publication Verdict

Hopper `0.1.0` has been published to crates.io for the framework package,
CLI, and companion crates listed in [docs/RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).
The codebase has the core surfaces Solana program authors expect from a serious
framework:

- on-chain `no_std` / `no_alloc` runtime and backend crates,
- fixed-layout zero-copy state definitions,
- optional proc macros,
- account validation and borrow tracking,
- SPL Token, Token-2022, associated-token, memo, Metaplex, and system helpers,
- schema and client generation,
- receipts, policy, migration, and virtual-state modules,
- examples that cover common program shapes, and
- CLI release gates for manifest/binary verification and source checks.

## Package Name Decision

The crates.io package name `hopper` is occupied by an unrelated crate. Hopper
publishes the top-level framework package as `hopper-lang` while keeping
the Rust library crate name as `hopper`:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.1.0" }
```

That keeps user code idiomatic:

```rust
use hopper::prelude::*;
```

## Crate-by-Crate Audit

| Crate / package | Release role | Readiness notes |
|---|---|---|
| `hopper-lang` (`lib` crate `hopper`) | Main framework API and prelude | Consumers import it as `hopper` through `package = "hopper-lang"` |
| `hopper-native` | Low-level account, syscall, hash, CPI, and entrypoint backend | Backend scope is documented; higher-level framework concerns stay in higher crates |
| `hopper-runtime` | Account views, layout contracts, borrow tracking, token checks, CPI/event helpers | Unsafe boundaries and `ZeroCopy` sealing are covered by tests and docs |
| `hopper-systems` (`lib` crate `hopper_core`) | Advanced state architecture: ABI types, headers, Pod overlays, field/segment maps, migrations, receipts, collections | Includes compile-fail and unit coverage for Pod, overlay, account, receipt, and collection behavior |
| `hopper-macros` | Declarative macro path without proc macros | Keeps a macro authoring path available for constrained programs |
| `hopper-derive` (`lib` crate `hopper_macros_proc`) | Proc macros for accounts, contexts, args, events, errors, programs, and migrations | `#[hopper::state]` requires user-written `Clone + Copy` and has trybuild coverage |
| `hopper-schema` | Layout manifests, compatibility, IDL and client projections | Layout-ID client assertions are part of `publish-check` |
| `hopper-solana` | Solana helper APIs, crypto, compute, sysvar, token-screening glue | Token-2022 extension reads avoid forcing full account deserialization |
| `hopper-system` | System-program helpers | Kept separate so programs can opt into only needed surfaces |
| `hopper-token` | SPL Token helper crate | Plain legacy builders stay behind explicit feature gates |
| `hopper-token-2022` | Token-2022 helper crate | Complements runtime extension readers and guide docs |
| `hopper-associated-token` | Associated Token Account helpers | Published before `hopper-lang` because the top-level crate depends on it |
| `hopper-metaplex` | Metaplex metadata and NFT CPI helpers | Optional from the top-level crate behind `metaplex` |
| `hopper-memo` | Memo helper crate | Small and no-std oriented |
| `hopper-finance` | AMM, slippage, and DeFi math helpers | Unit-tested independent crate |
| `hopper-lending` | Lending math and health helpers | Unit-tested independent crate |
| `hopper-staking` | Staking reward helpers | Unit-tested independent crate |
| `hopper-vesting` | Vesting schedule helpers | Unit-tested independent crate |
| `hopper-distribute` | Split and fee distribution helpers | Unit-tested independent crate |
| `hopper-multisig` | Multisig helper crate | Small reusable companion crate |
| `hopper-anchor` | Anchor compatibility helpers | Keeps migration and interop concerns isolated |
| `hopper-manager` | Manifest-oriented manager library | Used by CLI and manager docs |
| `hopper-sdk` | Off-chain reader, builder, diff, and receipt SDK | Tests cover parsing, diffing, and receipt decoding |
| `hopper-cli` | Developer and release tool | Published; source-only publish gate and package verification pass |

## Feature Coverage by User Need

| User need | Hopper surface | Publication status |
|---|---|---|
| Define zero-copy account state | `hopper_layout!`, `#[hopper::state]`, Pod/wire types | Shipped and compile-tested |
| Validate account ownership and layout | `AccountView`, `LayoutContract`, headers, layout IDs | Shipped |
| Avoid duplicate mutable borrows | Segment borrow registry and typed segment APIs | Shipped |
| Use common SPL programs | Token, Token-2022, ATA, Memo, Metaplex helper crates | Shipped as companion crates |
| Generate clients / manifests | `hopper-schema`, CLI client commands | Shipped with layout-ID assertion tests |
| Explain or verify release artifacts | `hopper verify`, `hopper publish-check` | Source gate shipped; binary gate needs built `.so` and manifest |
| Migrate state versions | Migration modules and proc macros | Shipped |
| Emit audit-friendly mutation output | Receipts and SDK decoding | Shipped |
| Write examples from templates | Examples plus CLI lifecycle commands | Shipped; example crates are non-public by default |
| Compare CU claims | Sibling benchmark repository | Required before public performance claims |

## Release Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Main framework naming | Publish top-level package as `hopper-lang`; keep library crate name `hopper` |
| Companion crates must exist before `hopper-lang` packages cleanly | Completed for `0.1.0`; keep the same order for future releases |
| Benchmarks can be overclaimed | Keep release docs tied to `hopper-bench` artifacts and source-only publish checks |
| Framework surface is broad for a first release | Keep examples non-public, feature-gate optional surfaces, and document what is shipped versus planned |
| Unsafe zero-copy APIs need ongoing review | Maintain `UNSAFE_INVARIANTS.md`, compile-fail tests, and release gates |

## Final Readiness Statement

From code structure, crate coverage, tests, and CLI gates, Hopper has shipped a
conservative first public release as a zero-copy Solana state framework. Future
release docs should keep benchmark language tied to reproducible `hopper-bench`
artifacts and continue documenting the `hopper-lang` package alias
everywhere users install the framework.
