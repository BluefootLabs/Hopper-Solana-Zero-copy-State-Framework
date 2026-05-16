# Getting Started with Hopper

Start with `examples/hopper-counter` when you want the five-minute path:
`use hopper::prelude::*`, `#[account]`, `#[derive(Accounts)]`, `#[program]`,
`Ctx<T>`, and `ctx.accounts.*`.

This guide walks through the same macro-first shape used by the compiled
`examples/hopper-vault` program. The advanced systems APIs are still available,
but they are not where a new Hopper program should begin.

## Prerequisites

- Rust stable
- Solana CLI with `cargo-build-sbf` for SBF builds and deploys
- A funded Solana keypair when deploying to a live cluster

## Install Hopper

For a new program, install the published CLI and scaffold from crates.io:

```bash
cargo install hopper-cli
hopper init my-vault --template minimal --yes
cd my-vault
```

The generated manifest imports the published `hopper-lang` package as the Rust
crate `hopper`:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.2.0", default-features = false, features = ["hopper-native-backend", "proc-macros"] }
```

The package is named `hopper-lang` on crates.io because the `hopper` package
name is already occupied by an unrelated crate. The library crate is still
`hopper`, so program code starts with:

```rust
use hopper::prelude::*;
```

When developing against a local framework checkout, use the CLI flag instead of
editing the generated file by hand:

```bash
hopper init my-vault --template minimal --local-path ../Hopper-Solana-Zero-copy-State-Framework --yes
```

## Step 1: Define State

Use `#[account]` on a `#[repr(C)]` struct. Hopper writes a 16-byte account
header, computes a layout fingerprint, and gives you checked zero-copy load
helpers. Multi-byte fields use Hopper's alignment-safe wire types.

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Vault {
    pub authority: Address,
    pub balance: WireU64,
    pub bump: u8,
}
```

Wire integers expose checked helpers so business logic stays direct without
forgetting overflow checks:

```rust
vault.balance.checked_add_assign(amount)?;
vault.balance.checked_sub_assign(amount)?;
```

## Step 2: Define Errors

```rust
hopper_error! {
    base = 6000;
    Unauthorized,
    InsufficientBalance,
    ZeroAmount,
}
```

Use generated errors with `hopper_require!`:

```rust
hopper_require!(amount > 0, ZeroAmount);
```

## Step 3: Define Accounts

`#[derive(Accounts)]` is the first-touch context API. Use `Account<'info, T>`
for existing state, `InitAccount<'info, T>` for accounts being created,
`Signer<'info>` for signers, and `Program<'info, System>` for the System
Program.

```rust
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init, payer = payer, space = Vault::INIT_SPACE)]
    pub vault: InitAccount<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
}
```

For raw accounts, use `UncheckedAccount<'info>`. For accounts owned by the
System Program, use `SystemAccount<'info>`.

## Step 4: Add Handlers

The CLI template includes the tiny runtime bridge through
`hopper::program_dispatch!(...)`. Most program authors work in the `#[program]`
module, where Hopper dispatches from the discriminator bytes and hands each
handler a typed `Ctx<T>`.

```rust
#[program]
mod vault_program {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Ctx<Initialize>) -> ProgramResult {
        ctx.init_vault()?;
        ctx.accounts.initialize()
    }

    #[instruction(1)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }

    #[instruction(2)]
    pub fn withdraw(ctx: Ctx<Withdraw>, amount: u64) -> ProgramResult {
        ctx.accounts.withdraw(amount)
    }
}

hopper::program_dispatch!(vault_program);
```

`ctx.bumps.field_name` is available for seed-derived accounts. Older Hopper
code that calls `ctx.bumps().field_name` still works.

## Step 5: Write Account Logic

Keep handlers thin and put behavior on the accounts struct. `get_mut()` borrows
the zero-copy state, while `as_account()` gives access to lamports and address
metadata.

```rust
impl<'info> Initialize<'info> {
    pub fn initialize(&self) -> ProgramResult {
        let mut vault = self.vault.get_mut_after_init()?;
        vault.set_inner(*self.payer.key(), 0, 0)
    }
}

impl<'info> Deposit<'info> {
    pub fn deposit(&self, amount: u64) -> ProgramResult {
        hopper_require!(amount > 0, ZeroAmount);

        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();

        authority.set_lamports(
            authority
                .lamports()
                .checked_sub(amount)
                .ok_or(ProgramError::InsufficientFunds)?,
        );
        vault_account.set_lamports(
            vault_account
                .lamports()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );

        let mut vault = self.vault.get_mut()?;
        vault.balance.checked_add_assign(amount)?;
        Ok(())
    }
}

impl<'info> Withdraw<'info> {
    pub fn withdraw(&self, amount: u64) -> ProgramResult {
        hopper_require!(amount > 0, ZeroAmount);

        let mut vault = self.vault.get_mut()?;
        if vault.balance.get() < amount {
            return Err(InsufficientBalance.into());
        }
        vault.balance.checked_sub_assign(amount)?;
        drop(vault);

        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();

        vault_account.set_lamports(
            vault_account
                .lamports()
                .checked_sub(amount)
                .ok_or(ProgramError::InsufficientFunds)?,
        );
        authority.set_lamports(
            authority
                .lamports()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );

        Ok(())
    }
}
```

## Build, Test, and Deploy

Inside a scaffolded Hopper program:

```bash
hopper build --host
hopper test
hopper build
```

`hopper build` defaults to SBF and delegates to `cargo build-sbf`. To deploy a
built program with the Solana CLI:

```bash
solana program deploy target/deploy/my_vault.so
```

Inside this framework repository, the corresponding host checks are:

```bash
cargo check -p hopper-counter --locked
cargo check -p hopper-vault --locked
cargo check -p hopper-escrow --locked
cargo run -p hopper-cli -- publish-check --source-only --full
```

## Inspect with the CLI

The published CLI binary is `hopper`:

```bash
hopper inspect <hex-data>
hopper explain <hex-data>
hopper compat <hex-old> <hex-new>
hopper plan <hex-old> <hex-new>
```

For manifest-backed workflows, use the manager commands:

```bash
hopper manager summary hopper.manifest.json
hopper manager layouts hopper.manifest.json
hopper manager decode hopper.manifest.json <hex-data>
```

See [CLI_REFERENCE.md](CLI_REFERENCE.md) for the complete command surface.

## The Pipeline

```text
1. Define     #[account] declares layout and schema metadata
2. Bind       #[derive(Accounts)] validates account order and constraints
3. Dispatch   #[program] routes instruction bytes to typed handlers
4. Execute    ctx.accounts.* methods mutate validated zero-copy state
5. Inspect    CLI decodes, explains, diffs, and plans migrations
```

## Next Steps

| Where to go | What you learn |
|---|---|
| [examples/hopper-counter](../examples/hopper-counter/src/lib.rs) | The smallest macro-first program |
| [examples/hopper-vault](../examples/hopper-vault/src/lib.rs) | Full SOL vault matching this guide |
| [examples/hopper-escrow](../examples/hopper-escrow/src/lib.rs) | Multi-instruction escrow shape |
| [examples/hopper-policy-vault](../examples/hopper-policy-vault/src/lib.rs) | Strict, sealed, and raw policy modes |
| [examples/hopper-token-2022-vault](../examples/hopper-token-2022-vault/src/lib.rs) | Token-2022 extension checks |
| [WRITING_HOPPER_PROGRAMS.md](WRITING_HOPPER_PROGRAMS.md) | Authoring patterns and program structure |
| [POLICY_GUARANTEES.md](POLICY_GUARANTEES.md) | Policy modes and safety guarantees |
| [UNSAFE_INVARIANTS.md](UNSAFE_INVARIANTS.md) | Audit ledger for unsafe boundaries |