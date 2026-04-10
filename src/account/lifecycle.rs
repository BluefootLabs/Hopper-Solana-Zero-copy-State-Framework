//! Account lifecycle operations: init, close, realloc.

use hopper_runtime::{error::ProgramError, AccountView, ProgramResult};

/// Sentinel byte written to byte 0 when an account is closed.
/// Prevents account revival attacks.
pub const CLOSE_SENTINEL: u8 = 0xFF;

/// Zero-initialize a byte slice. Must be called before `write_header`.
///
/// Solana does NOT guarantee zeroed account data on creation.
/// Always call this on freshly allocated accounts.
#[inline(always)]
pub fn zero_init(data: &mut [u8]) {
    // NOTE: Using a byte-by-byte fill to avoid any alignment issues.
    // The compiler will optimize this to memset.
    for byte in data.iter_mut() {
        *byte = 0;
    }
}

/// Safely close an account by draining all lamports to `destination`.
///
/// Zeroes the account data and writes the close sentinel to prevent revival.
#[inline]
pub fn safe_close(
    account: &AccountView,
    destination: &AccountView,
) -> ProgramResult {
    let lamports = account.lamports();
    if lamports == 0 {
        return Ok(());
    }

    // Add to destination
    let new_dest = destination.lamports()
        .checked_add(lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    destination.set_lamports(new_dest);

    // Drain source
    account.set_lamports(0);

    // Zero account data
    let mut data = account.try_borrow_mut()?;
    zero_init(&mut data);

    Ok(())
}

/// Close with sentinel -- writes `CLOSE_SENTINEL` to byte 0 after zeroing.
#[inline]
pub fn safe_close_with_sentinel(
    account: &AccountView,
    destination: &AccountView,
) -> ProgramResult {
    safe_close(account, destination)?;

    // Write sentinel to prevent revival
    let mut data = account.try_borrow_mut()?;
    if !data.is_empty() {
        data[0] = CLOSE_SENTINEL;
    }

    Ok(())
}

/// Reallocate an account to a new size.
///
/// Handles the rent-exemption delta and transfers lamports from/to `payer`.
#[inline]
pub fn safe_realloc(
    account: &AccountView,
    new_size: usize,
    payer: &AccountView,
) -> ProgramResult {
    account.resize(new_size)?;

    // Compute new rent and transfer delta
    let rent_needed = rent_exempt_min_internal(new_size);
    let current_lamports = account.lamports();

    if rent_needed > current_lamports {
        let deficit = rent_needed - current_lamports;
        // Transfer from payer to account
        let payer_lamports = payer.lamports()
            .checked_sub(deficit)
            .ok_or(ProgramError::InsufficientFunds)?;
        payer.set_lamports(payer_lamports);
        let acct_lamports = account.lamports()
            .checked_add(deficit)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        account.set_lamports(acct_lamports);
    }

    Ok(())
}

// Internal rent calculation (matches Solana's formula).
pub(crate) fn rent_exempt_min_internal(data_len: usize) -> u64 {
    // Solana formula: (128 + data_len) * 6960 lamports (approximately)
    // This is the standard exemption calculation.
    ((128 + data_len) as u64) * 6960
}
