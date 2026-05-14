# First Five Minutes

Start in framework mode. Import the prelude, declare account bytes, derive account validation, and put handler logic behind `ctx.accounts.*`.

## 1. Counter

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
        let mut counter = ctx.accounts.counter.get_mut()?;
        counter.value.checked_add_assign(1)?;
        Ok(())
    }
}
```

This is the default mental model: validated accounts enter through `#[derive(Accounts)]`, then the handler mutates typed zero-copy state through `ctx.accounts`.

## 2. Vault

For larger instructions, keep the handler tiny and put business rules on the accounts struct:

```rust
#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
    pub authority: Signer<'info>,
}

impl<'info> Deposit<'info> {
    pub fn deposit(&self, amount: u64) -> ProgramResult {
        let mut vault = self.vault.get_mut()?;
        vault.balance.checked_add_assign(amount)?;
        Ok(())
    }
}

#[program]
mod vault_program {
    use super::*;

    #[instruction(1)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }
}
```

See [examples/hopper-vault/src/lib.rs](../examples/hopper-vault/src/lib.rs) for the complete SOL-vault flow.

## 3. Dynamic Multisig

Use `#[hopper::dynamic_account]` when a fixed zero-copy body needs bounded dynamic metadata.

```rust
use hopper::prelude::*;

#[hopper::dynamic_account(discriminator = 7, version = 1)]
pub struct Multisig {
    pub threshold: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,

    #[tail(vec<u16, 8>)]
    pub weights: Vec<u16>,
}
```

`Address` and `Pubkey` vectors keep the borrowed zero-copy view path. Other `TailElement` vectors use `HopperVec<T, N>` through the same compact tail codec and generated editor helpers.

```rust
let view = Multisig::tail_view(data)?;
let signers: &[Address] = view.signers()?;
let weights: HopperVec<u16, 8> = view.weights()?;
```

## 4. Token Transfer

For token programs, keep account validation and CPI construction explicit. The prelude exposes the polymorphic Token / Token-2022 interface helpers:

```rust
let source_kind = TokenProgramKind::for_account(source.as_account())?;
let mint_kind = TokenProgramKind::for_account(mint.as_account())?;
require_eq!(source_kind, mint_kind);

interface_transfer_checked(
    source.as_account(),
    mint.as_account(),
    destination.as_account(),
    authority.as_account(),
    amount,
    decimals,
)?;
```

Use the Token-2022 extension constraints when a mint must carry transfer hooks, close authority, non-transferable semantics, or metadata pointers. See [docs/TOKEN_2022_GUIDE.md](TOKEN_2022_GUIDE.md).

## 5. Raw Escape Hatch

Most programs stay in framework mode. When a protocol needs lower-level control, move deliberately down the access tiers:

```rust
let mut vault = ctx.accounts.vault.get_mut()?;

// Systems-mode code can opt into segment leasing when disjoint field borrows
// matter more than whole-layout ergonomics.
let mut balance = ctx.vault_balance_mut()?;
```

Reach for `hopper::systems::*` only when you need layout fingerprints, segment leases, receipts, migrations, or raw policy-controlled access. The first-touch path remains `ctx.accounts.*`.
