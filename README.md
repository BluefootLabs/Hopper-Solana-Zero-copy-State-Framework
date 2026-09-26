# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

Hopper is a zero-copy Solana program framework for Rust developers building
on-chain trading, token-claim, marketplace, and governance logic. Declare accounts and
handlers, work with stored state in place, and define what each instruction
may change. Combine the framework with your application rules and integrations.

## Start with what you want to build

| What you want to build | Available starting point | What you add |
|---|---|---|
| Trading and settlement | [Funded token escrow, intent settlement, and order storage](https://hopperzero.dev/docs/use-cases#trading-and-settlement) | Product rules and production integrations; the orderbook example has no matching or settlement |
| Token claims and airdrops | [Claim building blocks](https://hopperzero.dev/docs/use-cases#token-claims-and-airdrops): account state, token CPI, vesting and distribution math | Eligibility, claim replay protection, funded custody, and recovery; no complete airdrop template is claimed |
| NFT and cNFT markets | [Marketplace integration guide](https://hopperzero.dev/docs/use-cases#nft-and-cnft-markets): state and Token Metadata helpers | Listing and settlement logic; Bubblegum cNFT support requires a custom integration |

For multisig and DAO treasuries, see the
[governance guide](https://hopperzero.dev/docs/governance): available building
blocks, a pinned Squads source review, and the application rules still needed.

For a first working program, start with the
[SOL vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-vault/src/lib.rs).
For on-chain limits and scoped writes, explore
[delegated application quotas](https://hopperzero.dev/docs/byte-allowance).

Start with typed accounts and ordinary Rust handlers. Add checked cross-program
calls, collections, state migrations, and generated clients as needed. Headered
layouts validate version and layout identity; compact layouts use a smaller
discriminator and size contract.

**Solana locks accounts. Hopper governs bytes.** Declared write policies are
enforced on Hopper-tracked access paths inside your program. Raw access needs
separate review. Byte policies do not create sub-account parallelism or an
automatic fee discount. Optional inspection tools help explain execution;
they are not required to enforce those on-chain checks.

Hopper owns its zero-dependency substrate in `crates/hopper-native`. That
boundary gives the framework one place to enforce borrows, write contracts,
and post-CPI checks while still exposing low-level control. This is the 0.4.0
release source; registry publication evidence is tracked on the
[release status page](https://hopperzero.dev/docs/release-status). The independently runnable `grillo-*`
and `hopper-topology` workspace packages are versioned 0.1.0; "independent"
means a separate recomputation boundary, not a third-party audit.

Four measured facts, with provenance in [BENCHMARKS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/BENCHMARKS.md):

- In the clean 2026-08-16 same-behavior vault matrix, Hopper measured 1,578 CU for deposit and 424 CU for withdraw. The run used eight samples and passed all 30 rollback gates.
- That run produced a 9,032-byte Hopper binary. Quasar's pinned beta snapshot produced the smallest binary in the matrix, while Pinocchio measured lower on the separate missing-signature failure row.
- Under pina's cross-framework fixtures, rebuilt with pina's recipe and a verifier that reproduces pina's published pinocchio numbers exactly (2026-09-24), the Hopper substrate hello world is 1,656 bytes at 116 CU, the smallest binary in that table. The Hopper macro counter is 8,312 bytes and initializes in 1,549 CU against Pina 3,301, Anchor v2 3,458, and Quasar 3,488; it increments in 349 against Pina 1,753, Anchor 2,117, and Quasar 330, with owner, header, and layout validation. The macro hello remains 138 CU against Anchor's 127 and Quasar's 115, and the like-for-like substrate counter is 6,728 bytes to pinocchio's 6,512. Account layouts, bump handling, and enabled checks differ as documented; competitor rows are pinned published results. See [bench/framework-comparison](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/bench/framework-comparison).
- The result is one locked contract under one toolchain, not a universal framework ranking. Exact source pins, artifact hashes, and the evidence archive are recorded below.

For normal programs, use `hopper-lang` as `hopper`: `use hopper::prelude::*`, `#[account]`, `#[derive(Accounts)]`, `#[program]`, typed wrappers, checked CPI, and SPL helpers. For advanced state work, reach for `hopper::systems::*` to get segment leases, layout manifests, receipts, policies, and low-level state machinery.

The 0.3.2 registry train contains 26 framework/CLI updates plus three unchanged
0.1.0 support packages. Fresh registry-only allowance and policy-probe programs
pass four compiled suites and exactly reproduce the tested, deployed v0 ELFs.
The September 25 devnet capture finalized 62 focused transactions: 40 allowance,
20 runtime-gate, and two orderbook transactions. Allowance consumption measured
889 CU and a limit update 831 CU, with complete expected account-state checks.
See the [on-chain evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/onchain-byte-policies-2026-09-25)
and [publication receipts](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/registry-publication-2026-09-25).

The preceding [0.3.1 evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/framework-refinement-2026-09-24)
retains 45 focused devnet transactions for mint plans, canonical bump persistence,
and rollback after successful nested mint CPIs. Its mint and PDA compiled suites
also pass on v0 and v3 in the 0.3.2 release gates.

## The 0.4.0 update

Named fixed-field inputs and `init_<account>_with(values)` keep initialization
in ordinary Rust. Existing constructors, explicit lifecycle calls, and borrowing
APIs remain available. Wire layouts stay unchanged. The release also fixes
header/body offsets in typed DSL wrappers, independently checks actual Rust
type bounds before safe projections, and rejects malformed segment geometry.
See [named initialization](https://hopperzero.dev/docs/named-initialization) and
[the 0.4 migration guide](https://hopperzero.dev/docs/migration-0-4).

The named SOL vault passed 19 finalized devnet transactions with complete
expected account-state checks and identical deployed ELF bytes before and
after testing. Deposit measured 1,602 CU and withdrawal 240 CU. Local gates
passed host tests, clippy, forged-size rejection builds, SBF v0/v3 fixtures,
and all 698 Cicada host semantic cases. These are scoped fixture results;
the dated peer measurements above were not rerun for this release.
See [the 0.4 evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/dx-memory-safety-2026-09-25).

## What's included

Version 0.3.2 adds [captured-selector cell accessors](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/ONCHAIN_BYTE_POLICIES.md)
and an on-chain quota example. A handler uses `ctx.book_spent_cell_mut()` to
acquire the element selected during binding, with its type and byte offset
inferred. The policy enforces the write in the program; Grillo and other
off-chain tools are optional inspection layers. These accessors are available
from the published 0.3.2 framework.

The [2026-09-24 refinement](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/SOURCE_REVIEW_2026-09-24.md)
adds exact-size mint creation plans,
automatic Token-2022 ownership checks for extension constraints, and retained
bumps for more direct binding paths. These APIs require 0.3.1; see the
[release status page](https://hopperzero.dev/docs/release-status) for registry availability.

The [2026-09-23 review](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/SOURCE_REVIEW_2026-09-23.md)
adds canonical literal-PDA derivation, correct canonical validation for
inferred bumps, retained validated bumps during direct binding, and finalized
CLI feature prerequisites. It records current peer source pins and cluster
observations, plus the remaining Cicada artifact-policy and Grillo replay work.

The [canonical-PDA evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/canonical-pda-2026-09-23)
records 71 CU for a literal constant check versus 1,074 CU for runtime search
in the same SBF v0 fixture. All 25 focused devnet transactions finalized with
the expected results and unchanged account snapshots. Both SBF architectures
also passed exhaustive bump refusal checks. These are fixture results, not
whole-program or cross-framework performance claims.

The [2026-09-22 source review](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/SOURCE_REVIEW_2026-09-22.md)
connects current peer changes to concrete Hopper fixes: signing-compatible
PDA seed bounds, client-side Cicada route checks, and unambiguous Grillo
snapshot evidence. The root README is also the `hopper-lang` crate README;
package-specific API details live with each companion crate.

- no_std / no_alloc program crates by default.
- Direct Hopper account access. No serialize/deserialize boundary.
- `#[account]`, `#[derive(Accounts)]`, `#[program]`, `Ctx<T>`, `Account<'info, T>`, `InitAccount<'info, T>`, `Signer<'info>`, `Program<'info, P>`, `UncheckedAccount<'info>`.
- Zero-copy account loads guarded by owner, discriminator, version, layout ID, size, signer, writable, seed, and custom constraints.
- PDA checks distinguish canonical search from selected-bump verification. Bare `bump` and `seeds_fn` search for the canonical off-curve address; typed accounts with an explicit or stored bump use one `sol_sha256`. Eligible empty, nonsigner `init` accounts defer supplied-bump verification to the signed creation CPI. `hopper::canonical_pda!` derives literal seeds and a canonical bump at build time, leaving an address comparison on chain.
- External account adapters for non-Hopper accounts: typed views, checked lenses, proof tokens, snapshots, lazy remaining parsing, SPL Token adapters.
- Checked CPI, signed CPI, stored instructions, Token and Token-2022 helpers, ATA, memo, and on-chain crypto.
- Systems-mode APIs for segmented layouts, dynamic tails, receipt trails, policy checks, schema manifests, migrations.
- Instruction touch maps (`touch-map` feature): enumerate the exact `(account, offset, size, read/write)` byte footprint observed through Hopper-tracked borrows. The documented touch-map-enabled smoke case measured +52 CU for ambient write observability; programs with the feature disabled pay none of it. Under capacity pressure the log coalesces exact unions instead of truncating, so contiguous same-kind workloads of any size emit a complete, verifier-conclusive map. Completeness requires denying or separately reviewing raw mutation escapes.
- Field-level write policies: `#[hopper::context(strict_writes)]` compiles declared mutable ranges into a static policy enforced at borrow acquisition, beyond Sealevel's account-level `writable` bit. Compiled-SBF tests exercise refusal before mutation. A dated 2026-07-14 devnet run recorded the same refusal shape for that earlier build; it is historical evidence, not an attestation of this release source. See [examples/hopper-sentinel](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-sentinel).
- `Seq<'a, T>` growable typed sequence tails: O(1) push over a `[count][elems]` wire, capacity derived from the account length (the layout id never changes as it grows), declared under `strict_writes` as one open-ended `tail(...)` range that protects the fixed head. With an explicit `lamports(...)` declaration, validated CPI helpers also refuse whole-account writable delegation for this tail-only grant. This avoids the full deserialize and reserialize path used by a Borsh-backed `Vec<T>` in Anchor v1; Anchor v2's published 2.0.0-rc.1 line includes separate PodVec and Slab work and remains self-described Alpha upstream.
- Collection DX without hidden serialization: bounded `Vec<'a, T, N>` and `String<'a, N>`, growable `Seq<'a, T>`, and O(1) `Slab<'a, T>` / `TailSlab<'a, T>` allocation over account bytes. `Slab` exposes collection-style `len`, `is_empty`, `capacity`, and `remaining_capacity` while retaining stable slot IDs, bitmap validation, double-free refusal, and corruption guards.
- Single-CPI account creation: `init` and `init_if_needed` create every account with the System Program's `CreateAccountAllowPrefund` (instruction 13, active on all public clusters). A pre-funded account is topped up by exactly its shortfall in the same CPI, and a fully funded one sends a zero delta that the System Program ignores, so one CPI body serves both. The old CreateAccount branch plus the Transfer, Allocate, Assign fallback are gone from the generated code.
- Generated `realloc_<field>()` accessors refuse a resize below the field layout's `required_len()`, so a handler cannot shrink an account into a state its own layout can never load again.
- The full migration suite: typed in-place `migrate_layout` with owner/writable gating baked into the runtime, `migrate(resize = grow|fit, payer = ...)` for payer-funded resizing (shrink refunds exactly the freed rent delta, never the deposits), `#[hopper::state(schema_epoch = N)]` + `#[account(epoch_migrate)]` for in-place epoch chains healed at bind, and `migrate_chain!` for typed multi-hop version chains with one up-front grow.
- Manifest-derived adversarial execution: `hopper fuzz generate` deterministically expands layouts, instruction roles, Accounts-derived PDA, typed, lifecycle, and relational constraints, aliases, write ranges, declared compatibility pairs, declared lamport effects, policies, argument bounds, and remaining-account ceilings into seeded cases; `hopper fuzz check` gates contract drift, and `hopper fuzz run` executes every case and required invariant hook through an application adapter. A checked plan alone is planning evidence, not proof that a program ran.
- Stable CU regression budgets and diffs: `hopper profile bench` reads the separate `hopper-bench` lab automatically when it is a sibling checkout, supports an explicit `--bench-root`, and gates measured rows against `cu_baselines.toml`.
- Versioned release-interface binding: `hopper::program_manifest!` embeds a canonical SHA-256 commitment to the program identity, layouts, instruction surface, events, policy contracts, and context fields represented by `ProgramManifest`. `hopper verify --release` requires the compiled ELF to carry that exact commitment. This binds the manifest-projected declaration to the artifact; it does not cover constraints absent from the manifest or prove handler behavior, deployment identity, or artifact freshness.
- Grillo: a separately runnable offline verifier maintained in this Hopper workspace (`grillo-manifest` + `grillo-verifier`). The current workspace CLI uses the v0.1 evidence format and recomputes `changed ⊆ acquired ⊆ authorized` from caller-supplied snapshots and touch evidence. The experimental Effect ABI v0.2 library surface additionally models full account transitions, supplied deployment/artifact identity, remaining-account grammar, and nested CPI. It checks that caller-supplied identity fields agree; it does not authenticate that frame or prove ledger provenance, and a v0.2 PASS requires invocation-entry/exit evidence. Both packages are versioned 0.1.0; v0.2 names an evidence/schema version, not a crate release. Grillo has no on-chain entrypoint and costs 0 SOL to deploy. `grillo verify m.json bundle.json` exits 0 PASS / 2 VIOLATION / 3 INCONCLUSIVE for the v0.1 bundle format.
- Upgrade authority gate: `hopper verify --authority-baseline <old-manifest>` (and `grillo authority-diff old new`) fails a release when any instruction gains authority. It flags a dropped signer, a newly writable account, byte ranges that reach another layout field (compared per field, so a field that only moved is not a new permission), a removed exact-cell rule, a new lamport permission, and weaker context constraints such as removed PDA seeds, `has_one`, owner, or address checks. A PDA seed swap or a different CPI program goes to review. Exit 2 means widened and exit 3 means review. A reviewed report approves its widenings only for the exact manifest pair whose SHA-256 digests it records. Under `--release`, `--baseline-so` or `--baseline-program <id>` must bind the old manifest to its released ELF, and `--candidate-buffer <addr>` binds the new manifest to the loader Buffer holding the pending upgrade, both read from the cluster, so neither side is an unbound declaration. IDL compatibility checkers answer whether callers break and treat these relaxations as additive; this gate answers whether the program gained power. It compares declarations and does not prove handler behavior.
- `hopper lint --deny-escapes`: a CI-deniable textual audit that rejects known ledger-bypassing accessor spellings in scanned project source. It is a review aid, not semantic proof against arbitrary Rust, FFI, dependency, or raw-backend mutation; those paths require explicit review.
- Runtime-direction readiness as opt-in Cargo features: `simd-0321` (r2 instruction-data entrypoint; gate active on all three clusters in the 2026-09-24 observations, kept opt-in because the r2 path measured CU-neutral for +368 bytes of `.text`) and `simd-0449` (O(1) account resolution from the pre-computed pointer table, one `from_raw_parts`, no stride walk; gate active on testnet and devnet, pending mainnet-beta).
- Opt-in 1-byte compact accounts for hot state: fixed compact layouts use exact `[disc][body]` sizing; compact-dynamic layouts admit bytes after a fixed minimum prefix. Layout identity comes from the manifest, IDL, and generated SDK constants. The optional registry data model can carry the same identity, but Hopper does not ship its lifecycle or automatic consumers.
- Headered Manager and generated-client decoders compare the `LAYOUT_ID` stored at bytes `4..12` before reading fields. Compact account bytes carry no fingerprint: the on-chain loader and all six generated SDKs check the discriminator, then require exact size for fixed layouts or the minimum prefix size for compact-dynamic layouts. Generated compact readers expose the manifest/IDL fingerprint as external identity metadata; they decode declared fixed fields but do not authenticate or validate a dynamic tail's application-specific payload. Raw-header Manager commands currently require the 16-byte headered form.

## Versioning

The September 25 runtime-policy fixture completed 20 finalized devnet
transactions, including typed selected-cell checks and the original ambient
gate cases. Its tested and deployed ELF matched before and after capture.
This focused proof covers the policy fixture; Cicada's current evidence is
23 local compiled lifecycle tests and 698 host semantic adapter cases.

Main framework: `hopper-lang` 0.4.0, imported as `hopper`. Keep the framework
and CLI on the same release line when using generated code and new APIs.
[Docs at docs.rs](https://docs.rs/crate/hopper-lang).
The facade pins its matching derive crate. See the
[0.4 migration notes](https://hopperzero.dev/docs/migration-0-4) for new generated
names, corrected body offsets, and stricter memory bounds; wire layouts are unchanged.

Install this release's CLI with `cargo install hopper-cli --version 0.4.0 --locked`.
Install this checkout's CLI with
`cargo install --path tools/hopper-cli --locked`.

The framework companion crates are versioned 0.4.0 in the workspace: hopper-runtime, hopper-systems, hopper-derive, hopper-macros, hopper-schema, hopper-native, hopper-solana, hopper-token, hopper-token-2022, hopper-associated-token, hopper-metaplex, hopper-system, hopper-memo, hopper-builtins, hopper-finance, hopper-lending, hopper-staking, hopper-vesting, hopper-distribute, hopper-multisig, hopper-anchor, hopper-manager, hopper-sdk, hopper-svm.

Benchmark snapshot: [BENCHMARKS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/BENCHMARKS.md). Regenerate from the separate [hopper-bench](https://github.com/BluefootLabs/hopper-bench) repo before changing benchmark claims.

Generated interop formats: TypeScript, Kotlin, Python, Go, C header-only, off-chain Rust, Hopper public IDL, Codama JSON, and (when the source is losslessly representable) current Solana IDL v0.1.0 JSON. That is six SDKs plus three interchange formats; the full manifest and lowered Rust audit preview are separate artifacts. `hopper compile --emit idl` emits Hopper's public IDL; the Solana projection is the separate `hopper schema export --anchor-idl <manifest> --program-id <pubkey>` path. That projection is fail-closed: it currently refuses Cicada because Hopper's u16-prefixed bounded `route_data` and `execute_intent` remaining-account contract cannot be represented faithfully. Its `--program-id` value is the expected address encoded in the IDL; export does not query RPC or prove that address is deployed. The projection publishes Hopper's exact wire discriminators and marks account bodies as custom `hopper-zero-copy-v1` serialization, so account bodies require a Hopper-aware decoder rather than Anchor Borsh. Headered readers assert Hopper layout IDs from bytes `4..12` before decode. Fixed compact readers assert exact size plus discriminator; compact-dynamic readers assert minimum prefix size plus discriminator. Both expose the layout fingerprint from manifest/IDL metadata. See [examples/hopper-compact-vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-compact-vault).

Security users should review [SECURITY.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/SECURITY.md) and [docs/UNSAFE_INVARIANTS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/UNSAFE_INVARIANTS.md).

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

`hopper deploy` builds from the lockfile, defaults to devnet, and refuses
mainnet unless `--cluster mainnet-beta` is explicit. A successful deployment
proves the artifact loaded; use the example transaction tests to prove program
behavior. Before sending anything, `hopper deploy --dry-run --cluster devnet`
queries live rent for the exact ELF and reports permanent loader-v3 rent,
recycled buffer working capital, and excluded fees separately. Dated historical
deployment costs and binary sizes remain in [BENCHMARKS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/BENCHMARKS.md). To
decode a confirmed transaction:

```sh
hopper explain <CONFIRMED_SIG> --manifest hopper.manifest.json
```

See [docs/cli/](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/cli/README.md) for deploy reference and
[cli/SMOKE.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/cli/SMOKE.md) for an end-to-end runbook. The finalized devnet evidence for six example lanes and the ledger-bound authority review, captured on 2026-09-19 with program ids, slots, and artifact hashes, is recorded in [docs/DEVNET_RELEASE_EVIDENCE.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/DEVNET_RELEASE_EVIDENCE.md) and archived under [audit/devnet-evidence-2026-09-19](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/devnet-evidence-2026-09-19).

### Add to an existing crate

```sh
cargo add hopper-lang@0.4.0 --rename hopper --features proc-macros
```

The exact dependency for this release is:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "=0.4.0", features = ["proc-macros"] }
```

To develop against a checkout, use a local path:

```toml
[dependencies]
hopper = { path = "../Hopper-Solana-Zero-copy-State-Framework", package = "hopper-lang", features = ["proc-macros"] }
```

Public links:

- Framework crate: [crates.io/hopper-lang](https://crates.io/crates/hopper-lang)
- Docs: [docs.rs/hopper-lang](https://docs.rs/crate/hopper-lang)
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

Initialize a fresh account with named values generated from its fixed fields:

```rust
ctx.init_vault_with(VaultFields {
    authority: *ctx.accounts.payer.key(),
    balance: 0,
    bump: 0,
})?;
```

Explicit `init_vault()`, checked borrow guards, `with_mut` closures, positional
`set_inner`, and low-level helpers remain available. The composed helper is for
explicit fresh initialization; propagate errors for transaction rollback.
Dynamic tails are initialized separately. See the
[named initialization guide](https://hopperzero.dev/docs/named-initialization).

## Docs

Start here:
- [docs/README.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/README.md): docs index.
- [docs/FIRST_FIVE_MINUTES.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/FIRST_FIVE_MINUTES.md): counter, vault, dynamic multisig, token transfer, raw escape hatch.
- [docs/GETTING_STARTED_SERIOUS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/GETTING_STARTED_SERIOUS.md): source-first setup and first serious flow.
- [docs/HOPPER_LAYERS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/HOPPER_LAYERS.md): framework mode, structured state, systems mode, mental mapping vs Anchor/Quasar.
- [docs/WRITING_HOPPER_PROGRAMS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/WRITING_HOPPER_PROGRAMS.md): Hopper patterns and program structure.

Advanced:
- [docs/PROFILING.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/PROFILING.md): hopper profile elf, binary artifacts, benchmark commands.
- [docs/PROTOCOL_GRADE_EXAMPLES.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/PROTOCOL_GRADE_EXAMPLES.md): receipt indexing, compatibility reports, migrations, typed cross-program reads, segment leases.
- [docs/COLLECTIONS_AND_RESIZING.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/COLLECTIONS_AND_RESIZING.md): bounded fields, growable `Seq`, stable-ID `Slab`, and safe grow/fit migrations.
- [docs/POLICY_GUARANTEES.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/POLICY_GUARANTEES.md): capability policy, sealed/raw/hybrid access, policy-vault example.
- [docs/MIGRATION_FROM_ANCHOR.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/MIGRATION_FROM_ANCHOR.md): Anchor to Hopper.
- [docs/MIGRATION_FROM_QUASAR.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/MIGRATION_FROM_QUASAR.md): Quasar to Hopper.
- [docs/HOPPER_VS_QUASAR.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/HOPPER_VS_QUASAR.md): Quasar casts vs Hopper checks.
- [docs/EFFECT_ABI_V0_1.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/EFFECT_ABI_V0_1.md) and [docs/EFFECT_ABI_V0_2.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/EFFECT_ABI_V0_2.md): the framework-neutral effect ABI, Grillo verification, and the v0.2 commitment-binding model.
- [docs/PORT_QUASAR_IN_20_MINUTES.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/PORT_QUASAR_IN_20_MINUTES.md): bounded-tail vault/multisig port guide.
- [docs/DYNAMIC_TAILS_FROM_QUASAR.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/DYNAMIC_TAILS_FROM_QUASAR.md): Quasar dynamic fields to Hopper fixed-body plus compact tail.
- [docs/TOKEN_2022_GUIDE.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/TOKEN_2022_GUIDE.md): zero-copy Token-2022 extension policy and constraint syntax.
- [docs/CRYPTO_CAPABILITIES.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/CRYPTO_CAPABILITIES.md): Solana crypto helpers, precompile checks, feature-gated heavy wrappers.
- [docs/AUDIT_READINESS_DOSSIER_2026-08-15.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/AUDIT_READINESS_DOSSIER_2026-08-15.md): executable audit-readiness, peer parity, and adoption blockers.
- [docs/ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md): pinned Anchor v2, Quasar, Pinocchio, Star Frame, Steel, Cicada, and Sentinel audit.
- [docs/COMPETITIVE_REFRESH_2026-09-19.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/COMPETITIVE_REFRESH_2026-09-19.md): dated competitor and network delta, including transaction v1 activation, 5,080-lamport rent, and the authority-gate positioning.
- [docs/SOLANA_NETWORK_BASELINE_2026-08-15.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/SOLANA_NETWORK_BASELINE_2026-08-15.md): confirmed Mainnet versus upcoming protocol behavior.
- [docs/CLI_REFERENCE.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/CLI_REFERENCE.md): lifecycle, schema, client, profiling, compatibility gates, Actions/mobile/test scaffolds, manager commands.

## Progressive learning path

Hopper layers so you don't learn systems mode first:

1. Framework mode: hopper::prelude, #[account], #[program], typed wrappers, PDA helpers, token modules, guard macros.
2. Structured state: keep #[account] and add bounded fields like String<'a, 32> or Vec<'a, Address, 10>. Hopper lowers them into fixed-body plus compact-tail. Use TailStr<'a> or TailBytes<'a> only when a protocol deliberately needs a final named field consuming remaining tail.
3. Systems mode: add hopper::systems, hopper::segment, hopper::receipt, hopper::policy, hopper::migration, hopper::interface for field leasing, audit trails, upgrades, and cross-program layout contracts.
4. Substrate mode: use hopper::substrate for direct Hopper Native tools like account views, hashes, PDA, input parsing, memory helpers, syscalls. The compute-budget probe needs the withdrawn SIMD-0049 syscall and is compiled for on-chain targets only under the `remaining-compute-units-syscall` feature.

## Access tiers

Normal handlers use ctx.accounts.* plus get() / get_mut() on typed wrappers. Reach for lower-level access only when a protocol explicitly needs systems-mode control:

1. segment_ref_typed / generated field accessors: default hot path for field-level borrow leasing.
2. get / get_mut on Account<'info, T>: validated whole-layout access.
3. segment_ref_const / dynamic segment_ref: advanced runtime-selected segment access.
4. `raw_ref` / `raw_mut`: unsafe typed escape hatch.
5. `as_mut_ptr`: full raw pointer escape for policy-controlled raw mode.

For variable-length data, use Quasar-style bounded fields directly in #[account]:

```rust
#[hopper::account(discriminator = 10, version = 1)]
pub struct Multisig<'a> {
  pub threshold: WireU64,
  pub label: String<'a, 32>,
  pub signers: Vec<'a, Address, 10>,
}
```

Hopper's typed overlays require alignment-1 Pod types. Use WireU64/WireI64/WireU128 for multi-byte scalar fields; native u64/i64/u128 are intentionally rejected for fixed typed overlay layouts. Bounded-field accounts like the one above also accept native scalars in the fixed head and lower them to the matching wire types.

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
| crates/hopper-builtins | Optional SBF memory-intrinsic overrides for runtime-length operations. |
| crates/hopper-memo | SPL Memo helpers. |
| crates/hopper-anchor | Anchor-compat interop surface. |
| crates/hopper-finance, -lending, -staking, -vesting, -distribute, -multisig | Domain crates: AMM math, lending health, staking rewards, vesting schedules, distribution splits, multisig thresholds. |
| crates/hopper-manager | Manifest-driven account inspection. |
| crates/hopper-sdk | Client-side SDK surface. |
| crates/hopper-svm | In-process host execution harness for tests. |
| crates/hopper-test | Test helpers and trace utilities (not published). |
| crates/hopper-topology | Hopper Loom: account-topology analysis and deterministic placement plans. |
| crates/grillo-manifest, crates/grillo-verifier | Grillo: mutation-manifest model and separately runnable offline byte-diff verifier. |
| tools/hopper-cli | hopper CLI: linting, schema export, inspect, profile. |
| examples | Example programs. |
| docs | Design notes, unsafe invariants, and the Effect ABI specs. |

The old split repos were folded back with subtree history preserved, then archived.

Companion repos:
- [hopper-bench](https://github.com/BluefootLabs/hopper-bench): benchmark harness and CU lab.

The in-process execution harness `hopper-svm` now lives in-tree at `crates/hopper-svm`.

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
- [examples/hopper-cicada](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-cicada): protected-execution flagship with exact-cell write authority, route/custody boundaries, and 25 host + 23 compiled lifecycle tests. It is production-shaped, not audited or deployed as a current release.
- [examples/hopper-counter](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-counter): minimal #[derive(Accounts)], Ctx<T>, ctx.accounts.* flow.
- [examples/hopper-vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-vault): SOL vault using named initialization, scoped borrows, checked helpers, and System transfer.
- [examples/hopper-escrow](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-escrow): state, `has_one`, and close lifecycle sketch. It does not execute SPL Token transfers.
- [examples/quasar-port-20-min](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/quasar-port-20-min): Quasar-style bounded dynamic port with Hopper guarantees.
- [examples/hopper-devnet-audit](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-devnet-audit): deployable devnet audit covering dynamic tails, contexts, segments, receipts, Token-2022 policy, field capabilities, substrate probes.
- [examples/hopper-argus-guard](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-argus-guard): Argus-style risk guard with checked exposure and authority-bound state.

Systems mode examples:
- [examples/hopper-proc-vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-proc-vault): generated/lowered account access for teams inspecting macro output.
- [examples/hopper-policy-vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-policy-vault): strict, sealed, raw, hybrid handlers side by side.
- [examples/hopper-showcase](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-showcase): broad feature tour.

Raw and benchmark examples:

- [examples/hopper-parity-vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-parity-vault): apples-to-apples benchmark target with intentionally low-level lamport mutation.
- [examples/hopper-token-2022-vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-token-2022-vault) and [examples/hopper-token-2022-ata](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-token-2022-ata): Token-2022 low-level validation and CPI examples.

For in-process tests, use the in-tree `crates/hopper-svm` crate as a dev-dependency.
For offline effect recomputation, use
[`grillo-verifier`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/crates/grillo-verifier): the current CLI uses the v0.1
evidence format to check `changed ⊆ acquired ⊆ authorized`, while the
experimental Effect ABI v0.2 library surface adds transition/CPI contracts and
checks supplied deployment identity for internal consistency. It does not
authenticate that identity or ledger provenance. Grillo has no on-chain
deployment cost.

## Benchmarks

The benchmark suite is maintained as a separate product repo:
[hopper-bench](https://github.com/BluefootLabs/hopper-bench)

The most recent archived comparison is the clean 2026-08-16 same-behavior
vault matrix. It pins the dated Hopper source below, Pinocchio 0.11.2, a Quasar `0.1.0-release`
branch snapshot, the pre-RC
Anchor v2 source snapshot, and Star Frame 0.30 to one program id, account state, seed
set, release profile, SBF toolchain, and Mollusk runner:

| Framework | Deposit CU | Withdraw CU | Binary bytes |
|---|---:|---:|---:|
| Hopper | 1,578 | 424 | 9,032 |
| Quasar 0.1 snapshot | 1,755 | 593 | 5,784 |
| Anchor v2 pre-RC snapshot | 1,785 | 615 | 6,432 |
| Pinocchio 0.11.2 | 3,697 | 2,542 | 7,512 |
| Star Frame 0.30 snapshot | 3,837 | 2,624 | 83,216 |

The strict run used clean Hopper
`8696640aad613b081c66e77f13ff679c6d4d1967` and benchmark
`af5bc95961a8a8b807a194a7d9fd1cd1249393c5`, required fresh artifacts,
used 8 samples, and passed all 30 rejection gates. Its evidence ZIP SHA-256 is
`c64af2460bcbfc0a9a3b8e5a7d8ecdbaa73ff34b7b5d20b0f17e89e44a84f747`.
The content-addressed summary is
[`audit/framework-matrix-2026-08-16.json`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/audit/framework-matrix-2026-08-16.json).

This is a measurement of one vault contract and the cited source pins, not a
measurement of later 0.3.0 source or a universal ranking. Quasar
has the smallest binary in the matrix, and Pinocchio is cheaper on the
separate missing-signature failure row. Historical primitive, four-way vault,
and router tables remain in [BENCHMARKS.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/BENCHMARKS.md) with their dates;
they are not release evidence for source changes made after the cited Hopper
commit. A clean rerun is required before publishing refreshed performance
claims for this release.

Hopper combines declarative account binding and generated clients with
byte-range write contracts, migration metadata, and an explicit raw substrate.
Re-run the strict matrix whenever a
measured framework source, dependency, toolchain, fixture, or runner pin
changes:

```powershell
cd ../hopper-bench
.\run-current-matrix.ps1 -HopperRoot ..\Hopper-Solana-Zero-copy-State-Framework -OutDir results\framework-vaults-current-YYYY-MM-DD
```

### Where Pinocchio Is Still The Right Choice

Use raw Pinocchio directly when a program wants a minimal manual substrate,
manual account validation, and no framework-level schema, lifecycle, or tooling
surface. Hopper is the framework-layer option for teams that want the same
low-level access model plus explicit safety and developer ergonomics.

## Safety posture

Hopper uses `unsafe` at the boundary where account bytes become typed views.
The framework keeps those boundaries small and documented, but this is still a
zero-copy framework and should be reviewed like one.

Hopper also maintains a competitor-bug-class regression suite. It turns
documented bug classes from other frameworks (CPI return-data UB,
self-close lamport imbalance, stale migration state, overstated
remaining-capacity, duplicate-account aliasing, and the two Anchor v2
alpha Slab classes, #4603 and #4616) into Hopper regression proofs.
Authoring that suite found and fixed a real Hopper bug (`safe_close` accepted
an aliased destination).

Independent external-audit scope and evidence preparation are in progress. No
independent reviewer is engaged and no independent audit report exists yet.

Verification lanes beyond the test matrix: Kani proofs over the raw-input
parser and tail codecs (`scripts/kani-*.sh`), and a Miri lane under Tree
Borrows over the aliasing core, the segment borrow ledger, write-policy
gate, native-boundary transmutes, and borrow registry
(`scripts/miri-core.sh`). The Miri lane caught and fixed two real
UB classes in test fixtures on its first run; that is what it is for.

See:

- `docs/UNSAFE_INVARIANTS.md`
- `crates/hopper-core/tests/unsafe_boundary_tests.rs`
- `crates/hopper-core/tests/overlay_equivalence_tests.rs`
- `crates/hopper-runtime/tests/competitor_bug_classes.rs` and
  `crates/hopper-core/tests/competitor_bug_classes.rs`
- [COMPARISON.md](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/COMPARISON.md) for which guarantees are structural

## Support

Hopper is open-source Solana infrastructure. Public-goods support and donations
can be sent to `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

Donation URI: <solana:F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT?label=solanadevdao.sol>

## License

Licensed under either of:

- MIT license (`LICENSE-MIT`)
- Apache License, Version 2.0 (`LICENSE-APACHE`)

at your option.
