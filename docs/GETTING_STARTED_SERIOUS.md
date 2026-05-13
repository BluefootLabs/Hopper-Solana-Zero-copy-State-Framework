# Getting Started with Hopper

Start with `examples/hopper-counter` when you want the five-minute framework
path: `#[account]`, `#[accounts]`, `#[program]`, one typed handler, done.

This guide then walks through a fuller Hopper Solana program: a SOL vault with
typed state, explicit initialization, phased validation, controlled mutation,
and CLI inspection. The snippets mirror the compiled `examples/hopper-vault`
program, so the guide tracks code that is kept in CI instead of a standalone
sketch.

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

The generated manifest uses the published framework package while importing it
as the Rust crate `hopper`:

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.1.0", default-features = false, features = ["hopper-native-backend", "proc-macros"] }
```

The package is named `hopper-lang` on crates.io because the `hopper`
package name is already occupied by an unrelated crate. The library crate name
is still `hopper`, so Rust code uses:

```rust
use hopper::prelude::*;
```

When developing against a local framework checkout, use the CLI flag instead of
editing the generated file by hand:

```bash
hopper init my-vault --template minimal --local-path ../Hopper-Solana-Zero-copy-State-Framework --yes
```

## Step 1: Define Account State

The framework-first account spelling is `#[account]`, shown in
`examples/hopper-counter`. The vault below uses Hopper's no-proc-macro layout
path because it demonstrates lower-level initialization and phased execution:

```rust
#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

hopper_layout! {
    /// A simple SOL vault account.
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}
```

`hopper_layout!` generates a `#[repr(C)]` layout, a 16-byte Hopper header,
`Vault::LEN`, discriminator/version constants, a deterministic layout ID, and
validated load helpers such as `Vault::load` and `Vault::load_mut`.

Wire integers are explicit by design. Convert to native values, compute, then
write the wire value back:

```rust
let next = vault.balance.get()
    .checked_add(amount)
    .ok_or(ProgramError::ArithmeticOverflow)?;
vault.balance = WireU64::new(next);
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

## Step 3: Add Entrypoint and Dispatch

```rust
#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init,
        1 => process_deposit,
        2 => process_withdraw,
    }
}
```

`hopper_dispatch!` reads the first instruction byte as the tag and passes the
remaining bytes to the selected handler.

## Step 4: Initialize the Vault

```rust
fn process_init(program_id: &Address, accounts: &[AccountView], _data: &[u8]) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let vault_account = &accounts[1];
    let system_program = &accounts[2];

    payer.check_signer()?.check_writable()?;
    vault_account.check_writable()?;

    hopper_init!(payer, vault_account, system_program, program_id, Vault)?;

    let mut vault = Vault::load_mut(vault_account, program_id)?;
    let vault = vault.get_mut();
    vault.authority = TypedAddress::from_account(payer);
    vault.balance = WireU64::new(0);

    Ok(())
}
```

`hopper_init!` creates the account when it has zero lamports. If the account is
already pre-funded but has no data, it transfers only any missing rent lamports
and uses Allocate + Assign. Either way it allocates `Vault::LEN`, zeroes the
data, and writes the Hopper header.
After initialization, `Vault::load_mut` gives a validated mutable layout view.

## Step 5: Parse Instruction Arguments

Use `InstructionArgs` and `ValidateArgs` when a handler has typed arguments:

```rust
struct DepositArgs {
    amount: u64,
}

impl<'a> InstructionArgs<'a> for DepositArgs {
    fn parse(data: &'a [u8]) -> Result<Self, ProgramError> {
        if data.len() < 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(Self {
            amount: u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]),
        })
    }
}

impl ValidateArgs for DepositArgs {
    fn validate(&self) -> Result<(), ProgramError> {
        hopper_require!(self.amount > 0, ZeroAmount);
        Ok(())
    }
}

struct DepositAccounts<'a> {
    depositor: &'a AccountView,
    vault: &'a AccountView,
}
```

## Step 6: Deposit with Phased Execution

`PhasedFrame` enforces the Resolve -> Validate -> Execute ordering in the type
system. The deposit handler resolves account positions first, validates access,
then mutates lamports and account state:

```rust
fn process_deposit(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let args = DepositArgs::parse(data)?;
    args.validate()?;

    PhasedFrame::new(program_id, accounts, data)?
        .resolve(2, |accts, _pid| {
            Ok(DepositAccounts {
                depositor: &accts[0],
                vault: &accts[1],
            })
        })?
        .validate_with_args(&args, |ctx, pid, _args| {
            ctx.depositor.check_signer()?.check_writable()?;
            ctx.vault.check_owned_by(pid)?.check_writable()?;
            Ok(())
        })?
        .execute_with_args(&args, |ctx, args| {
            let mut vault = Vault::load_mut(ctx.resolved().vault, ctx.program_id())?;

            let dep_lamports = ctx.resolved().depositor.lamports();
            ctx.resolved().depositor.set_lamports(
                dep_lamports
                    .checked_sub(args.amount)
                    .ok_or(ProgramError::InsufficientFunds)?,
            );
            let vault_lamports = ctx.resolved().vault.lamports();
            ctx.resolved().vault.set_lamports(
                vault_lamports
                    .checked_add(args.amount)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            );

            let v = vault.get_mut();
            let new_balance = v
                .balance
                .get()
                .checked_add(args.amount)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            v.balance = WireU64::new(new_balance);

            Ok(())
        })
}
```

## Step 7: Withdraw with Authority Checks

The withdraw path uses the same argument and account shape, but validates the
stored authority before moving lamports out of the vault:

```rust
fn process_withdraw(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let args = DepositArgs::parse(data)?;
    args.validate()?;

    PhasedFrame::new(program_id, accounts, data)?
        .resolve(2, |accts, _pid| {
            Ok(DepositAccounts {
                depositor: &accts[0],
                vault: &accts[1],
            })
        })?
        .validate_with_args(&args, |ctx, pid, _args| {
            ctx.depositor.check_signer()?;
            ctx.vault.check_owned_by(pid)?.check_writable()?;
            Ok(())
        })?
        .execute_with_args(&args, |ctx, args| {
            let mut vault = Vault::load_mut(ctx.resolved().vault, ctx.program_id())?;
            let v = vault.get_mut();

            v.authority.require_eq_account(ctx.resolved().depositor)?;

            let balance = v.balance.get();
            if balance < args.amount {
                return Err(InsufficientBalance.into());
            }
            v.balance = WireU64::new(balance - args.amount);

            let vault_lamports = ctx.resolved().vault.lamports();
            ctx.resolved().vault.set_lamports(
                vault_lamports
                    .checked_sub(args.amount)
                    .ok_or(ProgramError::InsufficientFunds)?,
            );
            let auth_lamports = ctx.resolved().depositor.lamports();
            ctx.resolved().depositor.set_lamports(
                auth_lamports
                    .checked_add(args.amount)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            );

            Ok(())
        })
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
cargo test -p hopper-vault
cargo test -p hopper-policy-vault
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

## The Full Pipeline

```text
1. Define     hopper_layout! or #[hopper::state] declares state
2. Resolve    PhasedFrame::resolve() binds account positions
3. Validate   validate_with_args() checks signatures, ownership, and args
4. Execute    execute_with_args() mutates state after validation
5. Record     StateReceipt can capture before/after evidence
6. Verify     Layout IDs and invariants guard compatibility
7. Inspect    CLI decodes, explains, diffs, and plans migrations
```

## Next Steps

| Where to go | What you learn |
|---|---|
| [`examples/hopper-vault`](../examples/hopper-vault/src/lib.rs) | Full SOL vault matching this guide |
| [`examples/hopper-policy-vault`](../examples/hopper-policy-vault/src/lib.rs) | Strict, sealed, and raw policy modes |
| [`examples/hopper-token-2022-vault`](../examples/hopper-token-2022-vault/src/lib.rs) | Token-2022 extension checks |
| [`examples/hopper-treasury`](../examples/hopper-treasury/src/lib.rs) | Multi-segment treasury state |
| [`examples/hopper-migration`](../examples/hopper-migration/src/lib.rs) | V1 to V2 layout evolution |
| [`examples/cross-program-read`](../examples/cross-program-read/) | Cross-program layout reads by fingerprint |
| [WRITING_HOPPER_PROGRAMS.md](WRITING_HOPPER_PROGRAMS.md) | Authoring patterns and program structure |
| [POLICY_GUARANTEES.md](POLICY_GUARANTEES.md) | Policy modes and safety guarantees |
| [UNSAFE_INVARIANTS.md](UNSAFE_INVARIANTS.md) | Audit ledger for unsafe boundaries |