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
- Behavior primitives are explicit today: `HopperBehavior` exposes `WRITES` and successful checks return `BehaviorChecked` markers (`crates/hopper-runtime/src/behavior.rs`). Callers must still incorporate those ranges into the installed policy and invoke the helper; the proposed `#[account(... behavior(...))]` codegen loop is not shipped. Quasar's `AccountBehavior` is side-effect-only hooks in the pinned comparison snapshot.
- Foreign lenses read other programs' accounts through a manifest with 4-way ABI-drift detection, owner, discriminator, wire fingerprint, schema-epoch range (`crates/hopper-runtime/src/foreign.rs::ForeignManifest`).
- Proof-carrying markers let downstream APIs require type-level evidence a check ran (`crates/hopper-runtime/src/proof.rs::AccountProof`).
- Token-2022 TLV constraints validate extension state without deserializing into owned structs.
- `hopper solana-check`, `publish-check`, and the SBF workflow keep deployable crate shape and direct-runtime assumptions honest.
- Actions, mobile, and security-test generators have a manifest-backed foundation for product scaffolding.

See [COMPARISON.md](../COMPARISON.md) for which capabilities are ordinary
feature work and which require an account-access-layer retrofit.

## Project maturity and soundness track record

Snapshot refreshed 2026-09-06 against public source and official docs, and
rechecked 2026-09-19. This is stated
factually because readers weighing the two frameworks need it, not as a knock
on Quasar's engineering, which is real.

- **Release status.** Quasar's default pin is `b0de7db` (2026-07-13) and its
  `0.1.0-release` pin is `0361701` (2026-07-26); no Quasar ref has a commit
  after `d981ac8` (2026-08-02), rechecked 2026-09-19. No public tag/release was
  found and its published framework crates remain 0.0.0. The release branch is
  substantially ahead: typed
  grow/shrink migrations, wire IDL/ABI hashing, expanded clients and CLI,
  QuasarSVM, Kani/Miri/fuzz lanes, and implemented CU/binary budget gates. It still describes the
  0.1 line as beta and unaudited, and had no public 0.1 tag/release at this
  snapshot.
  This Hopper tree is unpublished 0.3.0 development source; the registry
  release observed 2026-09-06 is 0.2.1. Hopper builds on
  stable Rust (pinned 1.96.0) and
  carries an internal line-by-line audit trail (`docs/UNSAFE_INVARIANTS.md`),
  but neither internal review nor documentation is a third-party audit.
- **Soundness history and current work.** The five 2026-07 issue classes below
  are retained as historical regression provenance, not asserted as the
  current open-issue count. Current 0.1 release-line work includes CPI
  return-data handling and optional mutable-account duplicate coverage.
  [#238](https://github.com/blueshift-gg/quasar/issues/238) and
  [#234](https://github.com/blueshift-gg/quasar/issues/234) (CPI return-data
  `assume_init` over uninitialized bytes, UB),
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
  `crates/hopper-core/tests/competitor_bug_classes.rs` (22 tests at
  2026-09-19, including the Anchor v2 classes #4603, #4616, #4886, #5043,
  #4906, and #4888).
- **The regression corpus bites both ways.** Authoring those tests found a real Hopper
  bug, `safe_close` previously accepted an aliased destination and silently
  burned the drained lamports, the exact #240 shape; which was fixed and
  pinned in the same pass. It did so again on 2026-09-19: Anchor v2's #4888
  realloc-below-minimum class applied to Hopper's `safe_realloc`, and was
  fixed (`safe_realloc_bounded` plus a `required_len()` floor in every
  generated realloc accessor) and pinned in the same commit. The regression
  corpus is designed to catch our own failures as well as peer-reported bug
  classes.
- **Verification depth.** A local enumeration of the pinned Quasar release
  branch found 183 Miri tests, including Tree-Borrows/strict-provenance lanes,
  and 87 Kani proof functions exercised by CI. These are test-list counts, not
  an external audit, but they are materially deeper than Hopper's current lane.
- **Benchmark culture.** Quasar's cross-framework benchmark work is currently
  an open draft ([#497](https://github.com/blueshift-gg/quasar/pull/497)), not
  a released result; unchanged at 2026-09-19, with no Quasar commits since
  2026-08-02. Hopper's clean 2026-08-16 five-way fixture pins Quasar's
  `0361701` 0.1 snapshot and reports Hopper/Quasar deposit at 1,578/1,755 CU,
  withdraw at 424/593 CU, and binaries at 9,032/5,784 bytes. All 30 parity
  gates passed from clean commits. That is fixture-specific evidence: Hopper
  is lower-CU on these two rows, while Quasar has the smaller binary. The
  Hopper binary in that archive was built with `crate-type = ["cdylib", "lib"]`,
  which kept LTO off; see the 2026-09-19 note in `BENCHMARKS.md`. See
  `BENCHMARKS.md` and
  [`audit/framework-matrix-2026-08-16.json`](../audit/framework-matrix-2026-08-16.json)
  for complete pins and provenance. The older four-way and router rows remain
  historical evidence, not current release-line rankings.

For the current source pins, corrected budget chronology, peer matrix, QEDGen
overlap, and Cicada/Grillo deployment facts, see the
[2026-09-06 competitive refresh](COMPETITIVE_REFRESH_2026-09-02.md) and the
[2026-09-19 refresh](COMPETITIVE_REFRESH_2026-09-19.md) that supersedes its
time-sensitive rows.

Use Quasar mental models to read Hopper programs. Use Hopper contracts when account bytes, upgrades, and long-lived protocol state need to be auditable.
