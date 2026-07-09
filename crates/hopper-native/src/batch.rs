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
pub fn close_and_transfer(
    source: &AccountView<'_>,
    destination: &AccountView<'_>,
) -> ProgramResult {
    let lamports = source.lamports();
    if lamports == 0 {
        // Already empty -- just close.
        source.close()?;
        return Ok(());
    }

    // Move lamports.
    destination.set_lamports(
        destination
            .lamports()
            .checked_add(lamports)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );

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
    from: &AccountView<'_>,
    to: &AccountView<'_>,
    amount: u64,
) -> ProgramResult {
    let from_lamports = from.lamports();
    if from_lamports < amount {
        return Err(ProgramError::InsufficientFunds);
    }
    let to_lamports = to.lamports();
    let new_to = to_lamports
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    from.set_lamports(from_lamports - amount);
    to.set_lamports(new_to);
    Ok(())
}

/// Verify that an account is rent-exempt using the **hardcoded** current
/// rent constants (the fast, syscall-free path).
///
/// # SAFETY-CRITICAL caveat
///
/// This gates on [`crate::sysvar::rent_exempt_minimum`], which cannot see a
/// rent *reprice*. If the cluster has raised rent, this can report an account
/// as rent-exempt when the runtime would reap it. For any decision where a
/// wrong "exempt" answer risks data loss, prefer
/// [`require_rent_exempt_with`], which reads the live [`Rent`] sysvar.
#[inline]
pub fn require_rent_exempt(account: &AccountView<'_>) -> ProgramResult {
    let min = crate::sysvar::rent_exempt_minimum(account.data_len());
    if account.lamports() >= min {
        Ok(())
    } else {
        Err(ProgramError::AccountNotRentExempt)
    }
}

/// Verify that an account is rent-exempt against a live [`Rent`] sysvar
/// (RECOMMENDED for reaping-relevant checks).
///
/// The caller reads the sysvar once (`Rent::get()`) and passes it in, so this
/// function adds no syscall of its own — the cost stays where the caller can
/// see it — while using the cluster's *actual* rent parameters. This is the
/// correct form when the cluster may have repriced rent since the constants
/// baked into [`require_rent_exempt`] were set: it uses
/// [`Rent::minimum_balance`], which byte-matches the runtime.
///
/// # Example
///
/// ```ignore
/// let rent = hopper::sysvar::Rent::get()?;
/// hopper::batch::require_rent_exempt_with(&rent, account)?;
/// ```
#[inline]
pub fn require_rent_exempt_with(
    rent: &crate::sysvar::Rent,
    account: &AccountView<'_>,
) -> ProgramResult {
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
pub fn require_same_address(a: &AccountView<'_>, b: &AccountView<'_>) -> ProgramResult {
    if crate::address::address_eq(a.address(), b.address()) {
        Ok(())
    } else {
        Err(ProgramError::InvalidArgument)
    }
}

/// Assert that an account's address matches an expected address.
#[inline]
pub fn require_address(account: &AccountView<'_>, expected: &Address) -> ProgramResult {
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
    account: &AccountView<'_>,
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
///
/// Fails with `AccountBorrowFailed` while any data borrow is outstanding
/// (zeroing would mutate memory a live `Ref`/`RefMut` still points at).
#[inline]
pub fn zero_data(account: &AccountView<'_>) -> ProgramResult {
    // Delegate to the borrow-guarded, SVM-memset-optimized helper rather
    // than duplicating an unguarded byte loop here.
    crate::mem::zero_account_data(account)
}

/// Checked realloc that also ensures the account remains rent-exempt
/// after resizing.
///
/// This is the safe version of `account.resize()` -- it verifies that
/// the account has enough lamports to cover rent at the new data length.
///
/// # Reaping caveat
///
/// The top-up target comes from the hardcoded
/// [`crate::sysvar::rent_exempt_minimum`] const, so it cannot see a rent
/// reprice. If the cluster ever raises the rent parameters this
/// UNDER-funds the account, leaving it reapable (data loss). Any resize
/// whose safety must survive a reprice should call
/// [`realloc_checked_with`] with a freshly read [`crate::sysvar::Rent`].
#[inline]
pub fn realloc_checked(
    account: &AccountView<'_>,
    new_len: usize,
    payer: Option<&AccountView<'_>>,
) -> ProgramResult {
    // Check rent requirement BEFORE resizing to avoid leaving the account
    // in an inconsistent state if the payer transfer fails.
    let min = crate::sysvar::rent_exempt_minimum(new_len);
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

/// Reaping-safe `realloc_checked`: tops the account up to rent-exemption
/// using the **live [`Rent`] sysvar**, so it stays correct after a rent
/// reprice.
///
/// [`realloc_checked`] computes its top-up from the hardcoded
/// [`crate::sysvar::rent_exempt_minimum`] const, which cannot see a
/// reprice and would UNDER-fund the account (leaving it reapable, data
/// lost) if the cluster ever raised `lamports_per_byte_year` or the
/// exemption threshold. Any resize whose correctness must survive a
/// reprice should call this variant with a freshly read sysvar:
///
/// ```ignore
/// let rent = hopper::sysvar::Rent::get()?;
/// hopper::batch::realloc_checked_with(&rent, account, new_len, Some(payer))?;
/// ```
///
/// [`Rent`]: crate::sysvar::Rent
#[inline]
pub fn realloc_checked_with(
    rent: &crate::sysvar::Rent,
    account: &AccountView<'_>,
    new_len: usize,
    payer: Option<&AccountView<'_>>,
) -> ProgramResult {
    // Top-up computed from the live sysvar, not the const snapshot.
    // Check rent BEFORE resizing so a failed payer transfer leaves the
    // account's data length unchanged (same ordering as realloc_checked).
    let min = rent.minimum_balance(new_len);
    let current = account.lamports();

    if current < min {
        if let Some(payer) = payer {
            let deficit = min - current;
            transfer_lamports(payer, account, deficit)?;
        } else {
            return Err(ProgramError::AccountNotRentExempt);
        }
    }

    account.resize(new_len)
}
