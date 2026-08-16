# Hopper vs Quasar

Quasar has an excellent first-touch story: write Anchor-shaped Solana programs, cast account bytes directly, and keep the program small. Hopper keeps that authoring shape while adding a stronger contract layer.

> Write like Quasar. Hopper verifies the bytes before it casts them.

## The Difference

Quasar optimizes for direct account access. Hopper optimizes for direct account access after the program proves the bytes match the declared contract.

That contract includes:

- owner, signer, writable, PDA, and `has_one` validation through `#[derive(Accounts)]`;
- discriminator, version, and layout fingerprint checks before typed account access;
- schema-fingerprinted bounded dynamic fields;
- final-only raw tails through `TailStr<'a>` and `TailBytes<'a>`;
- optional segment-level borrows, receipts, policies, migrations, and interface pins when a protocol needs them.

## Same First-Touch Shape

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

`profile = "tiny"` keeps dispatch compact: one-byte discriminators and no handler-level modifier instrumentation.

## Dynamic Fields

Quasar-style bounded fields stay inline in source:

```rust
#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
}
```

Hopper accepts this source shape and lowers fixed multi-byte scalars to wire
wrappers (`u64` -> `WireU64`) in the emitted layout so typed overlays stay
alignment-safe.

Hopper lowers that to fixed body plus `[u32 len][compact tail payload]`. The dynamic tail schema is included in the layout fingerprint, so changing a capacity or element type is an ABI change that tools can detect.

For deliberate remaining-bytes semantics, Hopper uses named final tails:

```rust
#[hopper::account(discriminator = 21, version = 1)]
pub struct Note<'a> {
    pub authority: Address,
    pub label: String<'a, 32>,
    pub reviewers: Vec<'a, Address, 4>,
    pub body: TailStr<'a>,
}
```

`TailStr<'a>` and `TailBytes<'a>` must be final. They are fingerprinted as `tail_str` / `tail_bytes`, and the outer Hopper tail length still bounds the account region.

## Where Hopper Adds More

- Segment leases let systems-mode code borrow disjoint byte ranges instead of whole accounts (`crates/hopper-runtime/src/segment_borrow.rs`).
- Instruction touch maps enumerate the exact `(account, offset, size, read/write)` footprint an instruction touched (`Context::for_each_touch`, `touch-map` feature). The documented touch-map-enabled smoke case measured +52 CU; disabled programs pay none of that feature cost.
- Field-level write policies: `#[hopper::context(strict_writes)]` enforces declared mutable byte ranges at borrow acquisition (`crates/hopper-runtime/src/write_policy.rs`).
- Behaviors are accountable: `HopperBehavior` plugins contribute their `WRITES` to the write policy and return `BehaviorChecked` proof tokens (`crates/hopper-runtime/src/behavior.rs`). Quasar's `AccountBehavior` is side-effect-only hooks.
- Foreign lenses read other programs' accounts through a manifest with 4-way ABI-drift detection — owner, discriminator, wire fingerprint, schema-epoch range (`crates/hopper-runtime/src/foreign.rs::ForeignManifest`).
- Proof-carrying markers let downstream APIs require type-level evidence a check ran (`crates/hopper-runtime/src/proof.rs::AccountProof`).
- Token-2022 TLV constraints validate extension state without deserializing into owned structs.
- `hopper solana-check`, `publish-check`, and the SBF workflow keep deployable crate shape and direct-runtime assumptions honest.
- Actions, mobile, and security-test generators have a manifest-backed foundation for product scaffolding.

See [COMPARISON.md](../COMPARISON.md) for which of these a Pinocchio-based framework could copy and which it structurally cannot.

## Project maturity and soundness track record

Snapshot refreshed 2026-08-15 against public source and official docs. This is stated
factually because readers weighing the two frameworks need it, not as a knock
on Quasar's engineering, which is real.

- **Release status.** Quasar's default branch and crates.io package remain
  0.0.0. Its active `0.1.0-release` branch is substantially ahead: typed
  grow/shrink migrations, wire IDL/ABI hashing, expanded clients and CLI,
  QuasarSVM, Kani/Miri/fuzz lanes, and CU budget work. It still describes the
  0.1 line as beta and unaudited, and had no public 0.1 tag/release at this
  snapshot.
  Hopper's public crates.io release is 0.2.1; this source workspace is the
  unpublished 0.3.0 line. Hopper builds on stable Rust (pinned 1.96.0) and
  carries an internal line-by-line audit trail (`docs/UNSAFE_INVARIANTS.md`),
  but neither internal review nor documentation is a third-party audit.
- **Soundness history and current work.** The five 2026-07 issue classes below
  are retained as historical regression provenance, not asserted as the
  current open-issue count. Current 0.1 release-line work includes CPI
  return-data handling and optional mutable-account duplicate coverage.
  [#238](https://github.com/blueshift-gg/quasar/issues/238) and
  [#234](https://github.com/blueshift-gg/quasar/issues/234) (CPI return-data
  `assume_init` over uninitialized bytes — UB),
  [#240](https://github.com/blueshift-gg/quasar/issues/240) (account
  self-close imbalance),
  [#239](https://github.com/blueshift-gg/quasar/issues/239) (migration leaves
  stale state), and
  [#242](https://github.com/blueshift-gg/quasar/issues/242)
  (`Remaining<T,N>` capacity overstated), plus a raw-handler
  duplicate-account aliasing footgun.
- **Hopper's posture on the same five classes.** Each class is structurally
  guarded and regression-pinned: `get_return_data` is
  `MaybeUninit`-prefix-only on both paths (sound where #238 is UB), the
  borrow registry rejects duplicate-account aliasing, migration edges zero
  grown regions and never advance the epoch on failure, remaining-account
  capacity is reported exactly, and `safe_close` rejects aliased
  destinations. The pins live in
  `crates/hopper-runtime/tests/competitor_bug_classes.rs` and
  `crates/hopper-core/tests/competitor_bug_classes.rs` (18 tests, including
  the Anchor v2 Slab classes #4603/#4616).
- **The suite bites both ways.** Authoring those tests found a real Hopper
  bug — `safe_close` previously accepted an aliased destination and silently
  burned the drained lamports, the exact #240 shape — which was fixed and
  pinned in the same pass. The framework audits itself.
- **Benchmark culture.** Quasar's cross-framework benchmark work is currently
  an open draft ([#497](https://github.com/blueshift-gg/quasar/pull/497)), not
  a released result. Hopper's older pinned matrix remains a dated measurement,
  not evidence about Quasar's current release branch.
  Hopper's older `hopper-bench` matrix (vault four-way re-measured
  2026-07-09, router three-way 2026-07-07) includes Quasar and measured Hopper
  lower on the Quasar-implemented vault rows and the 1–3-hop router rows. It
  is historical fixture evidence, not a current release-line ranking. Do not
  repeat its deltas until the clean, same-behavior five-way archive is
  committed. See `BENCHMARKS.md` for the dated rows and provenance caveats.

For the pinned release-branch audit, peer matrix, and Cicada validation, see
[the 2026-08-15 zero-copy framework audit](ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md).

Use Quasar mental models to read Hopper programs. Use Hopper contracts when account bytes, upgrades, and long-lived protocol state need to be auditable.
