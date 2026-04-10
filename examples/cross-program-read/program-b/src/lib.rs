//! # Program B -- Cross-Program Vault Reader
//!
//! Reads Program A's `Vault` account using `hopper_interface!`.
//!
//! **This crate has NO dependency on Program A.** It declares its own
//! `VaultView` struct with the same fields, types, sizes, and version as
//! Program A's `Vault`. Because `hopper_interface!` produces a deterministic
//! `LAYOUT_ID` from the field descriptors (SHA-256 based), `load_foreign()`
//! can verify ABI compatibility at runtime without any compile-time coupling.
//!
//! ## How It Works
//!
//! 1. Program A defines `Vault` with `hopper_layout!` → produces `LAYOUT_ID_A`.
//! 2. Program B defines `VaultView` with `hopper_interface!` using the same
//!    field spec → produces `LAYOUT_ID_B`.
//! 3. Same fields + same ordering + same types + same sizes → `LAYOUT_ID_A == LAYOUT_ID_B`.
//! 4. `VaultView::load_foreign()` checks `owner == PROGRAM_A_ID` and
//!    `layout_id == LAYOUT_ID_B`. Both pass. Read succeeds.
//! 5. If Program A changes its `Vault` layout, `LAYOUT_ID_A` changes,
//!    and `load_foreign()` rejects the account → no silent schema drift.
//!
//! ## Important
//!
//! The struct name in `hopper_interface!`/`hopper_layout!` is part of the
//! hash input. For cross-program reads to work, the interface struct name
//! must match the originating layout name exactly, OR you must use
//! `hopper_assert_fingerprint!` to pin to a known fingerprint value.
//!
//! In this example, we name the interface `Vault` (matching Program A)
//! to get automatic LAYOUT_ID matching. If you prefer a different name
//! (e.g., `VaultView`), you'd need to pin the fingerprint manually.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

// --- Interface View -------------------------------------------------
//
// This struct is declared independently -- no import from Program A.
// Because the field names, types, sizes, and version are identical to
// Program A's Vault, the computed LAYOUT_ID will match.

hopper_interface! {
    /// Read-only view of Program A's Vault account.
    ///
    /// Same byte layout as Program A's `Vault`. The `LAYOUT_ID` fingerprint
    /// is deterministic: same fields → same hash → `load_foreign` succeeds.
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}

// --- Hard-coded Program A address -----------------------------------
//
// In production, this would typically be a well-known deployed program ID
// constant or an instruction-selected target.

const PROGRAM_A_ID: Address = Address::new_from_array(five8_const::decode_32_const(
    "11111111111111111111111111111112"
));

// --- Errors ---------------------------------------------------------

hopper_error! {
    base = 7000;
    VaultReadFailed,
    InsufficientVaultBalance,
}

// --- Entrypoint -----------------------------------------------------

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_read_vault,
        1 => process_check_vault_min_balance,
    }
}

// --- Read Vault Balance (basic cross-program read) ------------------

fn process_read_vault(
    _program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.is_empty() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let vault_account = &accounts[0];

    // Tier 2: Cross-program read.
    // Validates: owner == PROGRAM_A_ID, layout_id matches, exact size.
    let verified = Vault::load_foreign(vault_account, &PROGRAM_A_ID)?;
    let vault = verified.get();

    // Access Program A's vault data -- zero copies, zero deserialization.
    let _balance = vault.balance.get();
    let _authority = vault.authority.as_bytes();

    Ok(())
}

// --- Check Vault Minimum Balance (with TrustProfile) ----------------

fn process_check_vault_min_balance(
    _program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.is_empty() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let vault_account = &accounts[0];

    // Parse minimum balance threshold from instruction data.
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let min_balance = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);

    // Use a TrustProfile for configurable validation.
    // Strict: owner + layout_id + exact size + reject closed.
    let profile = TrustProfile::strict(
        &PROGRAM_A_ID,
        &Vault::LAYOUT_ID,
        Vault::LEN,
    );
    let verified = Vault::load_with_profile(vault_account, &profile)?;
    let vault = verified.get();

    // Business logic: ensure vault meets minimum balance.
    hopper_require!(vault.balance.get() >= min_balance, InsufficientVaultBalance);

    Ok(())
}
