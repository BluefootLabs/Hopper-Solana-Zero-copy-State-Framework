//! Account lifecycle operations: init, close, realloc.

use crate::check::{check_owner, check_signer, check_writable};
use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult};

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

#[inline]
fn move_all_lamports(account: &AccountView<'_>, destination: &AccountView<'_>) -> ProgramResult {
    let lamports = account.lamports();
    let new_dest = destination
        .lamports()
        .checked_add(lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    destination.try_set_lamports(new_dest)?;
    account.try_set_lamports(0)?;
    Ok(())
}

/// Safely close a program-owned account by draining all lamports to `destination`.
///
/// This is the advanced no-sentinel primitive: it zeroes account data but does
/// not mark byte 0 with [`CLOSE_SENTINEL`]. Generated Hopper close helpers use
/// [`safe_close_with_sentinel`] via `hopper_close!`.
///
/// The source account must be writable and owned by `program_id`. The
/// destination must be writable because its lamport balance is credited.
#[inline]
pub fn safe_close(
    account: &AccountView<'_>,
    destination: &AccountView<'_>,
    program_id: &Address,
) -> ProgramResult {
    check_writable(account)?;
    check_owner(account, program_id)?;
    check_writable(destination)?;
    safe_close_unchecked(account, destination)
}

/// Explicitly unchecked variant of [`safe_close`].
///
/// This still uses fallible lamport and data mutation, but it does not verify
/// writability or ownership. Use it only after those preconditions were already
/// established by generated or hand-written validation.
#[inline]
pub fn safe_close_unchecked(account: &AccountView<'_>, destination: &AccountView<'_>) -> ProgramResult {
    // Fail before moving lamports if outstanding data borrows would prevent
    // zeroing the closed account.
    drop(account.try_borrow_mut()?);

    move_all_lamports(account, destination)?;

    // Zero account data even when the source has no lamports; closing is a data
    // lifecycle transition, not only a balance transfer.
    let mut data = account.try_borrow_mut()?;
    zero_init(&mut data);

    Ok(())
}

/// Close with sentinel -- writes `CLOSE_SENTINEL` to byte 0 after zeroing.
#[inline]
pub fn safe_close_with_sentinel(
    account: &AccountView<'_>,
    destination: &AccountView<'_>,
    program_id: &Address,
) -> ProgramResult {
    safe_close(account, destination, program_id)?;
    write_close_sentinel(account)
}

/// Explicitly unchecked sentinel close variant.
#[inline]
pub fn safe_close_with_sentinel_unchecked(
    account: &AccountView<'_>,
    destination: &AccountView<'_>,
) -> ProgramResult {
    safe_close_unchecked(account, destination)?;
    write_close_sentinel(account)
}

#[inline]
fn write_close_sentinel(account: &AccountView<'_>) -> ProgramResult {
    // Write sentinel to prevent revival.
    let mut data = account.try_borrow_mut()?;
    if !data.is_empty() {
        data[0] = CLOSE_SENTINEL;
    }
    Ok(())
}

/// Reallocate a program-owned account to a new size.
///
/// Handles the rent-exemption delta and transfers lamports from `payer` after
/// preflighting rent and balance checks. The order is intentional: the function
/// does all arithmetic and funding validation before the account data length is
/// changed, then performs the resize, then applies the lamport movement.
///
/// The resized account must be writable and owned by `program_id`. When a rent
/// top-up is required, `payer` must be writable and a signer.
#[inline]
pub fn safe_realloc(
    account: &AccountView<'_>,
    new_size: usize,
    payer: &AccountView<'_>,
    program_id: &Address,
) -> ProgramResult {
    check_writable(account)?;
    check_owner(account, program_id)?;
    safe_realloc_unchecked(account, new_size, payer, true)
}

/// Explicitly unchecked variant of [`safe_realloc`].
///
/// This does not verify the resized account's writability or ownership. When
/// `require_payer_signature_for_top_up` is true, payer signer+writable checks
/// still run if more lamports are required.
#[inline]
pub fn safe_realloc_unchecked(
    account: &AccountView<'_>,
    new_size: usize,
    payer: &AccountView<'_>,
    require_payer_signature_for_top_up: bool,
) -> ProgramResult {
    let rent_needed = rent_exempt_min_internal(new_size)?;
    let current_lamports = account.lamports();
    let deficit = rent_needed.saturating_sub(current_lamports);

    let payer_lamports_after = if deficit > 0 {
        check_writable(payer)?;
        if require_payer_signature_for_top_up {
            check_signer(payer)?;
        }
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

    // Fail fast on outstanding data borrows before changing data length.
    drop(account.try_borrow_mut()?);

    account.resize(new_size)?;

    if let (Some(payer_lamports), Some(account_lamports)) =
        (payer_lamports_after, account_lamports_after)
    {
        payer.try_set_lamports(payer_lamports)?;
        account.try_set_lamports(account_lamports)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    const PROGRAM_BYTES: [u8; 32] = [2; 32];
    const OTHER_OWNER_BYTES: [u8; 32] = [9; 32];

    fn program_id() -> Address {
        Address::new_from_array(PROGRAM_BYTES)
    }

    fn make_account_with_flags(
        data_len: usize,
        lamports: u64,
        seed: u8,
        owner: [u8; 32],
        is_writable: bool,
        is_signer: bool,
    ) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: The test owns `backing`, writes one valid RuntimeAccount header,
        // and keeps the backing buffer alive for the returned AccountView.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: u8::from(is_signer),
                is_writable: u8::from(is_writable),
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([seed; 32]),
                owner: NativeAddress::new_from_array(owner),
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

    fn make_account(
        data_len: usize,
        lamports: u64,
        seed: u8,
    ) -> (std::vec::Vec<u8>, AccountView<'static>) {
        make_account_with_flags(data_len, lamports, seed, PROGRAM_BYTES, true, true)
    }

    #[test]
    fn safe_close_rejects_non_writable_source() {
        let (_account_backing, account) =
            make_account_with_flags(8, 10, 1, PROGRAM_BYTES, false, true);
        let (_dest_backing, destination) = make_account(0, 5, 2);

        let result = safe_close(&account, &destination, &program_id());

        assert_eq!(result, Err(ProgramError::InvalidAccountData));
        assert_eq!(account.lamports(), 10);
        assert_eq!(destination.lamports(), 5);
    }

    #[test]
    fn safe_close_rejects_non_writable_destination() {
        let (_account_backing, account) = make_account(8, 10, 3);
        let (_dest_backing, destination) =
            make_account_with_flags(0, 5, 4, PROGRAM_BYTES, false, true);

        let result = safe_close(&account, &destination, &program_id());

        assert_eq!(result, Err(ProgramError::InvalidAccountData));
        assert_eq!(account.lamports(), 10);
        assert_eq!(destination.lamports(), 5);
    }

    #[test]
    fn safe_close_rejects_wrong_owner() {
        let (_account_backing, account) =
            make_account_with_flags(8, 10, 5, OTHER_OWNER_BYTES, true, true);
        let (_dest_backing, destination) = make_account(0, 5, 6);

        let result = safe_close(&account, &destination, &program_id());

        assert_eq!(result, Err(ProgramError::IncorrectProgramId));
        assert_eq!(account.lamports(), 10);
        assert_eq!(destination.lamports(), 5);
    }

    #[test]
    fn safe_close_fails_before_lamports_move_when_data_is_borrowed() {
        let (_account_backing, account) = make_account(8, 10, 7);
        let (_dest_backing, destination) = make_account(0, 5, 8);
        let _borrow = account.try_borrow().unwrap();

        let result = safe_close(&account, &destination, &program_id());

        assert_eq!(result, Err(ProgramError::AccountBorrowFailed));
        assert_eq!(account.lamports(), 10);
        assert_eq!(destination.lamports(), 5);
    }

    #[test]
    fn safe_close_with_sentinel_moves_lamports_and_marks_data() {
        let (_account_backing, account) = make_account(8, 10, 9);
        let (_dest_backing, destination) = make_account(0, 5, 10);

        safe_close_with_sentinel(&account, &destination, &program_id()).unwrap();

        assert_eq!(account.lamports(), 0);
        assert_eq!(destination.lamports(), 15);
        let data = account.try_borrow().unwrap();
        assert_eq!(data[0], CLOSE_SENTINEL);
        assert!(data[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn safe_realloc_rejects_non_writable_account() {
        let (_account_backing, account) = make_account_with_flags(
            16,
            rent_exempt_min_internal(32).unwrap(),
            11,
            PROGRAM_BYTES,
            false,
            true,
        );
        let (_payer_backing, payer) = make_account(0, 0, 12);

        let result = safe_realloc(&account, 32, &payer, &program_id());

        assert_eq!(result, Err(ProgramError::InvalidAccountData));
        assert_eq!(account.data_len(), 16);
    }

    #[test]
    fn safe_realloc_rejects_wrong_owner() {
        let (_account_backing, account) = make_account_with_flags(
            16,
            rent_exempt_min_internal(32).unwrap(),
            13,
            OTHER_OWNER_BYTES,
            true,
            true,
        );
        let (_payer_backing, payer) = make_account(0, 0, 14);

        let result = safe_realloc(&account, 32, &payer, &program_id());

        assert_eq!(result, Err(ProgramError::IncorrectProgramId));
        assert_eq!(account.data_len(), 16);
    }

    #[test]
    fn safe_realloc_checks_funding_before_resize() {
        let (_account_backing, account) = make_account(16, 0, 15);
        let (_payer_backing, payer) = make_account(0, 1, 16);

        let result = safe_realloc(&account, 64, &payer, &program_id());

        assert_eq!(result, Err(ProgramError::InsufficientFunds));
        assert_eq!(account.data_len(), 16);
        assert_eq!(account.lamports(), 0);
        assert_eq!(payer.lamports(), 1);
    }

    #[test]
    fn safe_realloc_rejects_non_writable_payer_when_top_up_required() {
        let needed = rent_exempt_min_internal(32).unwrap();
        let (_account_backing, account) = make_account(16, 0, 17);
        let (_payer_backing, payer) =
            make_account_with_flags(0, needed, 18, PROGRAM_BYTES, false, true);

        let result = safe_realloc(&account, 32, &payer, &program_id());

        assert_eq!(result, Err(ProgramError::InvalidAccountData));
        assert_eq!(account.data_len(), 16);
        assert_eq!(account.lamports(), 0);
        assert_eq!(payer.lamports(), needed);
    }

    #[test]
    fn safe_realloc_rejects_unsigned_payer_when_top_up_required() {
        let needed = rent_exempt_min_internal(32).unwrap();
        let (_account_backing, account) = make_account(16, 0, 19);
        let (_payer_backing, payer) =
            make_account_with_flags(0, needed, 20, PROGRAM_BYTES, true, false);

        let result = safe_realloc(&account, 32, &payer, &program_id());

        assert_eq!(result, Err(ProgramError::MissingRequiredSignature));
        assert_eq!(account.data_len(), 16);
        assert_eq!(account.lamports(), 0);
        assert_eq!(payer.lamports(), needed);
    }

    #[test]
    fn safe_realloc_fails_before_resize_when_data_is_borrowed() {
        let needed = rent_exempt_min_internal(32).unwrap();
        let (_account_backing, account) = make_account(16, needed, 21);
        let (_payer_backing, payer) = make_account(0, 0, 22);
        let _borrow = account.try_borrow().unwrap();

        let result = safe_realloc(&account, 32, &payer, &program_id());

        assert_eq!(result, Err(ProgramError::AccountBorrowFailed));
        assert_eq!(account.data_len(), 16);
        assert_eq!(account.lamports(), needed);
        assert_eq!(payer.lamports(), 0);
    }

    #[test]
    fn safe_realloc_moves_lamports_after_successful_resize() {
        let needed = rent_exempt_min_internal(32).unwrap();
        let (_account_backing, account) = make_account(16, 0, 23);
        let (_payer_backing, payer) = make_account(0, needed, 24);

        safe_realloc(&account, 32, &payer, &program_id()).unwrap();

        assert_eq!(account.data_len(), 32);
        assert_eq!(account.lamports(), needed);
        assert_eq!(payer.lamports(), 0);
    }
}
