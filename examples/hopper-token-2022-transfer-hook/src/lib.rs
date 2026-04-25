//! # Hopper Token-2022 Transfer Hook Example
//!
//! R6 audit closure. The Token-2022 transfer-hook extension lets a
//! mint register an arbitrary program that is invoked on every
//! transfer. It is both a powerful composability primitive and a
//! significant attack surface: protocols that accept Token-2022
//! mints without inspecting the hook binding are trusting code they
//! have not audited.
//!
//! This example demonstrates the **extension-aware validation
//! pattern** that `hopper-token-2022` + `hopper-solana` enable. The
//! program:
//!
//! 1. Stores a `HookedVault` state account that declares the *expected*
//!    hook program ID for a given mint.
//! 2. Exposes a `verify_hook_binding` instruction that reads the
//!    mint's TransferHook extension and rejects the call if the
//!    binding does not match the declared expectation. This is the
//!    pattern a DEX or lending market would run before accepting a
//!    deposit of hooked tokens.
//! 3. Exposes a `require_safe_mint` instruction that uses
//!    `check_safe_token_2022_mint` to reject any mint with exotic
//!    extensions (transfer fee, permanent delegate, confidential
//!    transfer, non-transferable, transfer hook). This is the
//!    pattern an AMM pool would use to guard against exotic mints
//!    even slipping into a pool whose math assumes vanilla SPL
//!    semantics.
//! 4. Exposes a `rotate_expected_hook` instruction so the authority
//!    can update the declared hook binding when a mint legitimately
//!    migrates to a new hook program. Gated on the vault's recorded
//!    authority.
//!
//! The example deliberately does **not** invoke Token-2022's
//! `TransferChecked`. Executing a transfer through a hooked mint
//! requires the caller to supply the hook's `ExtraAccountMetaList`
//! accounts in the transaction, which is a client-side concern
//! orthogonal to Hopper's framework layer. See
//! [`hopper-token-2022-vault`](../hopper-token-2022-vault/src/lib.rs)
//! for the vanilla transfer flow; combining the two is a
//! straightforward composition the client does, not the program.
//!
//! ## Instruction map
//!
//! | Disc | Name                   | Accounts |
//! |-----:|------------------------|----------|
//! | 0    | `init_hooked_vault`    | authority (signer, mut), vault (mut, program-owned), mint (read) |
//! | 1    | `verify_hook_binding`  | vault (read), mint (read) |
//! | 2    | `require_safe_mint`    | mint (read) |
//! | 3    | `rotate_expected_hook` | authority (signer), vault (mut), mint (read) |

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;
use hopper::hopper_token_2022::{
    check_safe_token_2022_mint,
    check_transfer_hook_program,
    read_transfer_hook,
};

#[cfg(target_os = "solana")]
mod __sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
fast_entrypoint!(process_instruction, 3);

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Authority;
#[derive(Clone, Copy)]
pub struct Mint;
#[derive(Clone, Copy)]
pub struct Program;

/// Vault state record. Pinned to a Token-2022 mint, declares the
/// expected transfer-hook program ID, and gates updates on a stored
/// authority. The proc-macro form is used here because the example
/// relies on the per-field `*_ABS_OFFSET` constants the macro emits.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct HookedVault {
    /// Authority permitted to rotate the expected hook program.
    pub authority: TypedAddress<Authority>,
    /// Mint this vault is bound to.
    pub mint: TypedAddress<Mint>,
    /// The hook program ID the authority expects to see bound to
    /// `mint`'s TransferHook extension. Verified on every
    /// `verify_hook_binding` instruction.
    pub expected_hook_program: TypedAddress<Program>,
    /// Bump byte for the vault PDA.
    pub bump: u8,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (disc, _rest) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match *disc {
        0 => process_init(program_id, accounts),
        1 => process_verify_hook_binding(program_id, accounts),
        2 => process_require_safe_mint(accounts),
        3 => process_rotate_expected_hook(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

// ---------------------------------------------------------------------------
// 0. init_hooked_vault
// ---------------------------------------------------------------------------
//
// Writes the 16-byte Hopper header + the HookedVault body. Assumes the
// vault account was already allocated (via system-program CreateAccount
// or equivalent) by the caller — this example keeps focus on the
// Token-2022 validation path, not on initialization CPI choreography.
fn process_init(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, vault, mint, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    authority.require_signer()?;
    vault.require_writable()?;
    vault.require_owned_by(program_id)?;

    // Write the 16-byte Hopper header.
    {
        let mut data = vault.try_borrow_mut()?;
        hopper::hopper_runtime::layout::init_header::<HookedVault>(&mut data)?;
    }

    // Populate the body. Segment-level borrows keep this crisp: one
    // registry, three non-overlapping writes, no whole-struct lock.
    let mut borrows = SegmentBorrowRegistry::new();
    {
        let mut authority_seg = vault.segment_mut::<[u8; 32]>(
            &mut borrows,
            HookedVault::AUTHORITY_ABS_OFFSET,
            32,
        )?;
        authority_seg.copy_from_slice(authority.address().as_array());
    }
    {
        let mut mint_seg = vault.segment_mut::<[u8; 32]>(
            &mut borrows,
            HookedVault::MINT_ABS_OFFSET,
            32,
        )?;
        mint_seg.copy_from_slice(mint.address().as_array());
    }
    // expected_hook_program defaults to all zeros; the authority sets
    // the first real binding via process_rotate_expected_hook.

    Ok(())
}

// ---------------------------------------------------------------------------
// 1. verify_hook_binding
// ---------------------------------------------------------------------------
//
// The headline instruction. Reads the mint's TransferHook extension,
// compares it against the binding declared in the vault state, and
// fails the call on any divergence.
fn process_verify_hook_binding(
    program_id: &Address,
    accounts: &[AccountView],
) -> ProgramResult {
    let [vault, mint, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    vault.require_owned_by(program_id)?;

    // Borrow the vault's stored binding and mint reference, validate
    // the supplied mint matches what the vault is pinned to.
    let vault_view = HookedVault::load(vault, program_id)?;
    if vault_view.mint.as_bytes() != mint.address().as_array() {
        return Err(ProgramError::InvalidAccountData);
    }

    // Read the mint's actual TransferHook extension and compare.
    let mint_data = mint.try_borrow()?;
    check_transfer_hook_program(
        &mint_data,
        vault_view.expected_hook_program.as_bytes(),
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// 2. require_safe_mint
// ---------------------------------------------------------------------------
//
// Rejects any mint that carries a DeFi-unsafe Token-2022 extension
// (transfer fee, permanent delegate, confidential transfer,
// non-transferable, transfer hook). The canonical "my AMM does not
// understand exotic mints" gate.
fn process_require_safe_mint(accounts: &[AccountView]) -> ProgramResult {
    let [mint, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    let mint_data = mint.try_borrow()?;
    check_safe_token_2022_mint(&mint_data)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 3. rotate_expected_hook
// ---------------------------------------------------------------------------
//
// Authority-gated update of the stored expected hook program ID.
// Reads the mint's current hook binding and stores it as the new
// expectation. A UI might prompt the human authority to review and
// re-sign when the mint's hook authority migrates; this instruction
// is the on-chain apply step.
fn process_rotate_expected_hook(
    program_id: &Address,
    accounts: &[AccountView],
) -> ProgramResult {
    let [authority, vault, mint, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    authority.require_signer()?;
    vault.require_writable()?;
    vault.require_owned_by(program_id)?;

    // Load-and-verify: confirm the signer matches the stored authority
    // and the mint matches the one the vault is bound to.
    {
        let vault_view = HookedVault::load(vault, program_id)?;
        if vault_view.authority.as_bytes() != authority.address().as_array() {
            return Err(ProgramError::MissingRequiredSignature);
        }
        if vault_view.mint.as_bytes() != mint.address().as_array() {
            return Err(ProgramError::InvalidAccountData);
        }
    }

    // Read the mint's current hook binding (must exist — rotating
    // into "no hook" is intentionally not supported; use
    // require_safe_mint to enforce absence instead).
    let new_hook_program: [u8; 32] = {
        let mint_data = mint.try_borrow()?;
        let hook = read_transfer_hook(&mint_data)?
            .ok_or(ProgramError::InvalidAccountData)?;
        *hook.program_id
    };

    // Commit the rotation.
    let mut borrows = SegmentBorrowRegistry::new();
    let mut slot = vault.segment_mut::<[u8; 32]>(
        &mut borrows,
        HookedVault::EXPECTED_HOOK_PROGRAM_ABS_OFFSET,
        32,
    )?;
    slot.copy_from_slice(&new_hook_program);

    Ok(())
}
