//! Batch account operations.
//!
//! Common multi-account patterns as single methods with clearer intent
//! and fewer repeated unsafe blocks. These are operations that every
//! serious Solana program needs but nobody bundles at the substrate level.

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::ProgramResult;

/// Transfer all lamports from `source` to `destination` and zero the source.
///
/// This is the standard "close an account" pattern: move all SOL to
/// the rent receiver and wipe the source account. Combines what would
/// normally be 3 separate operations (read lamports, set source to 0,
/// add to destination) into one safe call.
#[inline]
pub fn close_and_transfer(source: &AccountView, destination: &AccountView) -> ProgramResult {
    let lamports = source.lamports();
    if lamports == 0 {
        // Already empty -- just close.
        source.close()?;
        return Ok(());
    }

    // Move lamports.
    destination.set_lamports(destination.lamports().checked_add(lamports).ok_or(
        ProgramError::ArithmeticOverflow,
    )?);

    // Close source (zeros data, sets owner to system program).
    source.close()
}

/// Transfer `amount` lamports between two accounts without CPI.
///
/// For accounts owned by the current program, direct lamport
/// manipulation is cheaper than a system program CPI transfer.
/// This method checks for sufficient balance and overflow.
#[inline]
pub fn transfer_lamports(
    from: &AccountView,
    to: &AccountView,
    amount: u64,
) -> ProgramResult {
    let from_lamports = from.lamports();
    if from_lamports < amount {
        return Err(ProgramError::InsufficientFunds);
    }
    let to_lamports = to.lamports();
    let new_to = to_lamports.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;

    from.set_lamports(from_lamports - amount);
    to.set_lamports(new_to);
    Ok(())
}

/// Verify that an account is rent-exempt given the current rent parameters.
///
/// Reads the Rent sysvar and checks that the account's lamports meet
/// the minimum balance for its data length.
#[inline]
pub fn require_rent_exempt(account: &AccountView) -> ProgramResult {
    let rent = crate::sysvar::get_rent()?;
    let min = rent.minimum_balance(account.data_len());
    if account.lamports() >= min {
        Ok(())
    } else {
        Err(ProgramError::AccountNotRentExempt)
    }
}

/// Assert that two accounts have the same address.
///
/// Useful for verifying expected accounts match (e.g., token mint
/// matches the vault's expected mint).
#[inline]
pub fn require_same_address(a: &AccountView, b: &AccountView) -> ProgramResult {
    if crate::address::address_eq(a.address(), b.address()) {
        Ok(())
    } else {
        Err(ProgramError::InvalidArgument)
    }
}

/// Assert that an account's address matches an expected address.
#[inline]
pub fn require_address(account: &AccountView, expected: &Address) -> ProgramResult {
    if crate::address::address_eq(account.address(), expected) {
        Ok(())
    } else {
        Err(ProgramError::InvalidArgument)
    }
}

/// Assert that an account has the expected discriminator AND is owned
/// by the given program. This two-check combo is the most common
/// "is this the right account type?" pattern in Solana programs.
#[inline]
pub fn require_account_type(
    account: &AccountView,
    expected_disc: u8,
    expected_owner: &Address,
) -> ProgramResult {
    if account.disc() != expected_disc {
        return Err(ProgramError::InvalidAccountData);
    }
    account.require_owned_by(expected_owner)
}

/// Zero the data bytes of an account without changing lamports or owner.
///
/// Useful for "soft close" patterns where you want to mark an account
/// as consumed but leave it allocated for potential reuse.
#[inline]
pub fn zero_data(account: &AccountView) -> ProgramResult {
    let len = account.data_len();
    if len == 0 {
        return Ok(());
    }
    let data_ptr = account.data_ptr();
    unsafe {
        core::ptr::write_bytes(data_ptr, 0, len);
    }
    Ok(())
}

/// Checked realloc that also ensures the account remains rent-exempt
/// after resizing.
///
/// This is the safe version of `account.resize()` -- it verifies that
/// the account has enough lamports to cover rent at the new data length.
#[inline]
pub fn realloc_checked(
    account: &AccountView,
    new_len: usize,
    payer: Option<&AccountView>,
) -> ProgramResult {
    // Check rent requirement BEFORE resizing to avoid leaving the account
    // in an inconsistent state if the payer transfer fails.
    let rent = crate::sysvar::get_rent()?;
    let min = rent.minimum_balance(new_len);
    let current = account.lamports();

    if current < min {
        // Need more lamports. Transfer BEFORE resize so that if the
        // transfer fails, the account data length is unchanged.
        if let Some(payer) = payer {
            let deficit = min - current;
            transfer_lamports(payer, account, deficit)?;
        } else {
            return Err(ProgramError::AccountNotRentExempt);
        }
    }

    // Now resize -- the account already has enough lamports.
    account.resize(new_len)
}
