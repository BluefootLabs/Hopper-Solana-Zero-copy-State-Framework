//! Batch account operations.
//!
//! Common multi-account patterns as single methods with clearer intent
//! and fewer repeated unsafe blocks.

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::ProgramResult;

/// Transfer all lamports from `source` to `destination` and zero the source.
///
/// Both accounts must be writable and distinct. Borrow conflicts and credit
/// overflow are rejected before either balance changes, even if the caller
/// catches the error. The caller must verify that the executing program owns
/// the source and that the application authorizes closure.
#[inline]
pub fn close_and_transfer(
    source: &AccountView<'_>,
    destination: &AccountView<'_>,
) -> ProgramResult {
    if crate::address::address_eq(source.address(), destination.address()) {
        return Err(ProgramError::InvalidArgument);
    }
    source.require_writable()?;
    destination.require_writable()?;
    source.check_borrow_mut()?;
    let lamports = source.lamports();
    let credited = destination
        .lamports()
        .checked_add(lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    // Every fallible precondition is checked before either balance changes.
    // Closing cannot encounter a new borrow: there is no intervening CPI or callback.
    source.close()?;
    destination.set_lamports(credited);
    Ok(())
}

/// Transfer `amount` lamports between two accounts without CPI.
///
/// For accounts owned by the current program, direct lamport
/// manipulation is cheaper than a system program CPI transfer.
/// This method checks both writable flags, sufficient balance, and overflow.
/// A same-address transfer is balance-checked net zero. The caller must verify
/// ownership and application authority to debit the source.
///
/// # Gated programs (`strict_writes` + `lamports(...)`)
///
/// This substrate helper writes balances directly at the native layer
/// and **bypasses the runtime's lamport gate by design**; it is the
/// cheap no-CPI path and sits outside hopper-runtime's governed
/// surface. Under a context that declares `strict_writes` +
/// `lamports(...)` (the mutation-complete contract), use
/// `hopper_runtime::transfer_lamports` instead, also reachable via
/// `hopper::prelude` and as the generated `ctx.transfer_lamports(..)`
/// bound-context method: identical arithmetic, but both sides cross
/// the gated `native_boundary` funnel, so the mutation-complete
/// guarantee covers the move.
#[inline]
pub fn transfer_lamports(
    from: &AccountView<'_>,
    to: &AccountView<'_>,
    amount: u64,
) -> ProgramResult {
    from.require_writable()?;
    to.require_writable()?;
    let from_lamports = from.lamports();
    if from_lamports < amount {
        return Err(ProgramError::InsufficientFunds);
    }
    if crate::address::address_eq(from.address(), to.address()) {
        return Ok(());
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
/// [`require_rent_exempt_with`], which reads the live
/// [`crate::sysvar::Rent`] sysvar.
#[inline]
pub fn require_rent_exempt(account: &AccountView<'_>) -> ProgramResult {
    let min = crate::sysvar::rent_exempt_minimum(account.data_len());
    if account.lamports() >= min {
        Ok(())
    } else {
        Err(ProgramError::AccountNotRentExempt)
    }
}

/// Verify that an account is rent-exempt against a live [`crate::sysvar::Rent`] sysvar
/// (RECOMMENDED for reaping-relevant checks).
///
/// The caller reads the sysvar once (`Rent::get()`) and passes it in, so this
/// function adds no syscall of its own, the cost stays where the caller can
/// see it, while using the cluster's *actual* rent parameters. This is the
/// correct form when the cluster may have repriced rent since the constants
/// baked into [`require_rent_exempt`] were set: it uses
/// [`crate::sysvar::Rent::minimum_balance`], which byte-matches the runtime.
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
    // in an inconsistent state if the payer transfer fails, and check the
    // resize preconditions BEFORE the transfer so a refused resize (not
    // writable, live borrow, over the growth limit) cannot leave the
    // top-up behind.
    account.check_resize(new_len)?;
    let min = crate::sysvar::rent_exempt_minimum(new_len);
    let current = account.lamports();

    if current < min {
        // Need more lamports. Transfer BEFORE resize so that if the
        // transfer fails, the account data length is unchanged.
        if let Some(payer) = payer {
            if crate::address::address_eq(account.address(), payer.address()) {
                return Err(ProgramError::InvalidArgument);
            }
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
///
/// The optional payer is debited directly and must be owned by the executing
/// program. An underfunded target cannot pay itself. For a System-owned wallet
/// or PDA payer, use `ResizeWithPayer` (feature `cpi`) instead.
#[inline]
pub fn realloc_checked_with(
    rent: &crate::sysvar::Rent,
    account: &AccountView<'_>,
    new_len: usize,
    payer: Option<&AccountView<'_>>,
) -> ProgramResult {
    // Top-up computed from the live sysvar, not the const snapshot.
    // Check rent BEFORE resizing so a failed payer transfer leaves the
    // account's data length unchanged, and the resize preconditions BEFORE
    // the transfer (same ordering as realloc_checked).
    account.check_resize(new_len)?;
    let min = rent.minimum_balance(new_len);
    let current = account.lamports();

    if current < min {
        if let Some(payer) = payer {
            if crate::address::address_eq(account.address(), payer.address()) {
                return Err(ProgramError::InvalidArgument);
            }
            let deficit = min - current;
            transfer_lamports(payer, account, deficit)?;
        } else {
            return Err(ProgramError::AccountNotRentExempt);
        }
    }

    account.resize(new_len)
}

/// Resize program-owned state, funding only missing rent through System CPI.
///
/// The default invocation reads live rent. Growth is checked against the length
/// at instruction entry before a payer is charged. Newly exposed bytes are zeroed;
/// shrinking retains excess lamports in the account. This does not authorize an
/// application's resize: validate its authority before invoking this builder.
/// `program_id` must be the current entrypoint's program ID.
#[cfg(feature = "cpi")]
pub struct ResizeWithPayer<'a, 'info> {
    pub account: &'a AccountView<'info>,
    pub payer: &'a AccountView<'info>,
    pub system_program: &'a AccountView<'info>,
    pub program_id: &'a Address,
    pub new_len: usize,
}

#[cfg(feature = "cpi")]
impl ResizeWithPayer<'_, '_> {
    /// Fund from a transaction signer using the live Rent sysvar.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Also support a System-owned PDA payer derived by the current program.
    /// Nonempty signer seeds are verified by the SVM, not assumed valid here.
    #[inline]
    pub fn invoke_signed(&self, signers: &[crate::instruction::Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_with_rent(&crate::sysvar::Rent::get()?, signers)
    }

    /// Reuse a Rent value already read from the target cluster in this instruction.
    /// A fabricated or stale rent value can underfund the account; use `invoke`
    /// unless the caller already has the live sysvar.
    #[inline]
    pub fn invoke_signed_with_rent(
        &self,
        rent: &crate::sysvar::Rent,
        signers: &[crate::instruction::Signer<'_, '_>],
    ) -> ProgramResult {
        self.account.require_owned_by(self.program_id)?;
        self.account.require_writable()?;
        self.account.check_resize(self.new_len)?;
        let deficit = rent
            .minimum_balance(self.new_len)
            .saturating_sub(self.account.lamports());
        if deficit > 0 {
            if crate::address::address_eq(self.account.address(), self.payer.address()) {
                return Err(ProgramError::InvalidArgument);
            }
            require_address(self.system_program, &AccountView::SYSTEM_PROGRAM_ID)?;
            if !self.system_program.executable() {
                return Err(ProgramError::IncorrectProgramId);
            }
            self.payer
                .require_owned_by(&AccountView::SYSTEM_PROGRAM_ID)?;
            if !self.payer.is_data_empty() {
                return Err(ProgramError::InvalidAccountData);
            }
            crate::system::Transfer {
                from: self.payer,
                to: self.account,
                lamports: deficit,
            }
            .invoke_signed(signers)?;
        }
        self.account.resize(self.new_len)
    }
}
