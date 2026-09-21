//! Cross-program invocation via `sol_invoke_signed_c`.
//!
//! Provides both checked (borrow-validating) and unchecked invoke paths.

use crate::account_view::AccountView;
use crate::address::{address_eq, Address};
use crate::error::ProgramError;
use crate::instruction::{
    preflight_cpi_accounts, CpiAccount, InstructionAccount, InstructionView, Signer,
};
use crate::ProgramResult;
use core::mem::MaybeUninit;

#[cfg(all(test, not(target_os = "solana")))]
static LAST_HOST_ACCOUNT_INFOS_LEN: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

/// Default stack-sized ceiling for a *static* CPI call.
///
/// This is deliberately the low pre-SIMD-0339 value: it sizes the
/// `MaybeUninit` account/meta scratch arrays that some fixed-shape CPI
/// helpers stack-allocate, and every SBF call frame is only 4 KiB. Raising
/// it would grow those arrays for *every* program, including the vast
/// majority that never approach 64 accounts. Callers that genuinely need a
/// wider CPI opt in per-call through the const-generic `MAX_ACCOUNTS`
/// parameter (up to [`MAX_CPI_ACCOUNTS`]), which costs nothing when unused.
pub const MAX_STATIC_CPI_ACCOUNTS: usize = 64;

/// Hard ceiling on the number of account-infos in any single CPI.
///
/// Raised to 255 for **SIMD-0339** (`increase_cpi_account_info_limit`; active
/// on mainnet at slot 403,056,000), which lifts the runtime CPI account-info limit
/// from 64 to 255. This is a *ceiling* only; it does not size any array, so
/// widening it does not cost stack for programs that stay small. A per-call
/// const-generic `MAX_ACCOUNTS` still governs the actual scratch allocation.
pub const MAX_CPI_ACCOUNTS: usize = 255;

/// Maximum return data size (1 KiB).
pub const MAX_RETURN_DATA: usize = 1024;

/// Exact C instruction descriptor consumed by `sol_invoke_signed_c`.
///
/// `InstructionView` contains Rust fat slices and has a different field order;
/// it must never be passed to the syscall directly.
#[cfg(any(target_os = "solana", test))]
#[repr(C)]
struct CInstruction {
    program_id: *const Address,
    accounts: *const u8,
    accounts_len: u64,
    data: *const u8,
    data_len: u64,
}

#[cfg(any(target_os = "solana", test))]
impl CInstruction {
    #[inline(always)]
    fn from_view(instruction: &InstructionView<'_, '_, '_, '_>) -> Self {
        Self {
            program_id: instruction.program_id as *const Address,
            accounts: instruction.accounts.as_ptr() as *const u8,
            accounts_len: instruction.accounts.len() as u64,
            data: instruction.data.as_ptr(),
            data_len: instruction.data.len() as u64,
        }
    }
}

#[cfg(any(target_os = "solana", test))]
const _: () = {
    assert!(core::mem::size_of::<CInstruction>() == 40);
    assert!(core::mem::align_of::<CInstruction>() == 8);
    assert!(core::mem::offset_of!(CInstruction, program_id) == 0);
    assert!(core::mem::offset_of!(CInstruction, accounts) == 8);
    assert!(core::mem::offset_of!(CInstruction, accounts_len) == 16);
    assert!(core::mem::offset_of!(CInstruction, data) == 24);
    assert!(core::mem::offset_of!(CInstruction, data_len) == 32);
};

#[inline(always)]
fn specialized_instruction_accounts<'a, const ACCOUNTS: usize>(
    accounts: &[CpiAccount<'a>; ACCOUNTS],
    writable_mask: usize,
    signer_mask: usize,
) -> [InstructionAccount<'a>; ACCOUNTS] {
    core::array::from_fn(|index| {
        accounts[index].instruction_account(
            writable_mask & (1usize << index) != 0,
            signer_mask & (1usize << index) != 0,
        )
    })
}

/// Invoke a fixed-shape specialized builder through the same checked C-ABI
/// boundary as the generic CPI surface.
#[inline]
pub(crate) fn invoke_specialized_signed<'a, const ACCOUNTS: usize>(
    program_id: &Address,
    data: &[u8],
    accounts: &[CpiAccount<'a>; ACCOUNTS],
    writable_mask: usize,
    signer_mask: usize,
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    preflight_cpi_accounts(accounts, writable_mask)?;
    let instruction_accounts =
        specialized_instruction_accounts(accounts, writable_mask, signer_mask);
    let instruction = InstructionView {
        program_id,
        data,
        accounts: &instruction_accounts,
    };

    // SAFETY: the protocol masks above produced exact metas; preflight checked
    // outer writable privileges and every account's borrow compatibility.
    // The signed form with an empty seed list is the unsigned invoke (the
    // syscall reads the seed pointer only when the count is nonzero), so one
    // syscall site serves both instead of the two copies a branch compiled to.
    unsafe { invoke_signed_unchecked(&instruction, accounts, signers_seeds) }
}

// ---------------------------------------------------------------------

/// Invoke a CPI without borrow validation (lowest CU cost).
///
/// This is Tier C of the CPI surface. The checked variant
/// ([`invoke`]) enforces the full contract below
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
/// 2. **Account list consistency.** Every `CpiAccount<'_>` in `accounts`
///    must correspond to a real account previously passed to the
///    program's entrypoint (same address, same `is_Signer<'_, '_>` /
///    `is_writable` flags the runtime already knows about). The
///    runtime will not re-derive account permissions; invalid flags
///    propagate into the callee.
/// 3. **Writability coverage.** Every account that the `instruction`
///    marks writable must have `is_writable = true` in `accounts`,
///    and every account the instruction marks as Signer<'_, '_> must have
///    `is_Signer<'_, '_> = true`. Mismatches are rejected by the runtime but
///    the rejection path is not cheap and the caller is expected to
///    get this right.
/// 4. **No shared mutable slices across CPIs.** If the same account
///    appears more than once in `accounts` (duplicate accounts), the
///    caller is responsible for ensuring that any subsequent borrow
///    of that account's data respects the CPI's writes.
/// 5. **Valid instruction encoding.** `instruction.program_id`,
///    `instruction.accounts`, and `instruction.data` must all point
///    to valid memory for the duration of the call. An
///    `InstructionView<'_, '_, '_, '_>` built from a local `InstructionAccount` slice
///    is fine; one built from a dropped stack slot is not.
///
/// The runtime does not enforce any of these from the caller side -
/// it assumes a well-formed CPI. That is the cost of the Tier C path.
#[inline]
pub unsafe fn invoke_unchecked(
    instruction: &InstructionView<'_, '_, '_, '_>,
    accounts: &[CpiAccount<'_>],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let c_instruction = CInstruction::from_view(instruction);
        // Prevent LLVM from moving stack descriptor/meta initialization below
        // the opaque runtime call.
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        // SAFETY: the caller upholds the unchecked CPI contract and
        // `c_instruction` is the exact repr(C) syscall descriptor.
        let result = unsafe {
            crate::syscalls::sol_invoke_signed_c(
                &c_instruction as *const _ as *const u8,
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
        #[cfg(test)]
        LAST_HOST_ACCOUNT_INFOS_LEN.store(accounts.len(), core::sync::atomic::Ordering::SeqCst);
        let _ = (instruction, accounts);
        Ok(())
    }
}

/// Invoke a signed CPI without borrow validation.
///
/// Same as [`invoke_unchecked`] but also passes PDA Signer<'_, '_> seeds so
/// the callee can accept writes that would otherwise require a
/// signature.
///
/// # Safety
///
/// All of [`invoke_unchecked`]'s invariants apply, plus two more for
/// the Signer<'_, '_>-seeds path:
///
/// 6. **Signer<'_, '_> seeds must derive the claimed PDA.** For every
///    `Signer<'_, '_>` in `signers_seeds`, the derived address
///    (sha256 of `seeds || program_id || PDA_MARKER`) must equal an
///    address in `accounts` that is marked as Signer<'_, '_>. A mismatch will
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
    instruction: &InstructionView<'_, '_, '_, '_>,
    accounts: &[CpiAccount<'_>],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let c_instruction = CInstruction::from_view(instruction);
        // Keep every stack-backed descriptor, meta, account and signer seed
        // fully materialized before the opaque syscall boundary.
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        // SAFETY: the caller upholds the unchecked CPI contract and
        // `c_instruction` is the exact repr(C) syscall descriptor.
        let result = unsafe {
            crate::syscalls::sol_invoke_signed_c(
                &c_instruction as *const _ as *const u8,
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
        #[cfg(test)]
        LAST_HOST_ACCOUNT_INFOS_LEN.store(accounts.len(), core::sync::atomic::Ordering::SeqCst);
        let _ = (instruction, accounts, signers_seeds);
        Ok(())
    }
}

// ---------------------------------------------------------------------

/// Invoke a CPI with full validation.
///
/// Validates account count, address identity, Signer<'_, '_>/writable requirements,
/// and borrow compatibility before calling the runtime.
#[inline]
pub fn invoke<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
) -> ProgramResult {
    invoke_signed::<ACCOUNTS>(instruction, account_views, &[])
}

/// Invoke a signed CPI with full validation.
///
/// Validates account count, address identity, Signer<'_, '_>/writable requirements,
/// and borrow compatibility before calling the runtime.
#[inline]
pub fn invoke_signed<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    let metas_len = instruction.accounts.len();
    if ACCOUNTS < metas_len {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    // Fused validate+build: one pass over the instruction metas performs the
    // address/signer/writable/borrow checks and materializes exactly the
    // matching CpiAccount prefix. Extra caller views are neither validated nor
    // forwarded because the callee instruction cannot access them.
    let mut cpi_accounts: [MaybeUninit<CpiAccount<'_>>; ACCOUNTS] =
        // SAFETY: an array of `MaybeUninit<T>` is valid in any initialization
        // state; the initialized prefix is sliced to `metas_len` below, and on
        // an early return the array is discarded unread (`CpiAccount` is
        // `Copy`, so no drop runs).
        unsafe { MaybeUninit::uninit().assume_init() };

    let mut i = 0;
    while i < metas_len {
        let actual = account_views[i];
        let expected = &instruction.accounts[i];

        if !address_eq(actual.address(), expected.address) {
            return Err(ProgramError::InvalidAccountData);
        }

        // Non-empty signer seeds may grant this instruction-only signer
        // privilege to a PDA. The SVM remains the derivation authority.
        if expected.is_signer && !actual.is_signer() && signers_seeds.is_empty() {
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
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(actual));
        i += 1;
    }

    // SAFETY: exactly the first `metas_len` slots are initialized. Extra
    // caller views are absent from the instruction and are not forwarded.
    let accounts = unsafe {
        core::slice::from_raw_parts(cpi_accounts.as_ptr() as *const CpiAccount<'_>, metas_len)
    };

    // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts)
        } else {
            invoke_signed_unchecked(instruction, accounts, signers_seeds)
        }
    }
}

/// Invoke with a dynamic number of accounts (bounded by const generic).
#[inline]
pub fn invoke_with_bounds<const MAX_ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
) -> ProgramResult {
    invoke_signed_with_bounds::<MAX_ACCOUNTS>(instruction, account_views, &[])
}

/// Signed invoke with a dynamic number of accounts (bounded by const generic).
///
/// Returns `Err(InvalidArgument)` if `account_views.len() > MAX_ACCOUNTS`.
/// Validates accounts before invoking.
#[inline]
pub fn invoke_signed_with_bounds<const MAX_ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    if account_views.len() > MAX_ACCOUNTS {
        return Err(ProgramError::InvalidArgument);
    }

    let metas_len = instruction.accounts.len();
    let count = account_views.len();
    if count < metas_len {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let mut cpi_accounts: [MaybeUninit<CpiAccount<'_>>; MAX_ACCOUNTS] =
        // SAFETY: an array of `MaybeUninit<T>` is valid in any initialization
        // state; the first `metas_len` slots are written before being read
        // below, and on an early return the array is discarded unread
        // (`CpiAccount` is `Copy`, so no drop runs).
        unsafe { MaybeUninit::uninit().assume_init() };

    // Fused validate+build (see `invoke_signed`): one pass validates each
    // instruction meta and writes only its matching scratch slot.
    let mut i = 0;
    while i < metas_len {
        let actual = account_views[i];
        let expected = &instruction.accounts[i];

        if !address_eq(actual.address(), expected.address) {
            return Err(ProgramError::InvalidAccountData);
        }

        if expected.is_signer && !actual.is_signer() && signers_seeds.is_empty() {
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
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(actual));
        i += 1;
    }

    // SAFETY: first `metas_len` slots are initialized; extra caller views are
    // not part of the instruction and are not forwarded to the syscall.
    let accounts = unsafe {
        core::slice::from_raw_parts(cpi_accounts.as_ptr() as *const CpiAccount<'_>, metas_len)
    };

    // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts)
        } else {
            invoke_signed_unchecked(instruction, accounts, signers_seeds)
        }
    }
}

// ---------------------------------------------------------------------

/// Set return data for the current instruction.
#[inline(always)]
pub fn set_return_data(data: &[u8]) {
    #[cfg(target_os = "solana")]
    // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        crate::syscalls::sol_set_return_data(data.as_ptr(), data.len() as u64);
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = data;
    }
}

#[cfg(test)]
mod abi_tests {
    use super::*;
    use crate::instruction::Seed;
    use crate::{RuntimeAccount, NOT_BORROWED};

    #[repr(C)]
    struct Backing {
        header: RuntimeAccount,
        data: [u8; 8],
    }

    fn backing(tag: u8) -> Backing {
        Backing {
            header: RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 8,
                address: Address::new_from_array([tag; 32]),
                owner: Address::new_from_array([0xA5; 32]),
                lamports: 5,
                data_len: 8,
            },
            data: [tag; 8],
        }
    }

    #[test]
    fn c_instruction_and_specialized_meta_encoding_match_the_c_abi() {
        let program_id = Address::new_from_array([9; 32]);
        let key = Address::new_from_array([3; 32]);
        let data = [1, 2, 3];
        let metas = [InstructionAccount::new(&key, true, false)];
        let view = InstructionView {
            program_id: &program_id,
            data: &data,
            accounts: &metas,
        };
        let c = CInstruction::from_view(&view);
        assert_eq!(c.program_id, &program_id as *const Address);
        assert_eq!(c.accounts, metas.as_ptr() as *const u8);
        assert_eq!(c.accounts_len, 1);
        assert_eq!(c.data, data.as_ptr());
        assert_eq!(c.data_len, 3);

        let mut first_backing = backing(1);
        let mut second_backing = backing(2);
        let mut third_backing = backing(3);
        // SAFETY: each backing is an owned, aligned RuntimeAccount header
        // followed by its data, and outlives the view built over it; the three
        // backings are distinct allocations, so the views never alias.
        let first = unsafe { AccountView::new_unchecked(&mut first_backing.header) };
        // SAFETY: same contract as `first`, over its own backing.
        let second = unsafe { AccountView::new_unchecked(&mut second_backing.header) };
        // SAFETY: same contract as `first`, over its own backing.
        let third = unsafe { AccountView::new_unchecked(&mut third_backing.header) };
        let infos = [
            CpiAccount::from(&first),
            CpiAccount::from(&second),
            CpiAccount::from(&third),
        ];
        let encoded = specialized_instruction_accounts(&infos, 0b011, 0b100);
        assert!(encoded[0].is_writable);
        assert!(encoded[1].is_writable);
        assert!(!encoded[2].is_writable);
        assert!(!encoded[0].is_signer);
        assert!(!encoded[1].is_signer);
        assert!(encoded[2].is_signer);
        assert_eq!(encoded[0].address, first.address());
        assert_eq!(encoded[1].address, second.address());
        assert_eq!(encoded[2].address, third.address());
    }

    #[test]
    fn checked_paths_accept_pda_seeds_and_forward_only_instruction_metas() {
        let mut signer_backing = backing(4);
        let mut extra_backing = backing(5);
        // SAFETY: both backings are owned, aligned RuntimeAccount headers with
        // their data, distinct from each other, and outlive the views.
        let signer_view = unsafe { AccountView::new_unchecked(&mut signer_backing.header) };
        let extra_view = unsafe { AccountView::new_unchecked(&mut extra_backing.header) };
        let metas = [InstructionAccount::readonly_signer(signer_view.address())];
        let program_id = Address::new_from_array([6; 32]);
        let instruction = InstructionView {
            program_id: &program_id,
            data: &[],
            accounts: &metas,
        };

        LAST_HOST_ACCOUNT_INFOS_LEN.store(usize::MAX, core::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            invoke::<1>(&instruction, &[&signer_view]),
            Err(ProgramError::MissingRequiredSignature)
        );
        assert_eq!(
            LAST_HOST_ACCOUNT_INFOS_LEN.load(core::sync::atomic::Ordering::SeqCst),
            usize::MAX,
            "unsigned signer failure must happen before invoke"
        );

        let seed = Seed::from(&b"pda"[..]);
        let signer_seeds = [seed];
        let signers = [Signer::from(&signer_seeds)];
        invoke_signed::<1>(&instruction, &[&signer_view], &signers).unwrap();
        assert_eq!(
            LAST_HOST_ACCOUNT_INFOS_LEN.load(core::sync::atomic::Ordering::SeqCst),
            1
        );

        let caller_views = [&signer_view, &extra_view];
        invoke_signed::<2>(&instruction, &caller_views, &signers).unwrap();
        assert_eq!(
            LAST_HOST_ACCOUNT_INFOS_LEN.load(core::sync::atomic::Ordering::SeqCst),
            1,
            "fixed path must not forward caller views absent from metas"
        );
        invoke_signed_with_bounds::<2>(&instruction, &caller_views, &signers).unwrap();
        assert_eq!(
            LAST_HOST_ACCOUNT_INFOS_LEN.load(core::sync::atomic::Ordering::SeqCst),
            1,
            "bounded path must not forward caller views absent from metas"
        );
    }
}
