//! Cross-program invocation for Hopper programs.
//!
//! Provides both checked (borrow-validating) and unchecked invoke paths.
//! hopper-native-backend uses direct runtime syscalls; compatibility
//! backends delegate through `compat` after Hopper-level validation.

use crate::address::{address_eq, Address};
use crate::error::ProgramError;
use crate::ProgramResult;
use crate::instruction::InstructionView;
use crate::account::AccountView;

// Re-export Signer and Seed so callers can use `cpi::Signer` / `cpi::Seed`.
pub use crate::instruction::{Signer, Seed};

/// Maximum number of accounts in a static CPI call.
pub const MAX_STATIC_CPI_ACCOUNTS: usize = 64;

/// Maximum number of accounts in any CPI call.
pub const MAX_CPI_ACCOUNTS: usize = 128;

/// Maximum return data size (1 KiB).
pub const MAX_RETURN_DATA: usize = 1024;

// ══════════════════════════════════════════════════════════════════════
//  hopper-native-backend CPI
// ══════════════════════════════════════════════════════════════════════

#[cfg(feature = "hopper-native-backend")]
use crate::instruction::CpiAccount;
#[cfg(feature = "hopper-native-backend")]
use core::mem::MaybeUninit;

// ── Unchecked invoke ─────────────────────────────────────────────────

/// Invoke a CPI without borrow validation (lowest CU cost).
///
/// # Safety
///
/// The caller must ensure no account data borrows conflict with the CPI.
#[cfg(feature = "hopper-native-backend")]
#[inline]
pub unsafe fn invoke_unchecked(
    instruction: &InstructionView,
    accounts: &[CpiAccount],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let result = unsafe {
            hopper_native::syscalls::sol_invoke_signed_c(
                instruction as *const _ as *const u8,
                accounts.as_ptr() as *const u8,
                accounts.len() as u64,
                core::ptr::null(),
                0,
            )
        };
        if result == 0 { Ok(()) } else { Err(ProgramError::from(result)) }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (instruction, accounts);
        Ok(())
    }
}

/// Invoke a signed CPI without borrow validation.
///
/// # Safety
///
/// The caller must ensure no account data borrows conflict with the CPI.
#[cfg(feature = "hopper-native-backend")]
#[inline]
pub unsafe fn invoke_signed_unchecked(
    instruction: &InstructionView,
    accounts: &[CpiAccount],
    signers_seeds: &[Signer],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let result = unsafe {
            hopper_native::syscalls::sol_invoke_signed_c(
                instruction as *const _ as *const u8,
                accounts.as_ptr() as *const u8,
                accounts.len() as u64,
                signers_seeds.as_ptr() as *const u8,
                signers_seeds.len() as u64,
            )
        };
        if result == 0 { Ok(()) } else { Err(ProgramError::from(result)) }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (instruction, accounts, signers_seeds);
        Ok(())
    }
}

// ── CPI validation ───────────────────────────────────────────────────

/// Reject duplicate writable accounts before invoking CPI.
#[inline]
fn validate_no_duplicate_writable(
    instruction: &InstructionView,
    account_views: &[&AccountView],
) -> ProgramResult {
    let mut i = 0;
    while i < instruction.accounts.len() {
        if instruction.accounts[i].is_writable {
            let mut j = i + 1;
            while j < instruction.accounts.len() {
                if instruction.accounts[j].is_writable
                    && address_eq(account_views[i].address(), account_views[j].address())
                {
                    return Err(ProgramError::AccountBorrowFailed);
                }
                j += 1;
            }
        }
        i += 1;
    }
    Ok(())
}

#[inline]
fn signer_matches_pda(program_id: &Address, account: &Address, signers_seeds: &[Signer]) -> bool {
    let mut i = 0;
    while i < signers_seeds.len() {
        let signer = &signers_seeds[i];
        let seeds = unsafe {
            core::slice::from_raw_parts(signer.seeds, signer.len as usize)
        };

        if seeds.len() <= crate::address::MAX_SEEDS {
            let mut seed_refs: [&[u8]; crate::address::MAX_SEEDS] = [&[]; crate::address::MAX_SEEDS];
            let mut j = 0;
            while j < seeds.len() {
                seed_refs[j] = unsafe {
                    core::slice::from_raw_parts(seeds[j].seed, seeds[j].len as usize)
                };
                j += 1;
            }

            if let Ok(derived) = crate::compat::create_program_address(&seed_refs[..seeds.len()], program_id) {
                if address_eq(&derived, account) {
                    return true;
                }
            }
        }

        i += 1;
    }

    false
}

/// Validate CPI account views match the instruction's expectations.
#[inline]
fn validate_cpi_accounts(
    instruction: &InstructionView,
    account_views: &[&AccountView],
    signers_seeds: &[Signer],
) -> ProgramResult {
    if account_views.len() < instruction.accounts.len() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let mut i = 0;
    while i < instruction.accounts.len() {
        let expected = &instruction.accounts[i];
        let actual = account_views[i];

        if !address_eq(actual.address(), expected.address) {
            return Err(ProgramError::InvalidAccountData);
        }

        if expected.is_signer
            && !actual.is_signer()
            && !signer_matches_pda(instruction.program_id, actual.address(), signers_seeds)
        {
            return Err(ProgramError::MissingRequiredSignature);
        }

        if expected.is_writable && !actual.is_writable() {
            return Err(ProgramError::Immutable);
        }

        if expected.is_writable {
            actual.check_borrow_mut()?;
        } else {
            actual.check_borrow()?;
        }

        i += 1;
    }

    validate_no_duplicate_writable(instruction, account_views)?;

    Ok(())
}

// ── Checked invoke ───────────────────────────────────────────────────

/// Invoke a CPI with full validation.
#[cfg(feature = "hopper-native-backend")]
#[inline]
pub fn invoke<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
) -> ProgramResult {
    invoke_signed::<ACCOUNTS>(instruction, account_views, &[])
}

/// Invoke a signed CPI with full validation.
#[cfg(feature = "hopper-native-backend")]
#[inline]
pub fn invoke_signed<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
    signers_seeds: &[Signer],
) -> ProgramResult {
    validate_cpi_accounts(instruction, &account_views[..], signers_seeds)?;

    let mut cpi_accounts: [MaybeUninit<CpiAccount>; ACCOUNTS] =
        unsafe { MaybeUninit::uninit().assume_init() };

    let mut i = 0;
    while i < ACCOUNTS {
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(account_views[i]));
        i += 1;
    }

    let accounts: &[CpiAccount; ACCOUNTS] = unsafe {
        &*(cpi_accounts.as_ptr() as *const [CpiAccount; ACCOUNTS])
    };

    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts.as_slice())
        } else {
            invoke_signed_unchecked(instruction, accounts.as_slice(), signers_seeds)
        }
    }
}

/// Invoke with a dynamic number of accounts (bounded by const generic).
#[cfg(feature = "hopper-native-backend")]
#[inline]
pub fn invoke_with_bounds<const MAX_ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView],
) -> ProgramResult {
    invoke_signed_with_bounds::<MAX_ACCOUNTS>(instruction, account_views, &[])
}

/// Signed invoke with a dynamic number of accounts (bounded by const generic).
#[cfg(feature = "hopper-native-backend")]
#[inline]
pub fn invoke_signed_with_bounds<const MAX_ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView],
    signers_seeds: &[Signer],
) -> ProgramResult {
    if account_views.len() > MAX_ACCOUNTS {
        return Err(ProgramError::InvalidArgument);
    }

    validate_cpi_accounts(instruction, account_views, signers_seeds)?;

    let mut cpi_accounts: [MaybeUninit<CpiAccount>; MAX_ACCOUNTS] =
        unsafe { MaybeUninit::uninit().assume_init() };

    let count = account_views.len();
    let mut i = 0;
    while i < count {
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(account_views[i]));
        i += 1;
    }

    let accounts = unsafe {
        core::slice::from_raw_parts(cpi_accounts.as_ptr() as *const CpiAccount, count)
    };

    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts)
        } else {
            invoke_signed_unchecked(instruction, accounts, signers_seeds)
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Compatibility backends CPI
// ══════════════════════════════════════════════════════════════════════

/// Invoke a CPI through the active compatibility backend.
#[cfg(any(feature = "pinocchio-backend", feature = "solana-program-backend"))]
#[inline]
pub fn invoke<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
) -> ProgramResult {
    invoke_signed::<ACCOUNTS>(instruction, account_views, &[])
}

/// Invoke a signed CPI through the active compatibility backend.
#[cfg(any(feature = "pinocchio-backend", feature = "solana-program-backend"))]
#[inline]
pub fn invoke_signed<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
    signers_seeds: &[Signer],
) -> ProgramResult {
    validate_cpi_accounts(instruction, &account_views[..], signers_seeds)?;
    crate::compat::invoke_signed(instruction, account_views, signers_seeds)
}

// ── Return data ──────────────────────────────────────────────────────

/// Set return data for the current instruction.
#[inline(always)]
pub fn set_return_data(data: &[u8]) {
    crate::compat::set_return_data(data)
}

#[cfg(all(test, feature = "hopper-native-backend"))]
mod tests {
    use super::*;

    use crate::InstructionAccount;
    use hopper_native::{AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED};

    fn make_account(address: [u8; 32]) -> (std::vec::Vec<u8>, AccountView) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + 16];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array(address),
                owner: NativeAddress::new_from_array([9; 32]),
                lamports: 1,
                data_len: 16,
            });
        }
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    #[test]
    fn duplicate_writable_accounts_are_rejected_before_cpi() {
        let (_first_backing, first) = make_account([3; 32]);
        let (_second_backing, second) = make_account([3; 32]);

        let instruction_accounts = [
            InstructionAccount::writable(first.address()),
            InstructionAccount::writable(second.address()),
        ];
        let program_id = Address::new_from_array([7; 32]);
        let instruction = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &instruction_accounts,
        };

        let err = validate_no_duplicate_writable(&instruction, &[&first, &second]).unwrap_err();
        assert_eq!(err, ProgramError::AccountBorrowFailed);
    }
}
