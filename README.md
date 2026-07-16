# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

Hopper is a zero-copy Solana program framework. Write programs with the Anchor shape you're used to. Hopper verifies owner, role, discriminator, version, layout fingerprint, or compact exact-size identity before account bytes reach typed state. No deserialize-then-hope path. No unchecked cast hiding in a macro.

The framework gives you Anchor ergonomics, Quasar direct-state speed, and an escape hatch when you need raw SVM control. One production runtime: direct Solana account memory through Hopper's typed handles, validation layer, and CPI surface.

Hopper is also the only framework in the 2026 low-CU field with its own substrate: Anchor v2 (alpha), Typhoon, and star-frame all build on Pinocchio, and Quasar shares its lineage, while `crates/hopper-native` has zero external dependencies. That one decision is what makes segment-level borrows, touch maps, and field-level write policies possible — see [docs/THE_MOAT.md](docs/THE_MOAT.md). Hopper is published on crates.io (hopper-lang 0.3.0) with a line-by-line audit trail and generated clients in 8 targets.

Three measured facts, provenance in [BENCHMARKS.md](BENCHMARKS.md) (2026-07-07 runs; vault four-way re-measured 2026-07-09):

- Hopper's safe, validated overlay measures at the same net CU as a raw unsafe pointer cast (1 CU each, Mollusk primitive lab).
- In the first published router-class three-way (Hopper vs Quasar vs hand-written Pinocchio), Hopper beats Quasar on every CU row (1,559/3,035/4,512 vs 1,582/3,064/4,546, 2026-07-09), lands within 1.8-2.4% of raw Pinocchio while carrying full framework services, and ships the smallest binary of the three.
- A complete deployable program fits in 3,736 bytes (2026-07-09 build of the counter example; the artifact deployed to devnet on 2026-07-07 was 4,688 bytes), about 0.027 SOL of rent-exempt deploy cost; the equivalent Anchor 0.31.1 artifact costs ~1.36 SOL to deploy.

For normal programs, use `hopper-lang` as `hopper`: `use hopper::prelude::*`, `#[account]`, `#[derive(Accounts)]`, `#[program]`, typed wrappers, checked CPI, and SPL helpers. For advanced state work, reach for `hopper::systems::*` to get segment leases, layout manifests, receipts, policies, and low-level state machinery.

## What's included

- no_std / no_alloc program crates by default.
- Direct Hopper account access. No serialize/deserialize boundary.
- `#[account]`, `#[derive(Accounts)]`, `#[program]`, `Ctx<T>`, `Account<'info, T>`, `InitAccount<'info, T>`, `Signer<'info>`, `Program<'info, P>`, `UncheckedAccount<'info>`.
- Zero-copy account loads guarded by owner, discriminator, version, layout ID, size, signer, writable, seed, and custom constraints.
- External account adapters for non-Hopper accounts: typed views, checked lenses, proof tokens, snapshots, lazy remaining parsing, SPL Token adapters.
- Checked CPI, signed CPI, stored instructions, Token and Token-2022 helpers, ATA, memo, and on-chain crypto.
- Systems-mode APIs for segmented layouts, dynamic tails, receipt trails, policy checks, schema manifests, migrations.
- Instruction touch maps (`touch-map` feature): enumerate the exact `(account, offset, size, read/write)` byte footprint an instruction touched, at measured 0 CU (`Context::for_each_touch`). Under capacity pressure the log coalesces exact unions instead of truncating, so contiguous workloads of any size emit a complete, verifier-conclusive map.
- Field-level write policies: `#[hopper::context(strict_writes)]` compiles declared mutable ranges into a static policy enforced at borrow acquisition — beyond Sealevel's account-level `writable` bit. Proven on compiled SBF bytecode and live devnet: a tampered handler's out-of-range write is refused with `Custom(0xD000 | idx)` before any byte changes ([examples/hopper-sentinel](examples/hopper-sentinel/README.md), signatures in the README).
- `Seq<'a, T>` growable typed sequence tails: O(1) push over a `[count][elems]` wire, capacity derived from the account length (the layout id never changes as it grows), declared under `strict_writes` as one open-ended `tail(...)` range that protects the fixed head and still refuses whole-account CPI delegation — where Anchor's `Vec<T>` pays a full deserialize + reserialize every instruction.
- The full migration suite: typed in-place `migrate_layout` with owner/writable gating baked into the runtime, `migrate(resize = grow|fit, payer = ...)` for payer-funded resizing (shrink refunds exactly the freed rent delta — never the deposits), `#[hopper::state(schema_epoch = N)]` + `#[account(epoch_migrate)]` for in-place epoch chains healed at bind, and `migrate_chain!` for typed multi-hop version chains with one up-front grow.
- Grillo: an independent byte-diff verifier (`grillo-manifest` + `grillo-verifier`) proving `changed ⊆ acquired ⊆ authorized` for any transaction against the program's published manifest and emitted touch map.
- `hopper lint --deny-escapes`: a CI-deniable audit that every account write in a program routes through the governed `Context` surface — the raw escape hatches are grep-able and machine-refused.
- Runtime-direction readiness, compile-gated until cluster activation: `simd-0321` (r2 instruction-data entrypoint) and `simd-0449` (O(1) account resolution from the pre-computed pointer table — one `from_raw_parts`, no stride walk).
- Opt-in 1-byte compact accounts for hot state: exact `[disc][body]` sizing on-chain, with layout fingerprints supplied by the manifest, IDL, registry, and generated SDK constants.
- CLI, schema, IDL, and code generation tools that understand Hopper layout fingerprints before decoding accounts.

## Current Release

Main framework: hopper-lang 0.3.0, imported as hopper. [Docs at docs.rs](https://docs.rs/crate/hopper-lang/0.3.0).

Install the CLI: `cargo install hopper-cli`.

All companion crates target 0.3.0: hopper-runtime, hopper-systems, hopper-derive, hopper-schema, hopper-native, hopper-solana, hopper-token, hopper-token-2022, hopper-associated-token, hopper-system, hopper-memo, hopper-finance, hopper-lending, hopper-staking, hopper-vesting, hopper-distribute, hopper-multisig, hopper-anchor, hopper-manager, hopper-sdk.

Benchmark snapshot: [BENCHMARKS.md](BENCHMARKS.md). Regenerate from the separate [hopper-bench](https://github.com/BluefootLabs/hopper-bench) repo before changing benchmark claims.

Generated clients: TypeScript, Kotlin, Python, Go, C header-only, off-chain Rust, Codama JSON, Anchor IDL JSON. Headered readers assert Hopper layout IDs from bytes `4..12` before decode; compact readers assert exact size plus discriminator and expose the layout fingerprint from manifest/IDL metadata. See [examples/hopper-compact-vault](examples/hopper-compact-vault/README.md).

Security users should review [AUDIT.md](AUDIT.md) and [docs/UNSAFE_INVARIANTS.md](docs/UNSAFE_INVARIANTS.md).

## Hopper in 30 seconds

Write state, declare accounts, mutate through checked wrappers:

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

By the time counter reaches the closure, Hopper has already checked the account role and layout contract.

## Quick start

### Deploy to devnet in 4 steps

```sh
hopper init my-program --template minimal --yes
cd my-program
hopper build
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/my_program-keypair.json
```

That's it. The counter example deployed to devnet at D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F as a full zero-copy program in 4,688 bytes; today's tree (2026-07-09, after the writable-sections fix) builds the same example at 3,736 bytes — about 0.027 SOL of rent-exempt deploy cost at the network's `(bytes + 128) x 6,960` lamport formula (the deployed 4,688-byte artifact was ~0.034 SOL), versus ~1.36 SOL for a 190 KiB Anchor-class artifact (see [BENCHMARKS.md](BENCHMARKS.md), deploy-cost economics). To decode a confirmed transaction:

```sh
hopper explain <CONFIRMED_SIG> --manifest hopper.manifest.json
```

hopper deploy defaults to devnet and refuses mainnet unless you pass --cluster mainnet-beta. See [docs/cli/](docs/cli/README.md) for deploy reference and [cli/SMOKE.md](cli/SMOKE.md) for an end-to-end runbook.

### Add to an existing crate

```sh
cargo add hopper-lang --rename hopper --features proc-macros
```

Or in Cargo.toml:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.3.0", features = ["proc-macros"] }
```

For development inside this repo:

```toml
[dependencies]
hopper = { path = "../Hopper-Solana-Zero-copy-State-Framework", package = "hopper-lang", features = ["proc-macros"] }
```

Public links:

- Framework crate: [crates.io/hopper-lang](https://crates.io/crates/hopper-lang)
- Docs: [docs.rs/hopper-lang](https://docs.rs/crate/hopper-lang/0.3.0)
- CLI crate: [crates.io/hopper-cli](https://crates.io/crates/hopper-cli)
- Website: [hopperzero.dev](https://hopperzero.dev)

Minimal example:

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

Init uses the same surface. After ctx.init_vault(), mutate the fresh account:

```rust
ctx.accounts
    .vault
    .with_mut_after_init(|vault| vault.set_inner(*ctx.accounts.payer.key(), 0, 0))?;
```

## Docs

Start here:
- [docs/README.md](docs/README.md): docs index.
- [docs/FIRST_FIVE_MINUTES.md](docs/FIRST_FIVE_MINUTES.md): counter, vault, dynamic multisig, token transfer, raw escape hatch.
- [docs/GETTING_STARTED_SERIOUS.md](docs/GETTING_STARTED_SERIOUS.md): source-first setup and first serious flow.
- [docs/HOPPER_LAYERS.md](docs/HOPPER_LAYERS.md): framework mode, structured state, systems mode, mental mapping vs Anchor/Quasar.
- [docs/WRITING_HOPPER_PROGRAMS.md](docs/WRITING_HOPPER_PROGRAMS.md): Hopper patterns and program structure.

Advanced:
- [docs/PROFILING.md](docs/PROFILING.md): hopper profile elf, binary artifacts, benchmark commands.
- [docs/PROTOCOL_GRADE_EXAMPLES.md](docs/PROTOCOL_GRADE_EXAMPLES.md): receipt indexing, compatibility reports, migrations, typed cross-program reads, segment leases.
- [docs/POLICY_GUARANTEES.md](docs/POLICY_GUARANTEES.md): capability policy, sealed/raw/hybrid access, policy-vault example.
- [docs/MIGRATION_FROM_ANCHOR.md](docs/MIGRATION_FROM_ANCHOR.md): Anchor to Hopper.
- [docs/MIGRATION_FROM_QUASAR.md](docs/MIGRATION_FROM_QUASAR.md): Quasar to Hopper.
- [docs/HOPPER_VS_QUASAR.md](docs/HOPPER_VS_QUASAR.md): Quasar casts vs Hopper checks.
- [docs/THE_MOAT.md](docs/THE_MOAT.md): what compounds on the borrow ledger and sovereign substrate, and what any competitor can copy in a weekend — every claim cited to file, symbol, and test.
- [docs/PORT_QUASAR_IN_20_MINUTES.md](docs/PORT_QUASAR_IN_20_MINUTES.md): bounded-tail vault/multisig port guide.
- [docs/DYNAMIC_TAILS_FROM_QUASAR.md](docs/DYNAMIC_TAILS_FROM_QUASAR.md): Quasar dynamic fields to Hopper fixed-body plus compact tail.
- [docs/TOKEN_2022_GUIDE.md](docs/TOKEN_2022_GUIDE.md): zero-copy Token-2022 extension policy and constraint syntax.
- [docs/CRYPTO_CAPABILITIES.md](docs/CRYPTO_CAPABILITIES.md): Solana crypto helpers, precompile checks, feature-gated heavy wrappers.
- [docs/CLI_REFERENCE.md](docs/CLI_REFERENCE.md): lifecycle, schema, client, profiling, compatibility gates, Actions/mobile/test scaffolds, manager commands.

## Progressive learning path

Hopper layers so you don't learn systems mode first:

1. Framework mode: hopper::prelude, #[account], #[program], typed wrappers, PDA helpers, token modules, guard macros.
2. Structured state: keep #[account] and add bounded fields like String<'a, 32> or Vec<'a, Address, 10>. Hopper lowers them into fixed-body plus compact-tail. Use TailStr<'a> or TailBytes<'a> only when a protocol deliberately needs a final named field consuming remaining tail.
3. Systems mode: add hopper::systems, hopper::segment, hopper::receipt, hopper::policy, hopper::migration, hopper::interface for field leasing, audit trails, upgrades, and cross-program layout contracts.
4. Substrate mode: use hopper::substrate for direct Hopper Native tools like account views, CU budget probes, hashes, PDA, input parsing, memory helpers, syscalls.

## Access tiers

Normal handlers use ctx.accounts.* plus get() / get_mut() on typed wrappers. Reach for lower-level access only when a protocol explicitly needs systems-mode control:

1. segment_ref_typed / generated field accessors: default hot path for field-level borrow leasing.
2. get / get_mut on Account<'info, T>: validated whole-layout access.
3. segment_ref_const / dynamic segment_ref: advanced runtime-selected segment access.
4. `raw_ref` / `raw_mut` - unsafe typed escape hatch.
5. `as_mut_ptr` - full raw pointer escape for policy-controlled raw mode.

For variable-length data, use Quasar-style bounded fields directly in #[account]:

```rust
#[hopper::account(discriminator = 10, version = 1)]
pub struct Multisig<'a> {
  pub threshold: WireU64,
  pub label: String<'a, 32>,
  pub signers: Vec<'a, Address, 10>,
}
```

Hopper's typed overlays require alignment-1 Pod types. Use WireU64/WireI64/WireU128 for multi-byte scalar fields; native u64/i64/u128 are intentionally rejected for typed overlay APIs.

The source stays readable. The wire truth stays explicit: fixed body, u32 tail length, compact tail payload. Address/Pubkey vectors keep the borrowed zero-copy view; other T: TailElement vectors use HopperVec<T, N> through the same codec/editor path. Use #[hopper::dynamic_account] with #[tail(...)] when you want the systems-mode tail shape spelled out. For Quasar-style final tails, spell the last field as TailStr<'a> or TailBytes<'a>; Hopper fingerprints it as tail_str or tail_bytes.

Handlers with variable tails use generated remaining-account accessors: ctx.remaining_accounts() is strict and duplicate-rejecting, ctx.remaining_accounts_passthrough() preserves duplicates when protocol needs it, and ctx.remaining_accounts().signers::<N>()? validates bounded multisig signer lists without allocation.

## Repo structure

| Path | Purpose |
| --- | --- |
| . (hopper-lang) | Main framework: accounts, programs, CPI, PDA, prelude. |
| crates/hopper-runtime | Runtime: account views, borrow tracking, CPI, backend compat. |
| crates/hopper-core (hopper-systems) | State architecture: ABI types, headers, layouts, segments, policies, receipts. |
| crates/hopper-macros | Declarative macro surface. |
| crates/hopper-macros-proc (hopper-derive) | Proc-macro authoring. |
| crates/hopper-native | Native low-level backend. |
| crates/hopper-schema | Schema, IDL, Codama projection, layout manifests. |
| crates/hopper-system | System-program helpers. |
| crates/hopper-solana | Solana interop. |
| crates/hopper-spl | Token, Token-2022, ATA, Metaplex helpers. |
| crates/hopper-manager | Manifest-driven account inspection. |
| crates/hopper-sdk | Client-side SDK surface. |
| tools/hopper-cli | hopper CLI: linting, schema export, inspect, profile. |
| examples | Example programs. |
| docs | Design notes, unsafe invariants, audit/recovery. |

The old split repos were folded back with subtree history preserved, then archived.

Companion repos:
- [hopper-bench](https://github.com/BluefootLabs/hopper-bench): benchmark harness and CU lab.
- [hopper-svm](https://github.com/BluefootLabs/hopper-svm): in-process Solana execution harness.

## Tools and commands

CLI source in tools/hopper-cli. Supports lifecycle, linting, solana-check, schema/IDL export, manifest inspection, account decode, client generation, Solana Actions scaffolds, mobile bindings, security test matrices, manager workflows, profiling.

Quick reference:

```sh
cargo metadata --no-deps --format-version 1
cargo test -p hopper-cli cmd::lint::tests -- --nocapture
cargo test -p hopper-lang --features proc-macros,metaplex --test constant_integration -- --nocapture
```

## Examples

Framework examples:
- [examples/hopper-counter](examples/hopper-counter): minimal #[derive(Accounts)], Ctx<T>, ctx.accounts.* flow.
- [examples/hopper-vault](examples/hopper-vault): SOL vault using typed wrappers, set_inner, checked helpers, System transfer.
- [examples/hopper-escrow](examples/hopper-escrow): token-escrow using same facade.
- [examples/quasar-port-20-min](examples/quasar-port-20-min): Quasar-style bounded dynamic port with Hopper guarantees.
- [examples/hopper-devnet-audit](examples/hopper-devnet-audit): deployable devnet audit covering dynamic tails, contexts, segments, receipts, Token-2022 policy, field capabilities, substrate probes.
- [examples/hopper-argus-guard](examples/hopper-argus-guard): Argus-style risk guard with checked exposure and authority-bound state.

Systems mode examples:
- [examples/hopper-proc-vault](examples/hopper-proc-vault): generated/lowered account access for teams inspecting macro output.
- [examples/hopper-policy-vault](examples/hopper-policy-vault): strict, sealed, raw, hybrid handlers side by side.
- [examples/hopper-showcase](examples/hopper-showcase): broad feature tour.

Raw and benchmark examples:

- [examples/hopper-parity-vault](examples/hopper-parity-vault): apples-to-apples benchmark target with intentionally low-level lamport mutation.
- [examples/hopper-token-2022-vault](examples/hopper-token-2022-vault) and [examples/hopper-token-2022-ata](examples/hopper-token-2022-ata): Token-2022 low-level validation and CPI examples.

For in-process tests, use the sibling [hopper-svm](https://github.com/BluefootLabs/hopper-svm) repo as a dev-dependency.

## Benchmarks

The benchmark suite is maintained as a separate product repo:
[hopper-bench](https://github.com/BluefootLabs/hopper-bench)

Do not copy old benchmark numbers from this README. Regenerate numbers from the
benchmark repo before publishing performance claims.

The current same-provenance vault snapshot (re-measured 2026-07-09, four-way)
includes Hopper, the in-tree Anza Pinocchio target, Quasar's upstream vault
target, and a measured Anchor 0.31.1 comparator. Quasar implements only the
financial `deposit` / `withdraw` rows, so validation-only rows are marked `n/a`
rather than synthesized. In that run the Hopper vault `.so` also measures
smaller than Pinocchio's on the identical contract (7.46 vs 7.73 KiB);
Quasar's 5.47 KiB is still the smallest vault artifact. The same repo also
carries the first published router-class
three-way (Hopper / Quasar / hand-written Pinocchio, 2026-07-09): Hopper beats Quasar on every row, within 1.8-2.4% of
raw Pinocchio per hop, with the smallest binary of the three. See
[BENCHMARKS.md](BENCHMARKS.md) for both tables and provenance.

Treat the vault table as a measurement of that vault contract, not a universal
ranking. Within it, the facts are plain: Hopper won both rows Quasar's own
upstream vault implements (deposit and withdraw) under one lockfile, toolchain,
and seed set — and Quasar publishes no comparative CU benchmark of its own.
Re-run the benchmark repo at current heads before publishing fresh performance
language.

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

Hopper also maintains a competitor-bug-class regression suite: 18 pinned tests
that turn documented bug classes from other frameworks (CPI return-data UB,
self-close lamport imbalance, stale migration state, overstated
remaining-capacity, duplicate-account aliasing, and the two Anchor v2
alpha Slab classes, #4603 and #4616) into Hopper regression proofs.
Authoring that suite found and fixed a real Hopper bug (`safe_close` accepted
an aliased destination) — the framework audits itself.

Verification lanes beyond the test matrix: Kani proofs over the raw-input
parser and tail codecs (`scripts/kani-*.sh`), and a Miri lane under Tree
Borrows over the aliasing core — the segment borrow ledger, write-policy
gate, native-boundary transmutes, and borrow registry
(`scripts/miri-core.sh`). The Miri lane caught and fixed two real
UB classes in test fixtures on its first run; that is what it is for.

See:

- `docs/UNSAFE_INVARIANTS.md`
- `AUDIT.md`
- `crates/hopper-core/tests/unsafe_boundary_tests.rs`
- `crates/hopper-core/tests/overlay_equivalence_tests.rs`
- `crates/hopper-runtime/tests/competitor_bug_classes.rs` and
  `crates/hopper-core/tests/competitor_bug_classes.rs`
- [docs/THE_MOAT.md](docs/THE_MOAT.md) for which guarantees are structural

## Support

Hopper is open-source Solana infrastructure. Public-goods support and donations
can be sent to `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

Donation URI: <solana:F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT?label=solanadevdao.sol>

## License

Licensed under either of:

- MIT license (`LICENSE-MIT`)
- Apache License, Version 2.0 (`LICENSE-APACHE`)

at your option.
