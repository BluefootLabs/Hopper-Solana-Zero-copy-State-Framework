# Coming from Pinocchio

Hopper's raw layer works the way Pinocchio does. Accounts are views into the
loader's input buffer, nothing is copied or deserialized, and a CPI goes
straight to the syscall. A Pinocchio program ports by changing its imports and
a handful of calls, and it runs at the same cost. From there you can take as
much of the framework as you want, in the same crate: typed accounts,
generated account checks, `#[program]` dispatch.

## What it costs

pina's framework-comparison fixtures, built with its release recipe and run
once per instruction in Mollusk, measured on `main` on 2026-09-30, ahead of
the 0.5 release (`scripts/bench-framework-comparison.py`):

| Program | Size | Compute units |
|---|---:|---:|
| Hello world, hand-written Pinocchio | 3,160 B | 111 |
| Hello world, Hopper raw | 1,456 B | 111 |
| PDA counter, hand-written Pinocchio (create, increment) | 6,512 B | 1,490, 1,721 |
| PDA counter, Hopper raw (create, increment) | 6,608 B | 1,514, 1,722 |
| PDA counter, Hopper `#[program]` (create, increment) | 8,744 B | 1,471, 325 |

The raw counter is the same program as Pinocchio's: the same 10-byte account,
the same `CreateAccount` CPI, the same `create_program_address` on every
increment. Most of the 24 CU it adds on create go to checks Pinocchio does not
make (see [What behaves differently](#what-behaves-differently)). The `#[program]`
counter checks the PDA from the bump stored in the account with one SHA-256
instead of re-deriving it, which is why its increment is 325 CU; it keeps a
16-byte header, so its account is 25 bytes.

## The port, side by side

The increment handler of the Pinocchio counter, condensed:

```rust
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::{ProgramError, ProgramResult};

pinocchio::program_entrypoint!(process_instruction);
pinocchio::no_allocator!();
pinocchio::nostd_panic_handler!();

fn increment(program_id: &Address, accounts: &mut [AccountView]) -> ProgramResult {
    let [authority, counter] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !counter.owned_by(program_id) {
        return Err(ProgramError::IncorrectProgramId);
    }
    let (bump, count) = {
        let data = counter.try_borrow()?;
        if data.len() != 10 || data[0] != 1 {
            return Err(ProgramError::InvalidAccountData);
        }
        (data[1], u64::from_le_bytes(data[2..10].try_into().unwrap()))
    };
    let derived = Address::create_program_address(
        &[b"counter", authority.address().as_ref(), &[bump]],
        program_id,
    )?;
    if counter.address() != &derived {
        return Err(ProgramError::InvalidSeeds);
    }
    let next = count.checked_add(1).ok_or(ProgramError::ArithmeticOverflow)?;
    counter.try_borrow_mut()?[2..10].copy_from_slice(&next.to_le_bytes());
    Ok(())
}
```

The same handler on Hopper's raw layer, with the account described as a
compact state so the length and discriminator checks come from the type:

```rust
use hopper::prelude::{AccountView, Address, ProgramError, ProgramResult, WireU64};

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction, 3);

/// `[disc = 1][bump][count]`, 10 bytes.
#[derive(Clone, Copy, Debug, Default)]
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Counter {
    #[bump]
    pub bump: u8,
    pub count: WireU64,
}

fn increment(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, counter] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !counter.owned_by(program_id) {
        return Err(ProgramError::IncorrectProgramId);
    }
    let mut state = counter.load_compact_mut::<Counter>()?;
    let derived = hopper::pda::create_program_address(
        &[b"counter", authority.address().as_array(), &[state.bump]],
        program_id,
    )?;
    if counter.address() != &derived {
        return Err(ProgramError::InvalidSeeds);
    }
    state.count.checked_add_assign(1)?;
    Ok(())
}
```

The whole program, create and increment, is
[`bench/framework-comparison/programs/counter/hopper-substrate`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/bench/framework-comparison/programs/counter/hopper-substrate/src/lib.rs),
next to the Pinocchio original it is measured against in
[`bench/framework-comparison/reference/counter`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/bench/framework-comparison/reference/counter/src/lib.rs).

## Name by name

| Pinocchio | Hopper |
|---|---|
| `pinocchio::program_entrypoint!(process)` | `hopper::program_entrypoint!(process)`, or `(process, N)` to size the account scratch for at most `N` accounts |
| `pinocchio::lazy_program_entrypoint!` | `hopper::lazy_entrypoint!` |
| `pinocchio::no_allocator!()`, `default_allocator!()` | `hopper::no_allocator!()`, `hopper::default_allocator!()` |
| `pinocchio::nostd_panic_handler!()` | `hopper::nostd_panic_handler!()`; the `panic-location` feature reports the file and line |
| `account::AccountView` | `hopper::prelude::AccountView` |
| `address::Address`, `address!` | `hopper::prelude::Address`, `hopper::address!` |
| `error::{ProgramError, ProgramResult}` | `hopper::prelude::{ProgramError, ProgramResult}` |
| `account.try_borrow()`, `try_borrow_mut()` | the same names |
| `Address::create_program_address(seeds, id)` | `hopper::pda::create_program_address(seeds, id)` |
| `Address::find_program_address(seeds, id)` | `hopper::pda::find_program_address(seeds, id)` |
| `instruction::cpi::{Seed, Signer}` | `hopper::cpi::{Seed, Signer}` |
| `cpi::invoke_signed(&ix, &accounts, &signers)` | `hopper::cpi::invoke_signed(&ix, &accounts, &signers)` |
| `sysvars::rent::Rent::get()?.try_minimum_balance(n)?` | `hopper::sysvar::Rent::get()?.minimum_balance(n)` |
| `sysvars::clock::Clock::get()?` | `hopper::sysvar::Clock::get()?` |
| `pinocchio_system::instructions::CreateAccount` | `hopper::system::CreateAccount` |
| `pinocchio_token::instructions::TransferChecked` | `hopper::token::TransferChecked` |
| `solana_program_log::log("...")`, `pinocchio_log::log!` | `hopper::msg!("...")` |

A handler receives `&[AccountView]` rather than `&mut [AccountView]`: a view
is a pointer into the input buffer, and every mutation goes through a method
that checks it, so there is nothing to gain from a unique slice.

## What behaves differently

Each of these is a check Pinocchio leaves out, and each costs a few compute
units. They are the difference in the table above.

- **Every resize is checked.** The entrypoint records each account's length
  on entry (one store per account), so `resize` can refuse growth past the
  10 KiB the runtime allows for the instruction. Pinocchio records it only
  with its `account-resize` feature.
- **Rent reads the exemption threshold.** Pinocchio 0.11 reads the rate and
  assumes a threshold of 1.0, which is what every public cluster stores.
  Hopper reads both and is exact under 1.0 and under 2.0, the value Mollusk
  and older test validators still use: there, Hopper funds the rent-exempt
  minimum and the one-field formula funds half of it.
- **Malformed input traps.** A duplicate-account marker that does not name
  an earlier account stops the program instead of producing an aliased view.
  The loader never writes one; the check costs two instructions on entry.
- **Mutable borrows consult the write policy.** Hopper can pin an
  instruction to the fields it declares (`#[hopper::context(strict_writes)]`).
  With no policy installed that is one load and a branch per mutable borrow.
  A program that never declares a policy can compile the check out with the
  `unguarded-raw-surfaces` feature.

What stays the same: CPI builders check that no account in the instruction is
borrowed (the same check Pinocchio's builders make) and leave signer and
writable privileges to the runtime, which refuses an escalation before the
callee runs.

## Where to go next

- **Typed state on the raw path.** `#[hopper::state(compact)]` gives an
  account a one-byte discriminator and a zero-copy body;
  `load_compact_mut::<T>()` checks the length and discriminator and hands
  back the body, and `init_compact_mut::<T>()` (on `main`, released in 0.5)
  stamps a new account and returns its zeroed body in one borrow.
- **A cheaper PDA check.** Once an account is known to be owned by your
  program and its layout has validated, `hopper::pda::verify_pda_address(
  &[seed, key, &[bump]], program_id, account.address())` checks its address
  with one SHA-256, about 150 CU, where the derivation syscall costs 1,500.
  It is the check the framework counter's update makes. For an address with
  no such binding, `verify_pda_address_checked` keeps the curve check.
- **The framework path.** `#[derive(Accounts)]` generates the signer,
  owner, seed, and layout checks from the struct, and `#[program]` generates
  the dispatch. It is where the counter's increment drops to 325 CU. See
  [the Hopper model](THE_HOPPER_MODEL.md) and [writing a program](../README.md).
- **Tokens.** [The Token-2022 guide](TOKEN_2022_GUIDE.md) covers transfers,
  mints, extensions, and confidential transfers.
