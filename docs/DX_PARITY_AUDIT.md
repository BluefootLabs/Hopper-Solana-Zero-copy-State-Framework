# DX Parity Audit

This is the source-of-truth checklist for keeping Hopper's public first-touch
surface aligned with the Anchor/Quasar mental model while preserving Hopper's
zero-copy runtime and systems-mode escape hatches.

## Public First-Touch Surface

Beginner docs, `hopper add`, and the minimal/Quasar init templates should lead
with this shape:

```rust
use hopper::prelude::*;

#[derive(Accounts)]
pub struct Ix<'info> {
    #[account(mut)]
    pub account: Account<'info, MyState>,
    pub signer: Signer<'info>,
}

#[program]
mod app {
    use super::*;

    #[instruction(0)]
    pub fn ix(ctx: Ctx<Ix>) -> ProgramResult {
        ctx.accounts.ix()
    }
}

hopper::program_dispatch!(app);
```

Raw account access belongs in explicit systems/raw documentation and advanced
templates only.

## Mapping

| Quasar / Anchor surface | Hopper surface | Status |
| --- | --- | --- |
| Program module handlers | `#[program]` + `Ctx<T>` | Shipped |
| Account validation structs | `#[derive(Accounts)]` | Shipped |
| Typed zero-copy accounts | `Account<'info, T>` / `InitAccount<'info, T>` | Shipped |
| Signer role wrappers | `Signer<'info>` | Shipped |
| New instruction scaffold | `hopper add instruction` with `ctx.accounts.*` injection | Shipped |
| Minimal project scaffold | `hopper init --template minimal` first-touch template | Shipped |
| Dynamic fields | `#[hopper::dynamic_account]` + bounded `#[tail(...)]` fields | Shipped |
| Quasar migration scaffold | `hopper init --template quasar-port` dynamic-account template | Shipped |
| Generic program interfaces | `InterfaceSpec` + `Interface<'info, I>` | Shipped |
| Generic interface accounts | `InterfaceAccountLayout` + `InterfaceAccount<'info, T>` | Shipped |
| Token/Token-2022 interfaces | `TokenProgramKind`, `InterfaceTokenAccount`, `InterfaceMint` | Shipped |
| Init field writers | generated `set_inner(...)` helpers | Shipped |
| Bump access | `ctx.bumps.field` | Shipped |
| Release gate | `hopper publish-check` | Shipped |

## Regression Guards

- `tests/dx_parity_docs.rs` scans public docs, CLI add scaffold code, and the
  first-touch examples for old beginner-surface terms.
- `tools/hopper-cli/src/cmd/add.rs` has instruction-template regression tests.
- `tools/hopper-cli/src/cmd/lifecycle.rs` has minimal and Quasar-port template
  regression tests.
- `tests/compile_fail/quasar_mut_account_reference.rs` verifies the friendly
  diagnostic for Quasar-style reference-wrapped account fields.

## Allowed Raw Surface

Systems/lowered examples may still use raw account views, manual entrypoints,
and direct byte access when the file is explicitly about advanced runtime
control. Those APIs remain part of Hopper's safety and performance story; they
should not be the first screen a new app author sees.