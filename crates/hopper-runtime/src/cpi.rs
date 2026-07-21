//! Cross-program invocation for Hopper programs.
//!
//! Provides both checked (borrow-validating) and unchecked invoke paths.
//! Hopper uses direct runtime syscalls after Hopper-level validation.

use crate::account::AccountView;
use crate::address::{address_eq, Address};
use crate::error::ProgramError;
use crate::instruction::{CpiAccount, InstructionView};
use crate::ProgramResult;
use core::mem::MaybeUninit;

#[cfg(target_os = "solana")]
use crate::instruction::InstructionAccount;

// Re-export Signer and Seed so callers can use `cpi::Signer` / `cpi::Seed`.
pub use crate::instruction::{Seed, Signer};

/// Default stack-sized ceiling for a *static* CPI call.
///
/// This is deliberately the low pre-SIMD-0339 value. It is used to size
/// fixed `MaybeUninit` scratch arrays (e.g. `token.rs`) that live on the
/// SBF stack, whose per-frame budget is only 4 KiB. Raising this constant
/// would grow those arrays for every program regardless of need. Wide-CPI
/// callers instead pick a larger per-call const-generic `MAX_ACCOUNTS`
/// (bounded by [`MAX_CPI_ACCOUNTS`]), which is zero-cost when unused.
pub const MAX_STATIC_CPI_ACCOUNTS: usize = 64;

/// Hard ceiling on the number of account-infos in any single CPI.
///
/// Raised from 128 to 255 for **SIMD-0339** (`increase_cpi_account_info_limit`,
/// agave gate `H6iVbVaDZgDphcPbcZwc5LoznMPWQfnJ1AM7L1xzqvt5`, live on testnet
/// epoch 883), which lifts the runtime CPI account-info limit from 64 to 255.
/// This is a *ceiling* constant only — it does not size any stack array, so
/// widening it costs nothing for programs that stay small. The actual scratch
/// allocation is governed by a per-call const-generic `MAX_ACCOUNTS`.
///
/// Under 0339 every distinct account-info also carries a per-info CU cost, so
/// passing the *fewest* infos per CPI becomes a cost axis. [`DynCpi`] exploits
/// this by deduplicating account-infos by pubkey — see
/// [`invoke_signed_deduped`].
///
/// [`DynCpi`]: crate::dyn_cpi::DynCpi
pub const MAX_CPI_ACCOUNTS: usize = 255;

/// Maximum return data size (1 KiB).
pub const MAX_RETURN_DATA: usize = 1024;

// -- Hopper CPI -------------------------------------------------------

#[cfg(target_os = "solana")]
#[repr(C)]
struct CInstruction<'a> {
    program_id: *const Address,
    accounts: *const InstructionAccount<'a>,
    accounts_len: u64,
    data: *const u8,
    data_len: u64,
}

// -- Unchecked invoke -------------------------------------------------

/// Invoke a CPI without borrow validation (lowest CU cost).
///
/// # Safety
///
/// The caller must ensure no account data borrows conflict with the CPI.
#[inline]
pub unsafe fn invoke_unchecked(
    instruction: &InstructionView<'_, '_, '_, '_>,
    accounts: &[CpiAccount<'_>],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let c_instruction = CInstruction {
            program_id: instruction.program_id as *const Address,
            accounts: instruction.accounts.as_ptr(),
            accounts_len: instruction.accounts.len() as u64,
            data: instruction.data.as_ptr(),
            data_len: instruction.data.len() as u64,
        };

        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let result = unsafe {
            hopper_native::syscalls::sol_invoke_signed_c(
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
        let _ = (instruction, accounts);
        Ok(())
    }
}

/// Invoke a signed CPI without borrow validation.
///
/// # Safety
///
/// The caller must ensure no account data borrows conflict with the CPI.
#[inline]
pub unsafe fn invoke_signed_unchecked(
    instruction: &InstructionView<'_, '_, '_, '_>,
    accounts: &[CpiAccount<'_>],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let c_instruction = CInstruction {
            program_id: instruction.program_id as *const Address,
            accounts: instruction.accounts.as_ptr(),
            accounts_len: instruction.accounts.len() as u64,
            data: instruction.data.as_ptr(),
            data_len: instruction.data.len() as u64,
        };

        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let result = unsafe {
            hopper_native::syscalls::sol_invoke_signed_c(
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
        let _ = (instruction, accounts, signers_seeds);
        Ok(())
    }
}

// ---------------------------------------------------------------------

/// Reject duplicate writable accounts before invoking CPI.
#[inline]
fn validate_no_duplicate_writable(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
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
fn signer_authority_supplied(signers_seeds: &[Signer<'_, '_>]) -> bool {
    // PDA signer addresses are derived with the *calling* program id. A CPI
    // instruction only carries the callee id, so this layer cannot reproduce
    // that derivation without accidentally checking against the wrong
    // program. The SVM's `sol_invoke_signed` syscall performs the
    // authoritative seed validation and required-signer match. Preflight can
    // safely reject the unambiguous no-authority case and otherwise defer the
    // cryptographic check to the runtime.
    //
    // Host System-program emulation follows the same rule. It cannot know the
    // caller id either, so signed host tests should validate their PDA inputs
    // separately when caller-id correctness is the subject of the test.
    !signers_seeds.is_empty()
}

/// Per-account meta↔view correspondence + borrow-state validation — the
/// borrow-checked tier.
///
/// For each account: the view at index `i` must name the same address as
/// meta `i` (so the borrow check applies to the correct account), then
/// writable metas must be exclusively borrowable
/// ([`AccountView::check_borrow_mut`]) and read-only metas must be
/// shared-borrowable ([`AccountView::check_borrow`]). This is exactly the
/// per-account check Pinocchio's safe `invoke` performs before a CPI. No
/// signer, writability, or duplicate-writable validation happens here —
/// those belong to the default [`invoke_signed`] tier.
#[inline]
fn validate_cpi_borrows(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
) -> ProgramResult {
    if account_views.len() < instruction.accounts.len() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let mut i = 0;
    while i < instruction.accounts.len() {
        // The borrow state must be validated against the account the meta
        // actually names, not whatever view happens to sit at index `i`.
        // Without this, a caller passing views in a different order than
        // the metas would borrow-check the wrong (account, mutability)
        // pair and then reach `invoke_unchecked` with its aliasing
        // contract undischarged — UB from safe code. Pinocchio's safe
        // `invoke` keeps exactly this check for exactly this reason
        // (solana-instruction-view `cpi.rs`).
        if !address_eq(account_views[i].address(), instruction.accounts[i].address) {
            return Err(ProgramError::InvalidArgument);
        }
        if instruction.accounts[i].is_writable {
            account_views[i].check_borrow_mut()?;
        } else {
            account_views[i].check_borrow()?;
        }
        i += 1;
    }

    // BLD-MUT writable hand-off gate: swept once per CPI behind the
    // liveness branch, never reachable from the per-meta loop (the
    // 2026-07-09 bisect measured closure-reachable gate machinery at
    // ~+52 CU per router hop for ungated programs; see invoke_signed).
    if crate::write_policy::lamport_gate_active() {
        let mut m = 0;
        while m < instruction.accounts.len() {
            if instruction.accounts[m].is_writable {
                crate::write_policy::check_lamport_delegation(account_views[m].address())?;
            }
            m += 1;
        }
    }

    Ok(())
}

#[cfg(not(target_os = "solana"))]
fn is_host_system_transfer(instruction: &InstructionView<'_, '_, '_, '_>) -> bool {
    // `SYSTEM_PROGRAM_ID` is the all-zero address, so an OR-fold
    // is-zero check is equivalent to (and cheaper than) comparing
    // against the constant.
    crate::address::address_is_zero(instruction.program_id)
        && instruction.data.len() == 12
        && instruction.data[0..4] == [2, 0, 0, 0]
}

// This validator only walks `instruction.accounts` (address/signer/
// writable/borrow checks) — it never inspects `instruction.data` — so it
// is not actually Transfer-specific. `emulate_host_system_create_account`,
// `emulate_host_system_allocate`, and `emulate_host_system_assign` below
// reuse it verbatim for their host emulations instead of duplicating the
// same four checks under a second name. `min_views` is each instruction's
// account arity (2 for Transfer/CreateAccount, 1 for Allocate/Assign): the
// emulations index `account_views[..min_views]` directly, so the guard
// must refuse a shorter hand-built view list before they do.
#[cfg(not(target_os = "solana"))]
fn validate_host_system_transfer(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
    signers_seeds: &[Signer<'_, '_>],
    min_views: usize,
) -> ProgramResult {
    if account_views.len() < instruction.accounts.len() || account_views.len() < min_views {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let mut i = 0;
    while i < instruction.accounts.len() {
        let expected = &instruction.accounts[i];
        let actual = account_views[i];

        if !address_eq(actual.address(), expected.address) {
            return Err(ProgramError::InvalidAccountData);
        }
        if expected.is_signer && !actual.is_signer() && !signer_authority_supplied(signers_seeds) {
            return Err(ProgramError::MissingRequiredSignature);
        }
        if expected.is_writable && !actual.is_writable() {
            return Err(ProgramError::Immutable);
        }
        // Mirror the on-chain default tier's borrow-state checks so the
        // host emulation is not *weaker* than the borrow-checked tier it
        // sits above (tier ordering: checked ≥ default > borrow_checked).
        if expected.is_writable {
            actual.check_borrow_mut()?;
        } else {
            actual.check_borrow()?;
        }

        i += 1;
    }

    // BLD-MUT writable hand-off gate — post-loop sweep, mirroring the
    // on-chain tiers' once-per-CPI placement so the host emulation's
    // error surface (including the borrow-before-delegation precedence)
    // stays identical to on-chain.
    if crate::write_policy::lamport_gate_active() {
        let mut m = 0;
        while m < instruction.accounts.len() {
            if instruction.accounts[m].is_writable {
                crate::write_policy::check_lamport_delegation(account_views[m].address())?;
            }
            m += 1;
        }
    }

    validate_no_duplicate_writable(instruction, account_views)
}

#[cfg(not(target_os = "solana"))]
fn emulate_host_system_transfer(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
) -> ProgramResult {
    let amount = u64::from_le_bytes([
        instruction.data[4],
        instruction.data[5],
        instruction.data[6],
        instruction.data[7],
        instruction.data[8],
        instruction.data[9],
        instruction.data[10],
        instruction.data[11],
    ]);
    let from = account_views[0];
    let to = account_views[1];

    // BLD-MUT: pre-validate BOTH sides against the lamport gate before
    // any balance mutation. Relying on the per-account `set_lamports`
    // funnel alone would debit `from` and then have `to` refused at the
    // funnel, destroying lamports in host state on the error path — a
    // transfer must be all-or-nothing.
    crate::write_policy::check_lamport_mutation(from.address())?;
    crate::write_policy::check_lamport_mutation(to.address())?;

    // Self-transfer (same address = same underlying account): net zero.
    // Handled explicitly because the compute-both-then-apply sequence
    // below would otherwise credit from the pre-debit balance and mint
    // `amount` out of thin air.
    if address_eq(from.address(), to.address()) {
        if from.lamports() < amount {
            return Err(ProgramError::InsufficientFunds);
        }
        return Ok(());
    }

    // Compute both post-balances before applying either, so an
    // arithmetic refusal (insufficient funds, overflow) also cannot
    // half-apply the transfer.
    let debited = from
        .lamports()
        .checked_sub(amount)
        .ok_or(ProgramError::InsufficientFunds)?;
    let credited = to
        .lamports()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    from.set_lamports(debited)?;
    to.set_lamports(credited)?;
    Ok(())
}

#[cfg(not(target_os = "solana"))]
fn is_host_system_create_account(instruction: &InstructionView<'_, '_, '_, '_>) -> bool {
    // `CreateAccount { lamports, space, owner }` —
    // `[0u32 LE][lamports: u64 LE][space: u64 LE][owner: 32 bytes]`
    // (52 bytes). See `hopper_system::encoders::encode_create_account`.
    crate::address::address_is_zero(instruction.program_id)
        && instruction.data.len() == 52
        && instruction.data[0..4] == [0, 0, 0, 0]
}

/// Host-only emulation of the System Program's `CreateAccount`.
///
/// `init` / `init_if_needed`'s empty-branch lifecycle (`hopper_init!` in
/// `hopper-macros`) reaches this exact CPI, via
/// [`crate::system::CreateAccount`], to fund + allocate + assign a
/// brand-new account before writing the Hopper layout header into it.
/// Off-chain, the raw syscall wrappers ([`invoke_unchecked`] /
/// [`invoke_signed_unchecked`]) are no-ops by design (there is no runtime
/// to service the syscall) — without this emulation the account is left
/// at its pre-CPI zero-length state and the header write that immediately
/// follows fails with `AccountDataTooSmall`, making every `init` /
/// `init_if_needed` context untestable end-to-end through a host harness.
/// This reproduces the System Program's own observable effect: debit
/// `from`, credit `to`, resize `to` to `space` (zero-filling the new
/// region, mirroring [`AccountView::resize`]'s on-chain growth
/// semantics), and assign `to`'s owner.
#[cfg(not(target_os = "solana"))]
fn emulate_host_system_create_account(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
) -> ProgramResult {
    let lamports = u64::from_le_bytes(instruction.data[4..12].try_into().unwrap());
    let space = u64::from_le_bytes(instruction.data[12..20].try_into().unwrap()) as usize;
    let mut owner_bytes = [0u8; 32];
    owner_bytes.copy_from_slice(&instruction.data[20..52]);
    let owner = Address::new_from_array(owner_bytes);

    let from = account_views[0];
    let to = account_views[1];

    // The System Program refuses to create over an account that already
    // carries lamports or data. `hopper_init!` only issues this CPI once
    // it has already checked `to.data_len() == 0` itself, but the guard
    // is repeated here so a `CreateAccount` CPI built directly (bypassing
    // `hopper_init!`) gets the same off-chain refusal it would get
    // on-chain.
    if to.lamports() != 0 || to.data_len() != 0 {
        return Err(ProgramError::AccountAlreadyInitialized);
    }

    // BLD-MUT: pre-validate BOTH sides against the lamport gate before any
    // balance mutation — see the identical note on
    // `emulate_host_system_transfer`.
    crate::write_policy::check_lamport_mutation(from.address())?;
    crate::write_policy::check_lamport_mutation(to.address())?;

    let debited = from
        .lamports()
        .checked_sub(lamports)
        .ok_or(ProgramError::InsufficientFunds)?;
    let credited = to
        .lamports()
        .checked_add(lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    from.set_lamports(debited)?;
    to.set_lamports(credited)?;

    to.resize(space)?;
    // SAFETY: `to` was validated writable by `validate_host_system_transfer`
    // (the generic meta-check reused above) before this point, and this
    // function stands in for the System Program's own CreateAccount
    // handler — the one caller the real runtime authorizes to assign a
    // fresh (System-owned, empty) account's owner.
    unsafe {
        to.assign(&owner);
    }

    Ok(())
}

#[cfg(not(target_os = "solana"))]
fn is_host_system_allocate(instruction: &InstructionView<'_, '_, '_, '_>) -> bool {
    // `Allocate { space }` — `[8u32 LE][space: u64 LE]` (12 bytes).
    // See `hopper_system::encoders::encode_allocate`.
    crate::address::address_is_zero(instruction.program_id)
        && instruction.data.len() == 12
        && instruction.data[0..4] == [8, 0, 0, 0]
}

/// Host-only emulation of the System Program's `Allocate`.
///
/// `hopper_init!`'s PRE-FUNDED branch (`current_lamports > 0 &&
/// data_len == 0`) reaches this CPI, via [`crate::system::Allocate`],
/// after topping the account up to the rent-exempt minimum with a
/// `Transfer` — instead of `CreateAccount`, which refuses an account
/// that already carries lamports. Off-chain the raw syscall wrappers are
/// no-ops, so without this emulation that branch silently leaves the
/// account at zero length and the header write that follows fails with
/// `AccountDataTooSmall`. This reproduces the System Program's own
/// observable effect: resize the account to `space`, zero-filling the
/// new region (mirroring [`AccountView::resize`]'s on-chain growth
/// semantics). No lamports move in an `Allocate`, so — unlike the
/// Transfer/CreateAccount emulations — there is deliberately no BLD-MUT
/// lamport-mutation precheck here; the shared validator's
/// writable/borrow/delegation sweep is the whole gate, exactly as for
/// the real instruction.
#[cfg(not(target_os = "solana"))]
fn emulate_host_system_allocate(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
) -> ProgramResult {
    let space = u64::from_le_bytes(instruction.data[4..12].try_into().unwrap()) as usize;
    let target = account_views[0];

    // The System Program refuses to allocate an account that already
    // carries data (the "account already in use" class of refusal).
    // `hopper_init!` only issues this CPI once it has already checked
    // `data_len() == 0` itself, but the guard is repeated here so an
    // `Allocate` CPI built directly (bypassing `hopper_init!`) gets the
    // same off-chain refusal it would get on-chain.
    if target.data_len() != 0 {
        return Err(ProgramError::AccountAlreadyInitialized);
    }

    target.resize(space)
}

#[cfg(not(target_os = "solana"))]
fn is_host_system_assign(instruction: &InstructionView<'_, '_, '_, '_>) -> bool {
    // `Assign { owner }` — `[1u32 LE][owner: 32 bytes]` (36 bytes).
    // See `hopper_system::encoders::encode_assign`.
    crate::address::address_is_zero(instruction.program_id)
        && instruction.data.len() == 36
        && instruction.data[0..4] == [1, 0, 0, 0]
}

/// Host-only emulation of the System Program's `Assign`.
///
/// The third leg of `hopper_init!`'s PRE-FUNDED branch (Transfer-shortfall
/// → `Allocate` → `Assign`), via [`crate::system::Assign`] — see
/// [`emulate_host_system_allocate`] for why the branch needs host
/// emulation at all. This reproduces the System Program's own observable
/// effect: set the account's owner. Like the real `Assign`, it moves no
/// lamports, so there is deliberately no BLD-MUT lamport-mutation
/// precheck; the shared validator's writable/borrow/delegation sweep is
/// the whole gate.
#[cfg(not(target_os = "solana"))]
fn emulate_host_system_assign(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
) -> ProgramResult {
    let mut owner_bytes = [0u8; 32];
    owner_bytes.copy_from_slice(&instruction.data[4..36]);
    let owner = Address::new_from_array(owner_bytes);

    let target = account_views[0];

    // SAFETY: `target` was validated writable by
    // `validate_host_system_transfer` (the generic meta-check reused at
    // the dispatch site) before this point, and this function stands in
    // for the System Program's own Assign handler — the one caller the
    // real runtime authorizes to reassign a System-owned account's owner
    // (with the assignee's signature, which the same validator checked
    // against the builder's writable_signer meta).
    unsafe {
        target.assign(&owner);
    }

    Ok(())
}

// ---------------------------------------------------------------------

/// Invoke a CPI with full validation.
#[inline]
pub fn invoke<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
) -> ProgramResult {
    invoke_signed::<ACCOUNTS>(instruction, account_views, &[])
}

/// Invoke a signed CPI with full validation.
#[inline]
pub fn invoke_signed<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    #[cfg(not(target_os = "solana"))]
    if is_host_system_transfer(instruction) {
        validate_host_system_transfer(instruction, &account_views[..], signers_seeds, 2)?;
        return emulate_host_system_transfer(instruction, &account_views[..]);
    }
    #[cfg(not(target_os = "solana"))]
    if is_host_system_create_account(instruction) {
        validate_host_system_transfer(instruction, &account_views[..], signers_seeds, 2)?;
        return emulate_host_system_create_account(instruction, &account_views[..]);
    }
    #[cfg(not(target_os = "solana"))]
    if is_host_system_allocate(instruction) {
        validate_host_system_transfer(instruction, &account_views[..], signers_seeds, 1)?;
        return emulate_host_system_allocate(instruction, &account_views[..]);
    }
    #[cfg(not(target_os = "solana"))]
    if is_host_system_assign(instruction) {
        validate_host_system_transfer(instruction, &account_views[..], signers_seeds, 1)?;
        return emulate_host_system_assign(instruction, &account_views[..]);
    }

    let metas_len = instruction.accounts.len();

    // Fused validate+build (default tier). `check_meta` runs the default
    // tier's per-account contract — address identity, required-signer
    // presence (or supplied PDA authority), writability coverage,
    // and borrow state — in the *same* pass that materializes each
    // `CpiAccount` scratch slot. `post_check` then runs the BLD-MUT
    // lamport-delegation sweep (once per CPI, gate-liveness-guarded; see
    // the note at the sweep) and the duplicate-writable footgun scan,
    // then the syscall.
    dispatch_cpi_fixed::<ACCOUNTS>(
        instruction,
        account_views,
        signers_seeds,
        metas_len,
        |i| {
            let expected = &instruction.accounts[i];
            let actual = account_views[i];

            if !address_eq(actual.address(), expected.address) {
                return Err(ProgramError::InvalidAccountData);
            }

            if expected.is_signer
                && !actual.is_signer()
                && !signer_authority_supplied(signers_seeds)
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

            Ok(())
        },
        || {
            // BLD-MUT: a writable CPI meta delegates unbounded data AND
            // lamport mutation to the callee. The delegation sweep runs
            // ONCE per CPI here (not per meta) behind a liveness branch:
            // keeping gate machinery reachable from the per-meta closure
            // was measured to force spill-heavy codegen costing ~+52 CU
            // per router hop for ungated programs (2026-07-09 bisect).
            // Gated programs are still refused before the syscall.
            if crate::write_policy::lamport_gate_active() {
                let mut i = 0;
                while i < metas_len {
                    if instruction.accounts[i].is_writable {
                        crate::write_policy::check_lamport_delegation(account_views[i].address())?;
                    }
                    i += 1;
                }
            }
            validate_no_duplicate_writable(instruction, &account_views[..])
        },
    )
}

/// Fused validate-and-build for the fixed-array CPI tiers, plus the syscall
/// (a no-op off-chain). Shared tail of the fixed-array invoke tiers.
///
/// Performs ONE pass over the account array: for each meta index `i` in
/// `0..metas_len` it runs the tier-specific per-account check (`check_meta`)
/// AND writes the `CpiAccount` scratch slot in the same iteration, replacing
/// the previous validate-walk-then-build-walk pair. Slots `metas_len..
/// ACCOUNTS` (account infos with no corresponding meta) are build-only, as
/// before. `post_check` runs once after the pass — e.g. the default tier's
/// duplicate-writable scan, which needs the full meta list — and before the
/// syscall.
///
/// Fusing preserves observable behavior exactly: `check_meta` is invoked in
/// ascending meta order, so the first failing meta returns the same error at
/// the same point as the prior split; building a `CpiAccount` has no side
/// effects and `CpiAccount` is `Copy`, so a `?` early-return from
/// `check_meta` or `post_check` discards the never-read `MaybeUninit` scratch
/// with no drop and no observable difference.
///
/// Validation is the **caller's** responsibility via the two closures: every
/// caller must run at least the borrow-state checks over `account_views` (see
/// [`invoke_signed`] and [`invoke_signed_borrow_checked`]), which discharges
/// the `invoke_unchecked` safety contract.
#[inline]
fn dispatch_cpi_fixed<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
    signers_seeds: &[Signer<'_, '_>],
    metas_len: usize,
    check_meta: impl Fn(usize) -> ProgramResult,
    post_check: impl FnOnce() -> ProgramResult,
) -> ProgramResult {
    if ACCOUNTS < metas_len {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let mut cpi_accounts: [MaybeUninit<CpiAccount<'_>>; ACCOUNTS] =
        // SAFETY: an array of `MaybeUninit<T>` is valid in any initialization
        // state, so materializing it uninitialized is sound; every element is
        // written by the loop below before it is read, and on an early
        // `?`-return the array is discarded unread (`CpiAccount` is `Copy`, so
        // no drop runs on the partially-filled scratch).
        unsafe { MaybeUninit::uninit().assume_init() };

    let mut i = 0;
    while i < ACCOUNTS {
        if i < metas_len {
            check_meta(i)?;
        }
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(account_views[i]));
        i += 1;
    }

    post_check()?;

    // SAFETY: the loop above initialized all `ACCOUNTS` elements, and
    // `MaybeUninit<T>` has the same layout as `T`, so reinterpreting the
    // array as `[CpiAccount; ACCOUNTS]` reads only initialized memory.
    let accounts: &[CpiAccount<'_>; ACCOUNTS] =
        unsafe { &*(cpi_accounts.as_ptr() as *const [CpiAccount<'_>; ACCOUNTS]) };

    // SAFETY: `check_meta`/`post_check` validated the borrow state of each
    // account view (writable metas exclusively borrowable, read-only metas
    // shared-borrowable), so no live borrow conflicts with the runtime's
    // access during the CPI — exactly the invariant
    // `invoke_unchecked`/`invoke_signed_unchecked` require.
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
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
) -> ProgramResult {
    invoke_signed_with_bounds::<MAX_ACCOUNTS>(instruction, account_views, &[])
}

/// Signed invoke with a dynamic number of accounts (bounded by const generic).
#[inline]
pub fn invoke_signed_with_bounds<const MAX_ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    if account_views.len() > MAX_ACCOUNTS {
        return Err(ProgramError::InvalidArgument);
    }

    #[cfg(not(target_os = "solana"))]
    if is_host_system_transfer(instruction) {
        validate_host_system_transfer(instruction, account_views, signers_seeds, 2)?;
        return emulate_host_system_transfer(instruction, account_views);
    }

    let metas_len = instruction.accounts.len();
    let count = account_views.len();
    if count < metas_len {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let mut cpi_accounts: [MaybeUninit<CpiAccount<'_>>; MAX_ACCOUNTS] =
        // SAFETY: an array of `MaybeUninit<T>` is valid in any initialization
        // state; the first `count` slots are written before being read below,
        // and on an early `?`-return the array is discarded unread
        // (`CpiAccount` is `Copy`, so no drop runs on the partial scratch).
        unsafe { MaybeUninit::uninit().assume_init() };

    // Fused validate+build (default tier, dynamic): one pass runs the default
    // per-account contract for each meta AND writes its scratch slot; slots
    // `metas_len..count` are build-only. The duplicate-writable scan runs
    // afterward, exactly as `validate_cpi_accounts` ordered it.
    let mut i = 0;
    while i < count {
        let actual = account_views[i];
        if i < metas_len {
            let expected = &instruction.accounts[i];

            if !address_eq(actual.address(), expected.address) {
                return Err(ProgramError::InvalidAccountData);
            }

            if expected.is_signer
                && !actual.is_signer()
                && !signer_authority_supplied(signers_seeds)
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
        }
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(actual));
        i += 1;
    }

    // BLD-MUT writable hand-off gate, swept once per CPI behind the
    // liveness branch (never reachable from the hot per-meta loop; see
    // the 2026-07-09 bisect note in `invoke_signed`'s sweep).
    if crate::write_policy::lamport_gate_active() {
        let mut m = 0;
        while m < instruction.accounts.len() {
            if instruction.accounts[m].is_writable {
                crate::write_policy::check_lamport_delegation(account_views[m].address())?;
            }
            m += 1;
        }
    }

    validate_no_duplicate_writable(instruction, account_views)?;

    // SAFETY: the loop above initialized the first `count` slots, and
    // `MaybeUninit<T>` shares `T`'s layout, so reading exactly that prefix
    // reads only initialized memory.
    let accounts = unsafe {
        core::slice::from_raw_parts(cpi_accounts.as_ptr() as *const CpiAccount<'_>, count)
    };

    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts)
        } else {
            invoke_signed_unchecked(instruction, accounts, signers_seeds)
        }
    }
}

// -- SIMD-0339 dedup-aware path ---------------------------------------

/// Locate the deduplicated info that carries `address` (linear scan).
#[inline]
fn find_info(infos: &[&AccountView<'_>], address: &Address) -> Option<usize> {
    let mut i = 0;
    while i < infos.len() {
        if address_eq(infos[i].address(), address) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Validate metas against a **deduplicated** info set (matched by pubkey).
///
/// Unlike the default tier's positional validation, `infos` is *not*
/// positionally aligned
/// with `instruction.accounts`: it holds exactly one [`AccountView`] per
/// unique address. Each meta is resolved to its info by address. Signer
/// presence (or supplied PDA authority), writability coverage,
/// per-account borrow state, and the duplicate-writable footgun are all
/// enforced over the full (un-deduplicated) meta list, so collapsing the
/// info list never weakens what the default tier checks.
#[inline]
fn validate_cpi_accounts_deduped(
    instruction: &InstructionView<'_, '_, '_, '_>,
    infos: &[&AccountView<'_>],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    // Duplicate-writable footgun: two writable metas naming one account.
    // The infos are deduped, so `validate_no_duplicate_writable`'s
    // view-pair scan cannot observe it — check meta addresses directly.
    let mut i = 0;
    while i < instruction.accounts.len() {
        if instruction.accounts[i].is_writable {
            let mut j = i + 1;
            while j < instruction.accounts.len() {
                if instruction.accounts[j].is_writable
                    && address_eq(
                        instruction.accounts[i].address,
                        instruction.accounts[j].address,
                    )
                {
                    return Err(ProgramError::AccountBorrowFailed);
                }
                j += 1;
            }
        }
        i += 1;
    }

    let mut i = 0;
    while i < instruction.accounts.len() {
        let expected = &instruction.accounts[i];
        // Resolve this meta to its unique account-info by pubkey. A meta
        // whose account was never supplied as an info is a malformed CPI.
        let info = match find_info(infos, expected.address) {
            Some(idx) => infos[idx],
            None => return Err(ProgramError::NotEnoughAccountKeys),
        };

        if expected.is_signer && !info.is_signer() && !signer_authority_supplied(signers_seeds) {
            return Err(ProgramError::MissingRequiredSignature);
        }
        if expected.is_writable && !info.is_writable() {
            return Err(ProgramError::Immutable);
        }
        // Borrow state is checked per meta; `check_borrow`/`check_borrow_mut`
        // only *inspect* the borrow flag (they do not acquire), so resolving
        // several metas to the same info and checking each is sound. A
        // writable meta demands exclusive borrowability of that one info,
        // which is exactly the OR-merged requirement dedup must preserve.
        if expected.is_writable {
            info.check_borrow_mut()?;
        } else {
            info.check_borrow()?;
        }
        i += 1;
    }

    // BLD-MUT writable hand-off gate over the FULL un-deduplicated meta
    // list (dedup collapses infos, never the delegation requirement),
    // swept once per CPI behind the liveness branch — never reachable
    // from the per-meta loop (2026-07-09 bisect; see invoke_signed).
    if crate::write_policy::lamport_gate_active() {
        let mut m = 0;
        while m < instruction.accounts.len() {
            let expected = &instruction.accounts[m];
            if expected.is_writable {
                if let Some(idx) = find_info(infos, expected.address) {
                    crate::write_policy::check_lamport_delegation(infos[idx].address())?;
                }
            }
            m += 1;
        }
    }

    Ok(())
}

/// Invoke a CPI whose account-info list has been **deduplicated by pubkey**
/// — the SIMD-0339 fewest-infos-per-CPI optimization.
///
/// `instruction.accounts` (the metas) may reference the same account in
/// several positions and the callee still sees that full ordered list.
/// `infos`, by contrast, holds exactly one [`AccountView`] per unique
/// address. Because the SVM resolves account-infos to metas by pubkey, N
/// metas of one account need only ONE info; under SIMD-0339 every distinct
/// info also costs CU, so collapsing them is a measurable saving that a
/// naive one-info-per-meta builder cannot claim.
///
/// `infos.len()` must be `<= MAX_INFOS` (the deduped list is what is handed
/// to the syscall). Validation runs over the full, un-deduplicated meta
/// list via [`validate_cpi_accounts_deduped`], so this path is exactly as
/// strict as the default [`invoke_signed`] tier.
#[inline]
pub fn invoke_signed_deduped<const MAX_INFOS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    infos: &[&AccountView<'_>],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    if infos.len() > MAX_INFOS {
        return Err(ProgramError::InvalidArgument);
    }

    #[cfg(not(target_os = "solana"))]
    if is_host_system_transfer(instruction) {
        validate_cpi_accounts_deduped(instruction, infos, signers_seeds)?;
        // A System transfer names two distinct accounts (from, to); the
        // deduped info list preserves them at positions 0 and 1 because
        // dedup keeps first-occurrence (i.e. push/meta) order.
        if infos.len() < 2 {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        return emulate_host_system_transfer(instruction, infos);
    }

    validate_cpi_accounts_deduped(instruction, infos, signers_seeds)?;

    let mut cpi_accounts: [MaybeUninit<CpiAccount<'_>>; MAX_INFOS] =
        // SAFETY: an array of `MaybeUninit<T>` is valid in any initialization
        // state, so materializing it uninitialized is sound; the first
        // `count` elements are written below before they are read.
        unsafe { MaybeUninit::uninit().assume_init() };

    let count = infos.len();
    let mut i = 0;
    while i < count {
        cpi_accounts[i] = MaybeUninit::new(CpiAccount::from(infos[i]));
        i += 1;
    }

    // SAFETY: the loop initialized the first `count` elements, and
    // `MaybeUninit<T>` shares `T`'s layout, so reading exactly that prefix
    // reads only initialized memory.
    let accounts = unsafe {
        core::slice::from_raw_parts(cpi_accounts.as_ptr() as *const CpiAccount<'_>, count)
    };

    // SAFETY: `validate_cpi_accounts_deduped` above discharged the borrow /
    // aliasing contract (writable infos exclusively borrowable, read-only
    // infos shared-borrowable) required by the unchecked syscall wrappers.
    unsafe {
        if signers_seeds.is_empty() {
            invoke_unchecked(instruction, accounts)
        } else {
            invoke_signed_unchecked(instruction, accounts, signers_seeds)
        }
    }
}

/// Explicit alias for Hopper's validated CPI path.
#[inline]
pub fn invoke_checked<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
) -> ProgramResult {
    invoke::<ACCOUNTS>(instruction, account_views)
}

/// Explicit alias for Hopper's validated signed CPI path.
#[inline]
pub fn invoke_signed_checked<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    invoke_signed::<ACCOUNTS>(instruction, account_views, signers_seeds)
}

// -- Borrow-checked (Pinocchio-equivalent) tier -------------------------

/// Invoke a CPI with **borrow-state validation only** — the
/// Pinocchio-equivalent mid tier.
///
/// # Validation tiers
///
/// From most to least validation (and CU cost):
///
/// | Tier | Functions | Validates before the syscall |
/// |------|-----------|------------------------------|
/// | checked | [`invoke_checked`] / [`invoke_signed_checked`] | Explicit-by-name aliases of the default tier (same checks). |
/// | default | [`invoke`] / [`invoke_signed`] / [`invoke_with_bounds`] / [`invoke_signed_with_bounds`] | Meta↔view address match, required transaction signer or supplied PDA authority, meta writability vs. account writability, per-account borrow state, **and** duplicate-writable rejection. The SVM syscall authoritatively derives and matches PDA signers with the caller id. |
/// | borrow_checked | `invoke_borrow_checked` / [`invoke_signed_borrow_checked`] | Per-account borrow state only: writable metas must be exclusively borrowable, read-only metas shared-borrowable. |
/// | unchecked | [`invoke_unchecked`] / [`invoke_signed_unchecked`] (`unsafe`) | Nothing. |
///
/// Every **safe** tier additionally consults the BLD-MUT lamport gate
/// on writable metas: under a `strict_writes` context that declared its
/// lamport dimension (`lamports(...)`), handing an account to a callee
/// as writable requires that account to carry a whole-account data
/// grant *and* lamport permission. Instructions outside the feature pay
/// one `None`-check. The `unsafe` unchecked tier remains ungated (it is
/// the documented escape hatch and validates nothing).
///
/// # What this tier is
///
/// This tier performs exactly the per-account borrow-state checks that
/// Pinocchio's `invoke` performs before its syscall — nothing more. It
/// skips the default tier's meta↔view address comparison, signer/PDA
/// matching, the writability re-check, and the O(n²) pairwise
/// duplicate-writable scan, which together cost roughly 9–13 extra
/// instructions per CPI at instruction level (measured 2026-07-07).
/// `borrow_checked` therefore matches the CU cost of a hand-written
/// Pinocchio `invoke` while remaining a safe (non-`unsafe`) API,
/// because the borrow checks are precisely what discharge the
/// runtime's aliasing contract.
///
/// # When it is appropriate
///
/// Use this tier when the accounts were already validated at parse
/// time — the entrypoint/context layer has checked addresses and
/// writability, so re-checking per CPI buys nothing — i.e. when you
/// want the exact validation level of a raw Pinocchio program.
///
/// The default tier's duplicate-writable rejection guards a real
/// Sealevel footgun (two writable metas aliasing one account let a
/// callee double-mutate state behind your back) and is deliberately
/// **not** weakened or removed. Wide-CPI callers who have already run
/// `require_unique_writable_accounts` (the check-layer graph
/// constraint) — or whose account shape statically precludes duplicate
/// writables — can safely opt down to `borrow_checked`.
///
/// Off-chain (host builds) the syscall is a no-op; validation still
/// runs, and host-side System-program transfers are emulated the same
/// way the default tier emulates them.
#[inline]
pub fn invoke_borrow_checked<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
) -> ProgramResult {
    invoke_signed_borrow_checked::<ACCOUNTS>(instruction, account_views, &[])
}

/// Invoke a signed CPI with **borrow-state validation only** — the
/// Pinocchio-equivalent mid tier.
///
/// See [`invoke_borrow_checked`] for the full tier table, what this
/// tier validates (and deliberately does not), and when opting down
/// from the default tier is appropriate. `signers_seeds` are passed
/// straight through to the syscall; unlike [`invoke_signed`], no
/// required-signer/PDA-authority preflight is performed before the syscall.
#[inline]
pub fn invoke_signed_borrow_checked<const ACCOUNTS: usize>(
    instruction: &InstructionView<'_, '_, '_, '_>,
    account_views: &[&AccountView<'_>; ACCOUNTS],
    signers_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    #[cfg(not(target_os = "solana"))]
    if is_host_system_transfer(instruction) {
        // The emulation reads views[0] and views[1] directly; guard the
        // fixed-array length before indexing (ACCOUNTS may be < 2).
        if account_views.len() < 2 {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        validate_cpi_borrows(instruction, &account_views[..])?;
        return emulate_host_system_transfer(instruction, &account_views[..]);
    }

    let metas_len = instruction.accounts.len();

    // Fused validate+build (borrow_checked tier). `check_meta` runs the
    // per-account checks `validate_cpi_borrows` did — meta↔view address
    // correspondence and borrow state — while the scratch slot is
    // materialized in the same pass. The BLD-MUT lamport-delegation scan
    // runs ONCE per CPI in `post_check`, NOT per meta: the 2026-07-09
    // router bisect measured that any *reachable* gate-machinery call
    // inside this per-meta closure forces it into an outlined,
    // spill-heavy shape costing ~+52 CU per hop for programs that never
    // installed a gate (branch-inside variants only recovered to ~+21;
    // machinery-unreachable-from-the-closure recovered fully:
    // 1,564/3,044/4,525 → 1,559/3,035/4,512 measured). Gated programs
    // keep full enforcement — the sweep still refuses before the syscall
    // hand-off in `dispatch_cpi_fixed` — with one documented precedence
    // shift: in a multi-fault instruction, borrow errors now surface
    // before delegation errors (both are pre-syscall refusals).
    dispatch_cpi_fixed::<ACCOUNTS>(
        instruction,
        account_views,
        signers_seeds,
        metas_len,
        |i| {
            // The borrow state must be validated against the account the meta
            // actually names, not whatever view happens to sit at index `i`
            // (see `validate_cpi_borrows` for why: a mismatched order would
            // borrow-check the wrong (account, mutability) pair and reach
            // `invoke_unchecked` with its aliasing contract undischarged).
            if !address_eq(account_views[i].address(), instruction.accounts[i].address) {
                return Err(ProgramError::InvalidArgument);
            }
            if instruction.accounts[i].is_writable {
                account_views[i].check_borrow_mut()?;
            } else {
                account_views[i].check_borrow()?;
            }
            Ok(())
        },
        || {
            if crate::write_policy::lamport_gate_active() {
                let mut i = 0;
                while i < metas_len {
                    if instruction.accounts[i].is_writable {
                        crate::write_policy::check_lamport_delegation(account_views[i].address())?;
                    }
                    i += 1;
                }
            }
            Ok(())
        },
    )
}

// ---------------------------------------------------------------------

/// Set return data for the current instruction.
#[inline(always)]
pub fn set_return_data(data: &[u8]) {
    crate::return_data::set_return_data(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::InstructionAccount;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make_account(address: [u8; 32]) -> (std::vec::Vec<u64>, AccountView<'static>) {
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + 16).div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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

    // -- borrow_checked tier ------------------------------------------

    #[test]
    fn borrow_checked_rejects_live_mutable_data_borrow() {
        let (_backing, account) = make_account([21; 32]);
        let metas = [InstructionAccount::writable(account.address())];
        let program_id = Address::new_from_array([7; 32]);
        let instruction = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas,
        };

        let guard = account.try_borrow_mut().unwrap();
        let err = invoke_borrow_checked::<1>(&instruction, &[&account]).unwrap_err();
        assert_eq!(err, ProgramError::AccountBorrowFailed);
        drop(guard);
    }

    #[test]
    fn borrow_checked_succeeds_after_borrow_release() {
        let (_backing, account) = make_account([22; 32]);
        let metas = [InstructionAccount::writable(account.address())];
        let program_id = Address::new_from_array([7; 32]);
        let instruction = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas,
        };

        let guard = account.try_borrow_mut().unwrap();
        assert!(invoke_borrow_checked::<1>(&instruction, &[&account]).is_err());
        drop(guard);

        // Off-chain the syscall is a no-op, so Ok(()) here proves the
        // borrow validation passed once the guard was released.
        invoke_borrow_checked::<1>(&instruction, &[&account]).unwrap();
    }

    #[test]
    fn borrow_checked_permits_duplicate_writable_metas_unlike_default_tier() {
        let (_first_backing, first) = make_account([23; 32]);
        let (_second_backing, second) = make_account([23; 32]);

        let metas = [
            InstructionAccount::writable(first.address()),
            InstructionAccount::writable(second.address()),
        ];
        let program_id = Address::new_from_array([7; 32]);
        let instruction = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas,
        };

        // Default tier: duplicate writable metas are rejected — the
        // Sealevel double-mutation footgun `validate_no_duplicate_writable`
        // exists to guard.
        let err = invoke::<2>(&instruction, &[&first, &second]).unwrap_err();
        assert_eq!(err, ProgramError::AccountBorrowFailed);

        // borrow_checked tier: per-account borrow state ONLY, matching
        // what Pinocchio's `invoke` checks. Not rejecting duplicates is
        // the documented contract of this tier — callers opt down only
        // after `require_unique_writable_accounts` (or a statically
        // duplicate-free account shape) has ruled the footgun out.
        invoke_borrow_checked::<2>(&instruction, &[&first, &second]).unwrap();
    }

    #[test]
    fn borrow_checked_offchain_noop_path_returns_ok() {
        let (_backing, account) = make_account([24; 32]);
        let metas = [InstructionAccount::readonly(account.address())];
        let program_id = Address::new_from_array([7; 32]);
        let instruction = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas,
        };

        assert_eq!(
            invoke_borrow_checked::<1>(&instruction, &[&account]),
            Ok(())
        );
        assert_eq!(
            invoke_signed_borrow_checked::<1>(&instruction, &[&account], &[]),
            Ok(())
        );
    }

    // -- BLD-MUT lamport gate on writable metas -------------------------

    // Guarded-tier semantics: installs a data-declaring policy, which the
    // `unguarded-raw-surfaces` fence refuses at install (covered by its
    // own explicit test in that shape).
    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn writable_meta_is_refused_unless_both_dimensions_are_declared() {
        use crate::write_policy::{
            install_lamport_gate, write_policy_violation, WritePolicy, WriteRange,
        };

        let (_b0, delegable) = make_account([31; 32]);
        let (_b1, lamports_only) = make_account([32; 32]);
        let (_b2, undeclared) = make_account([33; 32]);
        let accounts = [delegable, lamports_only, undeclared];

        // Account 0 carries whole-account data + lamports (delegable);
        // account 1 lamports only; account 2 nothing.
        static P: WritePolicy =
            WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0, 1]);
        let _gate = install_lamport_gate(&accounts, &P);

        let program_id = Address::new_from_array([7; 32]);

        // Writable meta on the fully declared account: allowed on the
        // default AND borrow_checked tiers (off-chain no-op syscall).
        let metas0 = [InstructionAccount::writable(accounts[0].address())];
        let ix0 = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas0,
        };
        invoke::<1>(&ix0, &[&accounts[0]]).unwrap();
        invoke_borrow_checked::<1>(&ix0, &[&accounts[0]]).unwrap();

        // Lamports-only account: a writable hand-off is unbounded DATA
        // delegation too, so it is refused with the indexed policy error.
        let metas1 = [InstructionAccount::writable(accounts[1].address())];
        let ix1 = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas1,
        };
        assert_eq!(
            invoke::<1>(&ix1, &[&accounts[1]]).unwrap_err(),
            write_policy_violation(1)
        );
        assert_eq!(
            invoke_borrow_checked::<1>(&ix1, &[&accounts[1]]).unwrap_err(),
            write_policy_violation(1)
        );

        // Entirely undeclared account: refused on every safe tier,
        // including the deduped path.
        let metas2 = [InstructionAccount::writable(accounts[2].address())];
        let ix2 = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas2,
        };
        assert_eq!(
            invoke_signed_deduped::<1>(&ix2, &[&accounts[2]], &[]).unwrap_err(),
            write_policy_violation(2)
        );

        // Read-only metas are never lamport-gated.
        let metas_ro = [InstructionAccount::readonly(accounts[2].address())];
        let ix_ro = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas_ro,
        };
        invoke::<1>(&ix_ro, &[&accounts[2]]).unwrap();
    }

    // Guarded-tier semantics: installs a data-declaring policy, which the
    // `unguarded-raw-surfaces` fence refuses at install (covered by its
    // own explicit test in that shape).
    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn host_system_transfer_is_gated_through_the_lamport_funnel() {
        use crate::write_policy::{
            install_lamport_gate, write_policy_violation, WritePolicy, WriteRange,
        };

        let (_b0, from) = make_account([41; 32]);
        let (_b1, to) = make_account([42; 32]);
        let accounts = [from, to];

        // Both sides declared: the emulated transfer succeeds and the
        // balances actually move.
        static OPEN: WritePolicy = WritePolicy::with_lamports(
            &[WriteRange::whole_account(0), WriteRange::whole_account(1)],
            &[0, 1],
        );
        // Only `from` declared: the transfer must be refused before any
        // balance changes.
        static HALF: WritePolicy =
            WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0]);

        let system_id = Address::new_from_array([0; 32]);
        let mut data = [0u8; 12];
        data[0] = 2; // System Transfer tag
        data[4..12].copy_from_slice(&1u64.to_le_bytes());
        let metas = [
            InstructionAccount::writable(accounts[0].address()),
            InstructionAccount::writable(accounts[1].address()),
        ];
        let ix = InstructionView {
            program_id: &system_id,
            data: &data,
            accounts: &metas,
        };

        {
            let _gate = install_lamport_gate(&accounts, &OPEN);
            invoke::<2>(&ix, &[&accounts[0], &accounts[1]]).unwrap();
            assert_eq!(accounts[0].lamports(), 0);
            assert_eq!(accounts[1].lamports(), 2);
        }
        {
            let _gate = install_lamport_gate(&accounts, &HALF);
            assert_eq!(
                invoke::<2>(&ix, &[&accounts[0], &accounts[1]]).unwrap_err(),
                write_policy_violation(1)
            );
            // Refused before mutation: balances unchanged.
            assert_eq!(accounts[0].lamports(), 0);
            assert_eq!(accounts[1].lamports(), 2);
        }
    }

    // Guarded-tier semantics: installs a data-declaring policy, which the
    // `unguarded-raw-surfaces` fence refuses at install (covered by its
    // own explicit test in that shape).
    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn host_system_transfer_refusal_leaves_both_balances_untouched() {
        use crate::write_policy::{
            install_lamport_gate, write_policy_violation, WritePolicy, WriteRange,
        };

        let (_b0, from) = make_account([43; 32]);
        let (_b1, to) = make_account([44; 32]);
        let accounts = [from, to];

        // Only `from` is declared for lamport mutation.
        static HALF: WritePolicy =
            WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0]);
        let _gate = install_lamport_gate(&accounts, &HALF);

        let system_id = Address::new_from_array([0; 32]);
        let mut data = [0u8; 12];
        data[0] = 2; // System Transfer tag
        data[4..12].copy_from_slice(&1u64.to_le_bytes());
        // `to` is deliberately a READ-ONLY meta: the writable-meta
        // delegation gate then never fires for it, so without the
        // emulation's own both-sides pre-validation the refusal would
        // come from the `set_lamports` funnel *after* `from` was
        // already debited — destroying a lamport in host state.
        let metas = [
            InstructionAccount::writable(accounts[0].address()),
            InstructionAccount::readonly(accounts[1].address()),
        ];
        let ix = InstructionView {
            program_id: &system_id,
            data: &data,
            accounts: &metas,
        };

        assert_eq!(
            invoke_borrow_checked::<2>(&ix, &[&accounts[0], &accounts[1]]).unwrap_err(),
            write_policy_violation(1)
        );
        // Refused BEFORE any mutation: neither side moved (make_account
        // seeds each balance with 1 lamport).
        assert_eq!(accounts[0].lamports(), 1);
        assert_eq!(accounts[1].lamports(), 1);
    }

    #[test]
    fn borrow_checked_requires_enough_account_views() {
        let (_first_backing, first) = make_account([25; 32]);
        let (_second_backing, second) = make_account([26; 32]);

        let metas = [
            InstructionAccount::writable(first.address()),
            InstructionAccount::writable(second.address()),
        ];
        let program_id = Address::new_from_array([7; 32]);
        let instruction = InstructionView {
            program_id: &program_id,
            data: &[0u8],
            accounts: &metas,
        };

        let err = invoke_borrow_checked::<1>(&instruction, &[&first]).unwrap_err();
        assert_eq!(err, ProgramError::NotEnoughAccountKeys);
    }

    // -- FUSED-CPI: fused validate+build == prior validate-then-build ------

    /// Serialize the built `CpiAccount` scratch to a stable string. The
    /// production fused path writes `CpiAccount::from(view)` into each slot;
    /// its `Debug` (pointers + flags + lengths) is a faithful fingerprint of
    /// the scratch handed to the syscall.
    fn scratch_fingerprint(account_views: &[&AccountView<'_>]) -> std::string::String {
        let mut s = std::string::String::new();
        let mut i = 0;
        while i < account_views.len() {
            s.push_str(&std::format!(
                "[{}]={:?};",
                i,
                CpiAccount::from(account_views[i])
            ));
            i += 1;
        }
        s
    }

    /// PRE-fusion default tier: validate the *whole* meta list, THEN build
    /// the scratch in a second walk. Kept in the test as the byte-for-byte
    /// oracle the production fused path must match.
    fn reference_split_default(
        instruction: &InstructionView<'_, '_, '_, '_>,
        account_views: &[&AccountView<'_>],
        signers_seeds: &[Signer<'_, '_>],
    ) -> Result<std::string::String, ProgramError> {
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
                && !signer_authority_supplied(signers_seeds)
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
        // Mirrors production: the delegation sweep runs once per CPI
        // after the per-meta pass (borrow-before-delegation precedence).
        if crate::write_policy::lamport_gate_active() {
            let mut m = 0;
            while m < instruction.accounts.len() {
                if instruction.accounts[m].is_writable {
                    crate::write_policy::check_lamport_delegation(account_views[m].address())?;
                }
                m += 1;
            }
        }
        validate_no_duplicate_writable(instruction, account_views)?;
        // Second (build) walk over the FULL view list.
        Ok(scratch_fingerprint(account_views))
    }

    /// The fused default tier reproduced exactly as production `invoke_signed`
    /// runs it: interleave per-meta validation with the scratch build, then
    /// run the duplicate-writable scan.
    fn reference_fused_default(
        instruction: &InstructionView<'_, '_, '_, '_>,
        account_views: &[&AccountView<'_>],
        signers_seeds: &[Signer<'_, '_>],
    ) -> Result<std::string::String, ProgramError> {
        let metas_len = instruction.accounts.len();
        if account_views.len() < metas_len {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let mut s = std::string::String::new();
        let mut i = 0;
        while i < account_views.len() {
            let actual = account_views[i];
            if i < metas_len {
                let expected = &instruction.accounts[i];
                if !address_eq(actual.address(), expected.address) {
                    return Err(ProgramError::InvalidAccountData);
                }
                if expected.is_signer
                    && !actual.is_signer()
                    && !signer_authority_supplied(signers_seeds)
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
            }
            s.push_str(&std::format!("[{}]={:?};", i, CpiAccount::from(actual)));
            i += 1;
        }
        // Mirrors production's once-per-CPI delegation sweep placement.
        if crate::write_policy::lamport_gate_active() {
            let mut m = 0;
            while m < metas_len {
                if instruction.accounts[m].is_writable {
                    crate::write_policy::check_lamport_delegation(account_views[m].address())?;
                }
                m += 1;
            }
        }
        validate_no_duplicate_writable(instruction, account_views)?;
        Ok(s)
    }

    #[test]
    fn signed_preflight_defers_pda_derivation_to_the_svm() {
        let (_backing, account) = make_account([50; 32]);
        let callee = Address::new_from_array([7; 32]);
        let metas = [InstructionAccount::readonly_signer(account.address())];
        let instruction = InstructionView {
            program_id: &callee,
            data: &[0u8],
            accounts: &metas,
        };
        let views = [&account];
        let seed_bytes = [9u8];
        let seeds = [Seed::from(&seed_bytes)];
        let signers = [Signer::from(&seeds)];

        // The callee id is not the caller id and therefore cannot be used to
        // derive the PDA here. Host invocation is a no-op after preflight;
        // on SVM the invoke_signed syscall validates the same seed group
        // against the actual caller before granting signer privilege.
        assert_eq!(invoke_signed(&instruction, &views, &signers), Ok(()));
        assert_eq!(
            invoke_signed(&instruction, &views, &[]),
            Err(ProgramError::MissingRequiredSignature)
        );
    }

    #[test]
    fn fused_build_matches_split_build_and_per_tier_errors() {
        use crate::write_policy::{install_lamport_gate, write_policy_violation, WritePolicy};

        let program_id = Address::new_from_array([7; 32]);

        // (1) Valid multi-account CPI (two distinct writable accounts, no
        //     gate installed). Fused and split builds must produce the SAME
        //     scratch, and production `invoke` must accept it.
        {
            let (_a, first) = make_account([51; 32]);
            let (_b, second) = make_account([52; 32]);
            let metas = [
                InstructionAccount::writable(first.address()),
                InstructionAccount::writable(second.address()),
            ];
            let ix = InstructionView {
                program_id: &program_id,
                data: &[0u8],
                accounts: &metas,
            };
            let views: [&AccountView<'_>; 2] = [&first, &second];

            let split = reference_split_default(&ix, &views[..], &[]);
            let fused = reference_fused_default(&ix, &views[..], &[]);
            assert!(split.is_ok());
            // Same scratch bytes, and same Result overall.
            assert_eq!(split, fused);
            // Production fused path accepts the valid CPI (off-chain no-op).
            assert_eq!(invoke::<2>(&ix, &views), Ok(()));
        }

        // (2) Signer-missing meta: a required-signer meta over a non-signer
        //     account. Both builds refuse identically, and production too.
        {
            let (_a, acct) = make_account([53; 32]);
            let metas = [InstructionAccount::readonly_signer(acct.address())];
            let ix = InstructionView {
                program_id: &program_id,
                data: &[0u8],
                accounts: &metas,
            };
            let views: [&AccountView<'_>; 1] = [&acct];

            let split = reference_split_default(&ix, &views[..], &[]);
            let fused = reference_fused_default(&ix, &views[..], &[]);
            assert_eq!(split, Err(ProgramError::MissingRequiredSignature));
            assert_eq!(split, fused);
            assert_eq!(
                invoke::<1>(&ix, &views).unwrap_err(),
                ProgramError::MissingRequiredSignature
            );
        }

        // (3) Writable-meta lamport-delegation refusal: an installed gate
        //     that declares nothing for the account. The refusal must fire on
        //     the fused build exactly as on the split build (indexed policy
        //     error), and production must surface the same error.
        {
            let (_a, acct) = make_account([54; 32]);
            let accounts = [acct];
            static P: WritePolicy = WritePolicy::with_lamports(&[], &[]);
            let _gate = install_lamport_gate(&accounts, &P);

            let metas = [InstructionAccount::writable(accounts[0].address())];
            let ix = InstructionView {
                program_id: &program_id,
                data: &[0u8],
                accounts: &metas,
            };
            let views: [&AccountView<'_>; 1] = [&accounts[0]];

            let split = reference_split_default(&ix, &views[..], &[]);
            let fused = reference_fused_default(&ix, &views[..], &[]);
            assert_eq!(split, Err(write_policy_violation(0)));
            assert_eq!(split, fused);
            assert_eq!(
                invoke::<1>(&ix, &views).unwrap_err(),
                write_policy_violation(0)
            );
        }

        // (4) Deduped (duplicate account) case: two writable metas naming the
        //     SAME account. The deduped tier (unchanged by fusion) must still
        //     reject the double-mutation footgun.
        {
            let (_a, acct) = make_account([55; 32]);
            let metas = [
                InstructionAccount::writable(acct.address()),
                InstructionAccount::writable(acct.address()),
            ];
            let ix = InstructionView {
                program_id: &program_id,
                data: &[0u8],
                accounts: &metas,
            };
            // A single deduped info backs both metas.
            assert_eq!(
                invoke_signed_deduped::<1>(&ix, &[&acct], &[]).unwrap_err(),
                ProgramError::AccountBorrowFailed
            );
        }
    }
}
