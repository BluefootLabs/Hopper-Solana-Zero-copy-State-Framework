# Hopper vs Anchor vs Quasar vs Pinocchio

Hopper's positioning is intentionally narrow: **Anchor/Quasar-class DX,
Hopper-grade safety/state contracts, Pinocchio-class raw control.**

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
| Benchmark posture | Mature ecosystem | Publicly fast | Fast substrate | Same-provenance vault snapshot, scoped claims |

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

### Interfaces

Quasar's generic `Interface<T>` / `InterfaceAccount<T>` model accepts accounts
owned by a declared set of programs and dispatches layout reads by owner. Hopper
matches that with `InterfaceSpec`, `Interface<'info, I>`, and
`InterfaceAccount<'info, T>`.

`Interface<'info, I>` validates an executable program account against `I::IDS`.
`InterfaceAccount<'info, T>` validates account owner against
`T::Interface::IDS`, then reads the remote Hopper layout with the
cross-program layout loader. That keeps the generic owner-set path explicit,
zero-copy, and layout-fingerprinted. For SPL Token and Token-2022, Hopper also
ships the specialized `TokenProgramKind`, `InterfaceTokenAccount`,
`InterfaceMint`, and `interface_transfer_checked` helpers because those account
bytes use SPL base layouts rather than Hopper headers.

## Pinocchio

Pinocchio is a low-level substrate. It gives authors direct control and minimal overhead, but the author owns most invariants. Hopper keeps that control available through explicit systems/raw tiers while making framework-mode safety the default.

Use `hopper::systems::*` or raw access only when a protocol has a clear reason: field-level borrowing, custom CPI framing, carefully audited pointer access, or migration machinery.

## Hopper

Hopper's differentiated promise is:

> Anchor/Quasar-class DX, Hopper-grade safety/state contracts, Pinocchio-class raw control.

Those guarantees are concrete:

- Layout IDs and schema epochs make account compatibility explicit.
- Segment leases let protocols borrow disjoint fields instead of whole accounts.
- Dynamic tails keep hot fixed fields zero-copy while bounding variable metadata.
- Token-2022 extension checks stay zero-copy and declarative.
- Publish checks and client generators verify layout IDs instead of trusting comments.
- Raw access exists, but it is named, tiered, and policy-visible.

## Benchmark Language

Use cautious benchmark wording in release-facing material. Publish only tables
produced by one `hopper-bench` run that uses the same lockfile, SBF toolchain,
Mollusk version, seed set, feature flags, release profile, and command line for
every included framework.

The current same-provenance vault snapshot is:

| Scenario | Hopper | Anza Pinocchio | Quasar |
|---|---:|---:|---:|
| Authorize | **430 CU** | 2512 CU | n/a |
| Auth-fail (missing sig) | 72 CU | **41 CU** | n/a |
| Counter (segment-safe) | **462 CU** | 2539 CU | n/a |
| Deposit | **1668 CU** | 3856 CU | 1767 CU |
| Withdraw | **453 CU** | 2548 CU | 603 CU |
| Binary size | 6.59 KiB | 7.73 KiB | **6.27 KiB** |

`n/a` means Quasar's upstream vault example does not implement that instruction.
Do not mix old Pinocchio-style numbers with this table. See
[BENCHMARKS.md](../BENCHMARKS.md) for the result provenance and reproduction
command.

## Migration Priority

1. Start with [FIRST_FIVE_MINUTES.md](FIRST_FIVE_MINUTES.md).
2. Port account contexts to `#[derive(Accounts)]` and `ctx.accounts.*`.
3. Replace `load()` / `load_mut()` naming with Hopper's `get()` / `get_mut()` wrappers.
4. Move bounded dynamic fields to `#[hopper::dynamic_account]`.
5. Add systems features only after the first-touch program is working.
