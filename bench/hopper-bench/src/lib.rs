//! # Hopper CU Benchmark Program
//!
//! On-chain program that measures compute-unit cost of individual Hopper
//! primitives. Each instruction exercises one operation between two
//! `sol_log_compute_units()` syscalls. The CU delta is captured from
//! transaction logs.
//!
//! ## Instruction Mapping
//!
//! | Disc | Operation | Expected CU |
//! |------|-----------|-------------|
//! | 0 | check_signer | ~20 |
//! | 1 | check_writable | ~20 |
//! | 2 | check_owner | ~50 |
//! | 3 | check_account (Tier 1) | ~120 |
//! | 4 | check_keys_eq | ~40 |
//! | 5 | overlay (57 bytes) | ~8 |
//! | 6 | write_header | ~30 |
//! | 7 | zero_init (57 bytes) | ~15 |
//! | 8 | check_account_fast | ~12 |
//! | 9 | emit_event (32 bytes) | ~100 |
//! | 10 | trust_strict_load | ~130 |
//! | 11 | pod_from_bytes (57 bytes) | ~6 |
//! | 12 | receipt_begin_commit (57 bytes) | ~50 |
//! | 13 | fingerprint_check | ~15 |
//! | 14 | state_diff (57 bytes) | ~30 |
//! | 15 | overlay_mut + field_set | ~10 |
//! | 16 | raw_cast_baseline (unsafe ptr) | ~4 |
//! | 17 | receipt_full (enriched fields) | ~80 |
//! | 18 | receipt_emit (64B log) | ~150 |
//! | 19 | proc_macro_typed_dispatch | ~80 |

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;
use hopper::hopper_core::receipt::{Phase, CompatImpact};
#[allow(unused_imports)]
#[cfg(target_os = "solana")]
use hopper::hopper_runtime;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

// --- Benchmark Layout ------------------------------------------------

hopper_layout! {
    /// Benchmark account layout (same as Vault for realistic sizing).
    pub struct BenchVault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 19, version = 1)]
pub struct ProcBenchVault {
    pub balance: WireU64,
    pub pending_rewards: WireU64,
}

#[hopper::context]
pub struct ProcBenchDeposit {
    #[account(mut(balance))]
    pub vault: ProcBenchVault,
}

#[hopper::program]
mod proc_macro_bench_program {
    use super::*;

    #[instruction(19)]
    pub fn typed_deposit(ctx: Context<ProcBenchDeposit>, amount: u64) -> ProgramResult {
        let mut balance = ctx.vault_balance_mut()?;
        let next = balance
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *balance = WireU64::new(next);
        Ok(())
    }
}

// --- Entrypoint ------------------------------------------------------

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let disc = instruction_data
        .first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match disc {
        0 => bench_check_signer(accounts),
        1 => bench_check_writable(accounts),
        2 => bench_check_owner(accounts, program_id),
        3 => bench_check_account_tier1(accounts, program_id),
        4 => bench_check_keys_eq(accounts),
        5 => bench_overlay(accounts, program_id),
        6 => bench_write_header(accounts, program_id),
        7 => bench_zero_init(accounts, program_id),
        8 => bench_check_account_fast(accounts),
        9 => bench_emit_event(),
        10 => bench_trust_strict_load(accounts, program_id),
        11 => bench_pod_from_bytes(accounts, program_id),
        12 => bench_receipt(accounts, program_id),
        13 => bench_fingerprint_check(accounts, program_id),
        14 => bench_state_diff(accounts, program_id),
        15 => bench_overlay_mut_field_set(accounts, program_id),
        16 => bench_raw_cast_baseline(accounts, program_id),
        17 => bench_receipt_full(accounts, program_id),
        18 => bench_receipt_emit(accounts, program_id),
        19 => bench_proc_macro_typed_dispatch(accounts, program_id, instruction_data),
        20 => bench_write_proc_header(accounts, program_id),
        21 => bench_measurement_overhead(),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

// --- Benchmark Entry Points ------------------------------------------
//
// Each function calls sol_log_compute_units() around the measured
// operation. The CU delta is captured by parsing validator logs.
//
// Pattern:
//   log("BEGIN <name>")
//   sol_log_compute_units()   // prints remaining CU
//   <operation>
//   sol_log_compute_units()   // prints remaining CU
//   log("END <name>")
//
// Delta = first_remaining - second_remaining

/// Benchmark: check_signer (~20 CU).
fn bench_check_signer(accounts: &[AccountView]) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN check_signer");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    check_signer(account)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END check_signer");

    Ok(())
}

/// Benchmark: check_writable (~20 CU).
fn bench_check_writable(accounts: &[AccountView]) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN check_writable");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    check_writable(account)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END check_writable");

    Ok(())
}

/// Benchmark: check_owner (~50 CU).
fn bench_check_owner(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN check_owner");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    check_owner(account, program_id)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END check_owner");

    Ok(())
}

/// Benchmark: Tier 1 full check_account (~120 CU).
fn bench_check_account_tier1(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN check_account_tier1");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let _verified = BenchVault::load(account, program_id)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END check_account_tier1");

    Ok(())
}

/// Benchmark: check_keys_eq (~40 CU).
fn bench_check_keys_eq(accounts: &[AccountView]) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN check_keys_eq");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    check_keys_eq(&accounts[0], &accounts[1])?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END check_keys_eq");

    Ok(())
}

/// Benchmark: overlay (~8 CU for 57-byte BenchVault).
fn bench_overlay(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark. No conflicting borrows.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN overlay");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let _vault = BenchVault::overlay(data)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END overlay");

    Ok(())
}

/// Benchmark: write_header (~30 CU).
fn bench_write_header(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;
    check_writable(account)?;

    // SAFETY: Benchmark-only mutation. Exclusive access.
    let data = unsafe { account.borrow_unchecked_mut() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN write_header");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    BenchVault::write_init_header(data)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END write_header");

    Ok(())
}

/// Benchmark: zero_init (~15 CU for 57 bytes).
fn bench_zero_init(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;
    check_writable(account)?;

    // SAFETY: Benchmark-only mutation.
    let data = unsafe { account.borrow_unchecked_mut() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN zero_init");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    zero_init(data);

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END zero_init");

    Ok(())
}

fn bench_write_proc_header(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;
    check_writable(account)?;

    let data = unsafe { account.borrow_unchecked_mut() };
    hopper::hopper_runtime::layout::init_header::<ProcBenchVault>(data)
}

/// Benchmark: check_account_fast (~12 CU).
fn bench_check_account_fast(accounts: &[AccountView]) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN check_account_fast");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    check_signer_fast(account)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END check_account_fast");

    Ok(())
}

/// Benchmark: emit_event (~100 CU for 32-byte payload).
fn bench_emit_event() -> ProgramResult {
    let payload = [0x42u8; 32];

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN emit_event");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    emit_slices(&[&payload]);

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END emit_event");

    Ok(())
}

fn bench_measurement_overhead() -> ProgramResult {
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN measurement_overhead");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END measurement_overhead");

    Ok(())
}

/// Benchmark: TrustProfile::load Strict (~130 CU).
fn bench_trust_strict_load(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;

    let profile = TrustProfile::strict(program_id, &BenchVault::LAYOUT_ID, BenchVault::LEN);

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN trust_strict_load");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let _data = profile.load(account)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END trust_strict_load");

    Ok(())
}

/// Benchmark: pod_from_bytes (~6 CU for 57-byte BenchVault).
///
/// Direct typed view without header validation. Measures the raw
/// overlay cost that Tier B users pay.
fn bench_pod_from_bytes(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark. No conflicting borrows.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN pod_from_bytes");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let _vault = pod_from_bytes::<BenchVault>(data)
        .map_err(|_| ProgramError::InvalidAccountData)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END pod_from_bytes");

    Ok(())
}

/// Benchmark: receipt begin + commit (~50 CU for 57-byte account).
///
/// Measures the cost of StateReceipt snapshot + diff computation.
fn bench_receipt(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN receipt_begin_commit");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let mut receipt = StateReceipt::<64>::begin(&BenchVault::LAYOUT_ID, data);
    receipt.commit(data);

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END receipt_begin_commit");

    Ok(())
}

/// Benchmark: fingerprint check (~15 CU).
///
/// Measures layout_id comparison (8-byte memcmp).
fn bench_fingerprint_check(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN fingerprint_check");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let id = read_layout_id(data).ok_or(ProgramError::AccountDataTooSmall)?;
    if *id != BenchVault::LAYOUT_ID {
        return Err(ProgramError::InvalidAccountData);
    }

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END fingerprint_check");

    Ok(())
}

/// Benchmark: state diff (~30 CU for 57-byte account).
///
/// Measures snapshot + diff computation (without receipt overhead).
fn bench_state_diff(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN state_diff");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let snap = StateSnapshot::<64>::capture(data);
    let diff = snap.diff(data);
    let _ = diff.has_changes();

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END state_diff");

    Ok(())
}

/// Benchmark: overlay_mut + field set (~10 CU).
///
/// Measures the cost of getting a mutable overlay and writing one field.
/// This is the hot-path write cost for Tier A usage.
fn bench_overlay_mut_field_set(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;
    check_writable(account)?;

    // SAFETY: Benchmark-only mutation.
    let data = unsafe { account.borrow_unchecked_mut() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN overlay_mut_field_set");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let vault = BenchVault::overlay_mut(data)?;
    vault.balance.set(vault.balance.get() + 1);

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END overlay_mut_field_set");

    Ok(())
}

/// Benchmark: raw unsafe pointer cast (~4 CU).
///
/// This is the competitor-shaped baseline: a raw `*const u8 as *const T`
/// cast with only a size check. No header validation, no layout_id check,
/// no trust profile. This is what Quasar-style frameworks do.
///
/// The point: Hopper's safe overlay is within 4 CU of this raw path.
fn bench_raw_cast_baseline(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN raw_cast_baseline");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    // Raw cast: only a size check + pointer cast. No header, no fingerprint.
    if data.len() < core::mem::size_of::<BenchVault>() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    // SAFETY: BenchVault is repr(C), alignment 1 (all wire types are u8-aligned).
    // This is the minimal cost path, what competitors pay.
    let _vault = unsafe { &*(data.as_ptr() as *const BenchVault) };

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END raw_cast_baseline");

    Ok(())
}

/// Benchmark: receipt with enriched fields (~80 CU).
///
/// Measures StateReceipt with all enriched fields:
/// phase, compat_impact, validation_bundle_id, migration_flags.
fn bench_receipt_full(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN receipt_full");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let mut receipt = StateReceipt::<64>::begin(&BenchVault::LAYOUT_ID, data);
    receipt.set_phase(Phase::Init);
    receipt.set_compat_impact(CompatImpact::None);
    receipt.set_validation_bundle_id(42);
    receipt.set_migration_flags(0);
    receipt.commit(data);

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END receipt_full");

    Ok(())
}

/// Benchmark: receipt + emit (~150 CU).
///
/// Measures full receipt cycle: begin, set fields, commit, emit as event.
fn bench_receipt_emit(accounts: &[AccountView], program_id: &Address) -> ProgramResult {
    let account = accounts
        .first()
        .ok_or(ProgramError::NotEnoughAccountKeys)?;
    check_owner(account, program_id)?;

    // SAFETY: Read-only benchmark.
    let data = unsafe { account.borrow_unchecked() };

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN receipt_emit");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    let mut receipt = StateReceipt::<64>::begin(&BenchVault::LAYOUT_ID, data);
    receipt.set_phase(Phase::Update);
    receipt.commit(data);
    let wire = receipt.to_bytes();
    emit_slices(&[&wire]);

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END receipt_emit");

    Ok(())
}

/// Benchmark: proc-macro typed dispatch + binding + u64 decode (~80 CU).
fn bench_proc_macro_typed_dispatch(
    accounts: &[AccountView],
    program_id: &Address,
    instruction_data: &[u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);

    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("BEGIN proc_macro_typed_dispatch");
    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();

    proc_macro_bench_program::process_instruction(&mut ctx)?;

    #[cfg(target_os = "solana")]
    hopper_runtime::syscall::sol_log_compute_units();
    #[cfg(target_os = "solana")]
    hopper_runtime::msg!("END proc_macro_typed_dispatch");

    Ok(())
}
