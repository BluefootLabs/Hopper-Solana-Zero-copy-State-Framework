# Hopper vs Anchor vs Quasar vs Pinocchio

Hopper's positioning is intentionally narrow: Anchor/Quasar feel at the first touch, Pinocchio-class control when you need it, and Hopper-only state guarantees underneath.

## Summary

| Area | Anchor | Quasar | Pinocchio | Hopper |
| --- | --- | --- | --- | --- |
| First-touch account ergonomics | Excellent | Excellent | Minimal | `#[account]`, `#[derive(Accounts)]`, `Ctx<T>`, `ctx.accounts.*` |
| Runtime footprint | Higher-level framework | no_std / no_alloc | no_std / no_alloc | no_std / no_alloc framework crates |
| Zero-copy state | AccountLoader path | Primary path | Raw pointer path | Primary path with layout headers |
| Dynamic fields | Borsh account data | Inline bounded fields | Author-owned | Fixed body plus bounded compact tails |
| Token-2022 checks | Rich but deserializing | Base-layout oriented | Author-owned | Declarative zero-copy extension checks |
| Layout compatibility | IDL convention | Local type contract | Author-owned | Layout fingerprints and schema epochs |
| Borrow safety | Account-level | Account-level | Author-owned | Segment leases plus whole-layout wrappers |
| Raw escape hatch | Limited | Focused | Full | Tiered, explicit, policy-controlled |
| Benchmark posture | Mature ecosystem | Publicly fast | Fast substrate | Same-provenance proof required before direct claims |

## Anchor

Anchor remains the broadest Solana framework ecosystem. Its strength is mature onboarding, IDL tooling, client generation, and social proof. Hopper should not pretend every Anchor team should migrate.

Choose Hopper over Anchor when the account format is the product: fixed layouts, predictable account bytes, low allocation pressure, Token-2022 screening, and audit-visible state compatibility.

## Quasar

Quasar's best idea is the developer-facing shape: account struct, accounts struct, handler, `ctx.accounts.*`. Hopper now matches that shape while keeping Hopper's state-contract layer below it.

The important mapping is direct:

```rust
#[derive(Accounts)]
pub struct Increment<'info> {
    #[account(mut, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

#[instruction(0)]
pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
    let mut counter = ctx.accounts.counter.get_mut()?;
    counter.value.checked_add_assign(1)?;
    Ok(())
}
```

Dynamic fields map to `#[hopper::dynamic_account]`. `Address` / `Pubkey` vectors use borrowed views; other `TailElement` vectors use `HopperVec<T, N>` with generated editor helpers.

## Pinocchio

Pinocchio is a low-level substrate. It gives authors direct control and minimal overhead, but the author owns most invariants. Hopper keeps that control available through explicit systems/raw tiers while making framework-mode safety the default.

Use `hopper::systems::*` or raw access only when a protocol has a clear reason: field-level borrowing, custom CPI framing, carefully audited pointer access, or migration machinery.

## Hopper

Hopper's differentiated promise is:

> Anchor/Quasar feel. Hopper guarantees.

Those guarantees are concrete:

- Layout IDs and schema epochs make account compatibility explicit.
- Segment leases let protocols borrow disjoint fields instead of whole accounts.
- Dynamic tails keep hot fixed fields zero-copy while bounding variable metadata.
- Token-2022 extension checks stay zero-copy and declarative.
- Publish checks and client generators verify layout IDs instead of trusting comments.
- Raw access exists, but it is named, tiered, and policy-visible.

## Benchmark Language

Use cautious benchmark wording in release-facing material:

> Hopper framework mode is Quasar-class fast; direct Pinocchio claims wait for same-provenance `hopper-bench` results.

Do not mix old Pinocchio-style numbers with Hopper-vs-Quasar release claims. Run the sibling [hopper-bench](https://github.com/BluefootLabs/hopper-bench) harness before publishing performance tables.

## Migration Priority

1. Start with [FIRST_FIVE_MINUTES.md](FIRST_FIVE_MINUTES.md).
2. Port account contexts to `#[derive(Accounts)]` and `ctx.accounts.*`.
3. Replace `load()` / `load_mut()` naming with Hopper's `get()` / `get_mut()` wrappers.
4. Move bounded dynamic fields to `#[hopper::dynamic_account]`.
5. Add systems features only after the first-touch program is working.
