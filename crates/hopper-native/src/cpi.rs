//! Cross-program invocation via `sol_invoke_signed_c`.
//!
//! Provides both checked (borrow-validating) and unchecked invoke paths.

use crate::account_view::AccountView;
use crate::address::address_eq;
use crate::error::ProgramError;
use crate::instruction::{CpiAccount, InstructionView, Signer};
use crate::ProgramResult;
use core::mem::MaybeUninit;

/// Maximum number of accounts in a static CPI call.
pub const MAX_STATIC_CPI_ACCOUNTS: usize = 64;

/// Maximum number of accounts in any CPI call.
pub const MAX_CPI_ACCOUNTS: usize = 128;

/// Maximum return data size (1 KiB).
pub const MAX_RETURN_DATA: usize = 1024;

// ── Unchecked invoke ─────────────────────────────────────────────────

/// Invoke a CPI without borrow validation (lowest CU cost).
///
/// This is Tier C of the CPI surface. The checked variant
/// ([`invoke`](crate::cpi::invoke)) enforces the full contract below
/// before calling this function; prefer that unless you have measured
/// a reason to bypass the validation pass.
///
/// # Safety
///
/// The caller must uphold every one of the following invariants. A
/// violation of any of them is undefined behaviour, because the Solana
/// runtime's `sol_invoke_signed_c` syscall assumes they already hold.
///
/// 1. **No aliasing borrows.** No `&` or `&mut` references into any
///    account data region referenced by `accounts` may be live for
///    the duration of the call. The CPI can (and will) mutate those
///    regions via the callee, and Rust's aliasing rules do not permit
///    the caller to hold outstanding references to memory that is
///    about to change under it.
/// 2. **Account list consistency.** Every `CpiAccount` in `accounts`
///    must correspond to a real account previously passed to the
///    program's entrypoint (same address, same `is_signer` /
///    `is_writable` flags the runtime already knows about). The
///    runtime will not re-derive account permissions; invalid flags
///    propagate into the callee.
/// 3. **Writability coverage.** Every account that the `instruction`
///    marks writable must have `is_writable = true` in `accounts`,
///    and every account the instruction marks as signer must have
///    `is_signer = true`. Mismatches are rejected by the runtime but
///    the rejection path is not cheap and the caller is expected to
///    get this right.
/// 4. **No shared mutable slices across CPIs.** If the same account
///    appears more than once in `accounts` (duplicate accounts), the
///    caller is responsible for ensuring that any subsequent borrow
///    of that account's data respects the CPI's writes.
/// 5. **Valid instruction encoding.** `instruction.program_id`,
///    `instruction.accounts`, and `instruction.data` must all point
///    to valid memory for the duration of the call. An
///    `InstructionView` built from a local `InstructionAccount` slice
///    is fine; one built from a dropped stack slot is not.
///
/// The runtime does not enforce any of these from the caller side —
/// it assumes a well-formed CPI. That is the cost of the Tier C path.
#[inline]
pub unsafe fn invoke_unchecked(
    instruction: &InstructionView,
    accounts: &[CpiAccount],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        // Build the C-ABI instruction struct on the stack.
        // The Solana runtime expects:
        //   struct { program_id: *const u8, accounts: *const SolAccountMeta, acct_len: u64, data: *const u8, data_len: u64 }
        // But sol_invoke_signed_c takes the instruction as raw bytes.
        let result = unsafe {
            crate::syscalls::sol_invoke_signed_c(
                instruction as *const _ as *const u8,
                accounts.as_ptr() as *const u8,
                accounts.len() as u64,
                core::ptr::null(),
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(ProgramError::from(result))
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (instruction, accounts);
        Ok(())
    }
}

/// Invoke a signed CPI without borrow validation.
///
/// Same as [`invoke_unchecked`] but also passes PDA signer seeds so
/// the callee can accept writes that would otherwise require a
/// signature.
///
/// # Safety
///
/// All of [`invoke_unchecked`]'s invariants apply, plus two more for
/// the signer-seeds path:
///
/// 6. **Signer seeds must derive the claimed PDA.** For every
///    `Signer` in `signers_seeds`, the derived address
///    (sha256 of `seeds || program_id || PDA_MARKER`) must equal an
///    address in `accounts` that is marked as signer. A mismatch will
///    cause the runtime to reject the CPI, but the caller is expected
///    to have verified this before reaching the Tier C path.
/// 7. **Seed lifetime.** `signers_seeds` (and every `&[u8]` it points
///    at) must outlive the call. Temporary seed slices built inside a
///    function frame are fine; seeds referencing dropped storage are
///    not.
///
/// For the happy path the caller should hold a `CpiValidator` or
/// equivalent proof-object constructed by the checked path and let
/// that drive both this function's inputs and the aliasing discipline
/// required above.
#[inline]
pub unsafe fn invoke_signed_unchecked(
    instruction: &InstructionView,
    accounts: &[CpiAccount],
    signers_seeds: &[Signer],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let result = unsafe {
            crate::syscalls::sol_invoke_signed_c(
                instruction as *const _ as *const u8,
                accounts.as_ptr() as *const u8,
                accounts.len() as u64,
                signers_seeds.as_ptr() as *const u8,
                signers_seeds.len() as u64,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(ProgramError::from(result))
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (instruction, accounts, signers_seeds);
        Ok(())
    }
}

// ── CPI validation ───────────────────────────────────────────────────

/// Validate that CPI account views match the instruction's expectations.
///
/// Checks:
/// - Sufficient number of accounts.
/// - Address identity (order-dependent matching).
/// - Signer requirements.
/// - Writable requirements.
/// - Borrow compatibility (writable accounts must not be already borrowed,
///   read-only accounts must not be exclusively borrowed).
#[inline]
fn validate_cpi_accounts(
    instruction: &InstructionView,
    account_views: &[&AccountView],
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

        if expected.is_signer && !actual.is_signer() {
            return Err(ProgramError::MissingRequiredSignature);
        }

        if expected.is_writable && !actual.is_writable() {
            return Err(ProgramError::Immutable);
        }

        // Borrow compatibility: writable needs exclusive access,
        // read-only needs at least shared access.
        if expected.is_writable {
            actual.check_borrow_mut()?;
        } else {
            actual.check_borrow()?;
        }

        i += 1;
    }

    Ok(())
}

// ── Checked invoke ───────────────────────────────────────────────────

/// Invoke a CPI with full validation.
///
/// Validates account count, address identity, signer/writable requirements,
/// and borrow compatibility before calling the runtime.
#[inline]
pub fn invoke<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
) -> ProgramResult {
    invoke_signed::<ACCOUNTS>(instruction, account_views, &[])
}

/// Invoke a signed CPI with full validation.
///
/// Validates account count, address identity, signer/writable requirements,
/// and borrow compatibility before calling the runtime.
#[inline]
pub fn invoke_signed<const ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView; ACCOUNTS],
    signers_seeds: &[Signer],
) -> ProgramResult {
    validate_cpi_accounts(instruction, &account_views[..])?;

    // Build CpiAccount array on the stack.
    let mut cpi_accounts: [MaybeUninit<CpiAccount>; ACCOUNTS] =
        unsafe { MaybeUninit::uninit().assume_init() };

    let mut i = 0;
    while i < ACCOUNTS {
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(account_views[i]));
        i += 1;
    }

    // SAFETY: All ACCOUNTS slots are now initialized.
    let accounts: &[CpiAccount; ACCOUNTS] =
        unsafe { &*(cpi_accounts.as_ptr() as *const [CpiAccount; ACCOUNTS]) };

    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts.as_slice())
        } else {
            invoke_signed_unchecked(instruction, accounts.as_slice(), signers_seeds)
        }
    }
}

/// Invoke with a dynamic number of accounts (bounded by const generic).
#[inline]
pub fn invoke_with_bounds<const MAX_ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView],
) -> ProgramResult {
    invoke_signed_with_bounds::<MAX_ACCOUNTS>(instruction, account_views, &[])
}

/// Signed invoke with a dynamic number of accounts (bounded by const generic).
///
/// Returns `Err(InvalidArgument)` if `account_views.len() > MAX_ACCOUNTS`.
/// Validates accounts before invoking.
#[inline]
pub fn invoke_signed_with_bounds<const MAX_ACCOUNTS: usize>(
    instruction: &InstructionView,
    account_views: &[&AccountView],
    signers_seeds: &[Signer],
) -> ProgramResult {
    if account_views.len() > MAX_ACCOUNTS {
        return Err(ProgramError::InvalidArgument);
    }

    validate_cpi_accounts(instruction, account_views)?;

    let mut cpi_accounts: [MaybeUninit<CpiAccount>; MAX_ACCOUNTS] =
        unsafe { MaybeUninit::uninit().assume_init() };

    let count = account_views.len();
    let mut i = 0;
    while i < count {
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(account_views[i]));
        i += 1;
    }

    // SAFETY: first `count` slots are initialized.
    let accounts =
        unsafe { core::slice::from_raw_parts(cpi_accounts.as_ptr() as *const CpiAccount, count) };

    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts)
        } else {
            invoke_signed_unchecked(instruction, accounts, signers_seeds)
        }
    }
}

// ── Return data ──────────────────────────────────────────────────────

/// Set return data for the current instruction.
#[inline(always)]
pub fn set_return_data(data: &[u8]) {
    #[cfg(target_os = "solana")]
    unsafe {
        crate::syscalls::sol_set_return_data(data.as_ptr(), data.len() as u64);
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = data;
    }
}
