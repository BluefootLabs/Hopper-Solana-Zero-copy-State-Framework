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
- Instruction touch maps enumerate the exact `(account, offset, size, read/write)` footprint an instruction touched, at measured 0 CU (`Context::for_each_touch`, `touch-map` feature).
- Field-level write policies: `#[hopper::context(strict_writes)]` enforces declared mutable byte ranges at borrow acquisition (`crates/hopper-runtime/src/write_policy.rs`).
- Behaviors are accountable: `HopperBehavior` plugins contribute their `WRITES` to the write policy and return `BehaviorChecked` proof tokens (`crates/hopper-runtime/src/behavior.rs`). Quasar's `AccountBehavior` is side-effect-only hooks.
- Foreign lenses read other programs' accounts through a manifest with 4-way ABI-drift detection — owner, discriminator, wire fingerprint, schema-epoch range (`crates/hopper-runtime/src/foreign.rs::ForeignManifest`).
- Proof-carrying markers let downstream APIs require type-level evidence a check ran (`crates/hopper-runtime/src/proof.rs::AccountProof`).
- Token-2022 TLV constraints validate extension state without deserializing into owned structs.
- `hopper solana-check`, `publish-check`, and the SBF workflow keep deployable crate shape and direct-runtime assumptions honest.
- Actions, mobile, and security-test generators have a manifest-backed foundation for product scaffolding.

See [THE_MOAT.md](THE_MOAT.md) for which of these a Pinocchio-based framework could copy and which it structurally cannot.

## Project maturity and soundness track record

Facts verified 2026-07-07 against public trackers and registries (sources in
`docs/audit/GAP_CLOSURE_AND_INNOVATION_2026.md`, section 2). This is stated
factually because readers weighing the two frameworks need it, not as a knock
on Quasar's engineering, which is real.

- **Release status.** Quasar is v0.0.0 — no tags, no releases, not published
  on crates.io — and describes itself as "Beta … not audited". It builds only
  on nightly Rust with a bespoke toolchain. Hopper is published
  (hopper-lang 0.2.1 on crates.io), builds on stable Rust (pinned 1.96.0),
  and carries a line-by-line audit trail (`AUDIT.md`,
  `docs/UNSAFE_INVARIANTS.md`).
- **Open soundness issues.** Quasar's tracker carries five open
  unsoundness/correctness issues as of 2026-07:
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
  `crates/hopper-core/tests/competitor_bug_classes.rs` (13 tests).
- **The suite bites both ways.** Authoring those tests found a real Hopper
  bug — `safe_close` previously accepted an aliased destination and silently
  burned the drained lamports, the exact #240 shape — which was fixed and
  pinned in the same pass. The framework audits itself.
- **Benchmark culture.** Quasar publishes no comparative CU benchmark.
  Hopper's pinned, provenance-checked `hopper-bench` matrix (vault four-way
  and router three-way, 2026-07-07) is currently the only published
  cross-framework table that includes Quasar; in it, Hopper wins both rows
  Quasar's upstream vault implements (deposit −106 CU, withdraw −150 CU) and
  beats Quasar on every 1–3-hop router row (−18/−20/−21 CU) with a smaller
  binary. See
  `BENCHMARKS.md`.

Use Quasar mental models to read Hopper programs. Use Hopper contracts when account bytes, upgrades, and long-lived protocol state need to be auditable.
