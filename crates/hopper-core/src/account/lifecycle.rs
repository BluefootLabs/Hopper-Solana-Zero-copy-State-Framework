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
/// This is the advanced no-sentinel primitive: it zeroes account data but does
/// not mark byte 0 with [`CLOSE_SENTINEL`]. Generated Hopper close helpers use
/// [`safe_close_with_sentinel`] via `hopper_close!`.
#[inline]
pub fn safe_close(account: &AccountView, destination: &AccountView) -> ProgramResult {
    let lamports = account.lamports();
    if lamports == 0 {
        return Ok(());
    }

    // Add to destination
    let new_dest = destination
        .lamports()
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
pub fn safe_close_with_sentinel(account: &AccountView, destination: &AccountView) -> ProgramResult {
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
/// Handles the rent-exemption delta and transfers lamports from `payer` after
/// preflighting rent and balance checks. The order is intentional: the function
/// does all arithmetic and funding validation before the account data length is
/// changed, then performs the resize, then applies the lamport movement.
#[inline]
pub fn safe_realloc(account: &AccountView, new_size: usize, payer: &AccountView) -> ProgramResult {
    let rent_needed = rent_exempt_min_internal(new_size)?;
    let current_lamports = account.lamports();
    let deficit = rent_needed.saturating_sub(current_lamports);

    let payer_lamports_after = if deficit > 0 {
        Some(
            payer
                .lamports()
                .checked_sub(deficit)
                .ok_or(ProgramError::InsufficientFunds)?,
        )
    } else {
        None
    };
    let account_lamports_after = if deficit > 0 {
        Some(
            current_lamports
                .checked_add(deficit)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        )
    } else {
        None
    };

    account.resize(new_size)?;

    if let (Some(payer_lamports), Some(account_lamports)) =
        (payer_lamports_after, account_lamports_after)
    {
        payer.set_lamports(payer_lamports);
        account.set_lamports(account_lamports);
    }

    Ok(())
}

// Internal rent calculation (matches Solana's formula).
pub(crate) fn rent_exempt_min_internal(data_len: usize) -> Result<u64, ProgramError> {
    // Solana formula: (128 + data_len) * 6960 lamports (approximately)
    // This is the standard exemption calculation.
    let data_len = u64::try_from(data_len).map_err(|_| ProgramError::ArithmeticOverflow)?;
    data_len
        .checked_add(128)
        .and_then(|bytes| bytes.checked_mul(6960))
        .ok_or(ProgramError::ArithmeticOverflow)
}

#[cfg(all(test, feature = "hopper-native-backend"))]
mod tests {
    use super::*;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make_account(data_len: usize, lamports: u64, seed: u8) -> (std::vec::Vec<u8>, AccountView) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: The test owns `backing`, writes one valid RuntimeAccount header,
        // and keeps the backing buffer alive for the returned AccountView.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([seed; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports,
                data_len: data_len as u64,
            });
        }
        // SAFETY: `raw` points at the RuntimeAccount header just initialized above.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        // SAFETY: hopper-runtime AccountView is repr(transparent) over the active
        // hopper-native AccountView when the hopper-native backend feature is enabled.
        let view = unsafe { core::mem::transmute::<NativeAccountView, AccountView>(backend) };
        (backing, view)
    }

    #[test]
    fn safe_realloc_checks_funding_before_resize() {
        let (_account_backing, account) = make_account(16, 0, 1);
        let (_payer_backing, payer) = make_account(0, 1, 2);

        let result = safe_realloc(&account, 64, &payer);

        assert_eq!(result, Err(ProgramError::InsufficientFunds));
        assert_eq!(account.data_len(), 16);
        assert_eq!(account.lamports(), 0);
        assert_eq!(payer.lamports(), 1);
    }

    #[test]
    fn safe_realloc_moves_lamports_after_successful_resize() {
        let needed = rent_exempt_min_internal(32).unwrap();
        let (_account_backing, account) = make_account(16, 0, 3);
        let (_payer_backing, payer) = make_account(0, needed, 4);

        safe_realloc(&account, 32, &payer).unwrap();

        assert_eq!(account.data_len(), 32);
        assert_eq!(account.lamports(), needed);
        assert_eq!(payer.lamports(), 0);
    }
}
