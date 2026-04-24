//! # Lazy-Dispatch Vault (R3 bench target)
//!
//! Eight-instruction dispatch vault built twice from the same source
//! tree:
//!
//! * `--features eager` → uses `fast_entrypoint!` and pre-parses all
//!   accounts before the dispatch match.
//! * `--features lazy` → uses `hopper_lazy_entrypoint!` and only
//!   materialises the accounts each variant actually touches.
//!
//! The two builds are byte-for-byte identical except for the
//! entrypoint glue and the handler signatures (`&[AccountView]` for
//! eager, `&mut LazyContext` for lazy). Mollusk-SVM runs both under
//! the same workload and records the CU delta.
//!
//! ## Why this exists
//!
//! Hopper's parity vault (see `examples/hopper-parity-vault`) exercises
//! the **eager** entrypoint because that is the fair-comparison target
//! against Quasar and Pinocchio. The lazy entrypoint is Hopper-only —
//! no competitor ships a lazy variant — so it does not belong in the
//! cross-framework bench. This crate exists so the *intra-framework*
//! CU difference between eager and lazy is measurable on a realistic
//! dispatch pattern: eight instructions, each touching a different
//! subset of the eight declared accounts. Most variants touch two or
//! three of the eight, which is where lazy parsing earns its keep.
//!
//! ## Instruction map
//!
//! | Disc | Name          | Accounts touched (of 8) |
//! |-----:|---------------|-------------------------|
//! | 0    | `ping`        | 0                       |
//! | 1    | `get_balance` | 1 (vault)               |
//! | 2    | `authorize`   | 2 (user, vault)         |
//! | 3    | `counter`     | 2 (user, vault)         |
//! | 4    | `deposit`     | 3 (user, vault, sys)    |
//! | 5    | `withdraw`    | 2 (user, vault)         |
//! | 6    | `sweep`       | 8 (all)                 |
//! | 7    | `flush`       | 8 (all)                 |
//!
//! A program that receives instruction `0` (ping) should parse *zero*
//! accounts under the lazy entrypoint and all eight under the eager
//! one. That is the largest win the bench can showcase. Instruction
//! `6` and `7` touch all eight accounts, so lazy and eager should
//! converge within noise for those.
//!
//! ## Running
//!
//! ```bash
//! cargo build-sbf -p lazy-dispatch-vault --features eager
//! mv target/deploy/lazy_dispatch_vault.so target/deploy/lazy_dispatch_eager.so
//! cargo build-sbf -p lazy-dispatch-vault --features lazy
//! mv target/deploy/lazy_dispatch_vault.so target/deploy/lazy_dispatch_lazy.so
//! # Then drive both through a Mollusk harness (see bench/lazy-dispatch-bench).
//! ```

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

#[cfg(not(any(feature = "eager", feature = "lazy")))]
compile_error!(
    "lazy-dispatch-vault requires exactly one of the `eager` or `lazy` features. \
     Build with `cargo build-sbf -p lazy-dispatch-vault --features eager` or \
     `--features lazy`."
);

#[cfg(all(feature = "eager", feature = "lazy"))]
compile_error!(
    "lazy-dispatch-vault requires exactly one of `eager` or `lazy`, not both."
);

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

// ---------------------------------------------------------------------------
// Shared instruction handlers (backend-agnostic).
//
// Each handler takes already-resolved &AccountView references. The only
// difference between the eager and lazy builds is how those references
// are produced: eager parses all 8 up front and slices, lazy only
// parses the ones the variant declares a need for.
// ---------------------------------------------------------------------------

fn handle_ping() -> ProgramResult {
    // Touches zero accounts. Pure compute-unit floor measurement.
    Ok(())
}

fn handle_get_balance(vault: &AccountView) -> ProgramResult {
    // Touches one account. Read-only lamport probe.
    let _ = vault.lamports();
    Ok(())
}

fn handle_authorize(user: &AccountView, vault: &AccountView) -> ProgramResult {
    if !user.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !vault.is_writable() {
        return Err(ProgramError::Immutable);
    }
    Ok(())
}

fn handle_counter(user: &AccountView, vault: &AccountView) -> ProgramResult {
    if !user.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut borrows = SegmentBorrowRegistry::new();
    let mut counter = vault.segment_mut::<WireU64>(&mut borrows, 0, 8)?;
    let next = (*counter)
        .get()
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    *counter = WireU64::new(next);
    Ok(())
}

fn handle_deposit(
    _user: &AccountView,
    _vault: &AccountView,
    _system_program: &AccountView,
) -> ProgramResult {
    // No CPI in the bench path — we only want to measure the cost of
    // resolving the three accounts, not the cost of the system transfer.
    // A real program would invoke Transfer here.
    Ok(())
}

fn handle_withdraw(user: &AccountView, vault: &AccountView) -> ProgramResult {
    if !user.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !vault.is_writable() {
        return Err(ProgramError::Immutable);
    }
    Ok(())
}

fn handle_sweep(accounts: &[&AccountView]) -> ProgramResult {
    // Touches all 8. Iterates every account; this is where the eager
    // entrypoint pays no worse than the lazy one.
    for acct in accounts {
        let _ = acct.lamports();
    }
    Ok(())
}

fn handle_flush(accounts: &[&AccountView]) -> ProgramResult {
    // Also 8. Slight variant to keep the inliner honest — sums the
    // lamports instead of just reading them.
    let mut total = 0u64;
    for acct in accounts {
        total = total.wrapping_add(acct.lamports());
    }
    let _ = total;
    Ok(())
}

// ---------------------------------------------------------------------------
// Eager variant: standard fast_entrypoint! + slice dispatch.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "solana", feature = "eager"))]
fast_entrypoint!(process_eager, 8);

#[cfg(feature = "eager")]
fn process_eager(
    _program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (disc, _rest) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match *disc {
        0 => handle_ping(),
        1 => {
            let [_, vault, ..] = accounts else {
                return Err(ProgramError::NotEnoughAccountKeys);
            };
            handle_get_balance(vault)
        }
        2 => {
            let [user, vault, ..] = accounts else {
                return Err(ProgramError::NotEnoughAccountKeys);
            };
            handle_authorize(user, vault)
        }
        3 => {
            let [user, vault, ..] = accounts else {
                return Err(ProgramError::NotEnoughAccountKeys);
            };
            handle_counter(user, vault)
        }
        4 => {
            let [user, vault, system_program, ..] = accounts else {
                return Err(ProgramError::NotEnoughAccountKeys);
            };
            handle_deposit(user, vault, system_program)
        }
        5 => {
            let [user, vault, ..] = accounts else {
                return Err(ProgramError::NotEnoughAccountKeys);
            };
            handle_withdraw(user, vault)
        }
        6 => {
            if accounts.len() < 8 {
                return Err(ProgramError::NotEnoughAccountKeys);
            }
            let refs: [&AccountView; 8] = [
                &accounts[0], &accounts[1], &accounts[2], &accounts[3],
                &accounts[4], &accounts[5], &accounts[6], &accounts[7],
            ];
            handle_sweep(&refs)
        }
        7 => {
            if accounts.len() < 8 {
                return Err(ProgramError::NotEnoughAccountKeys);
            }
            let refs: [&AccountView; 8] = [
                &accounts[0], &accounts[1], &accounts[2], &accounts[3],
                &accounts[4], &accounts[5], &accounts[6], &accounts[7],
            ];
            handle_flush(&refs)
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

// ---------------------------------------------------------------------------
// Lazy variant: hopper_lazy_entrypoint! + on-demand account parsing.
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "solana", feature = "lazy"))]
hopper_lazy_entrypoint!(process_lazy);

#[cfg(feature = "lazy")]
fn process_lazy(ctx: &mut LazyContext) -> ProgramResult {
    let (disc, _rest) = ctx
        .instruction_data()
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match *disc {
        0 => {
            // Zero accounts parsed. This is the headline CU win.
            handle_ping()
        }
        1 => {
            // Parse through index 1 (skip user, take vault).
            ctx.skip(1)?;
            let vault = ctx.next_account()?;
            handle_get_balance(&vault)
        }
        2 => {
            let user = ctx.next_account()?;
            let vault = ctx.next_account()?;
            handle_authorize(&user, &vault)
        }
        3 => {
            let user = ctx.next_account()?;
            let vault = ctx.next_account()?;
            handle_counter(&user, &vault)
        }
        4 => {
            let user = ctx.next_account()?;
            let vault = ctx.next_account()?;
            let system_program = ctx.next_account()?;
            handle_deposit(&user, &vault, &system_program)
        }
        5 => {
            let user = ctx.next_account()?;
            let vault = ctx.next_account()?;
            handle_withdraw(&user, &vault)
        }
        6 => {
            // Eight accounts; parse all of them.
            let a = ctx.next_account()?;
            let b = ctx.next_account()?;
            let c = ctx.next_account()?;
            let d = ctx.next_account()?;
            let e = ctx.next_account()?;
            let f = ctx.next_account()?;
            let g = ctx.next_account()?;
            let h = ctx.next_account()?;
            let refs: [&AccountView; 8] = [&a, &b, &c, &d, &e, &f, &g, &h];
            handle_sweep(&refs)
        }
        7 => {
            let a = ctx.next_account()?;
            let b = ctx.next_account()?;
            let c = ctx.next_account()?;
            let d = ctx.next_account()?;
            let e = ctx.next_account()?;
            let f = ctx.next_account()?;
            let g = ctx.next_account()?;
            let h = ctx.next_account()?;
            let refs: [&AccountView; 8] = [&a, &b, &c, &d, &e, &f, &g, &h];
            handle_flush(&refs)
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
