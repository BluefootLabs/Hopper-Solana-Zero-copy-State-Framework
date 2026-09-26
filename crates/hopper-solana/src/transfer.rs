//! Verify the spend and receipt of a token transfer on chain.
//!
//! A successful CPI is separate from an application's minimum-receipt policy.
//! Capture immediately before the CPI and propagate `verify()` errors with `?`
//! so the transaction rolls back if the outcome is unacceptable. These helpers
//! return errors; catching an error does not undo a successful CPI.

use crate::interface::{InterfaceTokenAccount, TokenProgramKind};
use hopper_runtime::{AccountView, Address, ProgramError};

/// Observed base token amounts, in raw mint units (not UI-scaled units).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenTransferOutcome {
    pub debited: u64,
    pub credited: u64,
}

/// A single-use snapshot bound to the source and destination account views.
///
/// Supports classic SPL Token and Token-2022 base balances. It checks program
/// ownership, account shape, initialized state, expected mint and unchanged token
/// authorities, then requires an exact debit and credit in `min_received..=amount`.
/// No data borrow is retained across CPI. Account references prevent accidentally
/// verifying a different pair of accounts against these balances.
///
/// This verifies net balance changes, not which internal operation caused them.
/// It does not authorize the caller, screen extensions, resolve transfer hooks,
/// or inspect confidential balances. Apply those policies separately. Verify
/// immediately after the intended transfer, before unrelated balance changes.
#[must_use = "call verify after the CPI and propagate any error"]
pub struct TokenTransferSnapshot<'a, 'info> {
    source: &'a AccountView<'info>,
    destination: &'a AccountView<'info>,
    kind: TokenProgramKind,
    mint: Address,
    source_authority: Address,
    destination_authority: Address,
    source_before: u64,
    destination_before: u64,
    amount: u64,
    min_received: u64,
}

fn read(
    account: &AccountView<'_>,
    kind: TokenProgramKind,
    mint: &Address,
) -> Result<(u64, Address), ProgramError> {
    account.require_owned_by(kind.program_id())?;
    let data = account.try_borrow()?;
    let token = InterfaceTokenAccount::from_data(&data, kind)?;
    // Both initialized and frozen are valid base states; a transfer CPI must
    // independently enforce transferability. Reject unknown state tags.
    if !matches!(token.state()?, 1 | 2) {
        return Err(ProgramError::InvalidAccountData);
    }
    token.assert_mint(mint)?;
    Ok((token.amount()?, *token.owner()?))
}

impl<'a, 'info> TokenTransferSnapshot<'a, 'info> {
    /// Capture balances and policy before CPI.
    ///
    /// `expected_mint` must come from the application's validated configuration.
    /// Rejects self-transfers, zero amounts and `min_received > amount`.
    /// Set `min_received == amount` for an exact receipt; explicitly choosing zero
    /// permits a transfer whose entire amount is withheld as a fee.
    pub fn capture(
        source: &'a AccountView<'info>,
        destination: &'a AccountView<'info>,
        expected_mint: &Address,
        amount: u64,
        min_received: u64,
    ) -> Result<Self, ProgramError> {
        if source.address() == destination.address() || amount == 0 || min_received > amount {
            return Err(ProgramError::InvalidArgument);
        }
        let kind = TokenProgramKind::for_account(source)?;
        let (source_before, source_authority) = read(source, kind, expected_mint)?;
        let (destination_before, destination_authority) = read(destination, kind, expected_mint)?;
        if source_before < amount {
            return Err(ProgramError::InsufficientFunds);
        }
        Ok(Self {
            source,
            destination,
            kind,
            mint: *expected_mint,
            source_authority,
            destination_authority,
            source_before,
            destination_before,
            amount,
            min_received,
        })
    }

    /// Re-read the same accounts and enforce the captured transfer policy.
    ///
    /// A policy or authority mismatch returns `InvalidAccountData`. Reversed
    /// balance movement also fails. Propagate the error to roll back the transaction.
    pub fn verify(self) -> Result<TokenTransferOutcome, ProgramError> {
        let (source_after, source_authority) = read(self.source, self.kind, &self.mint)?;
        let (destination_after, destination_authority) =
            read(self.destination, self.kind, &self.mint)?;
        if source_authority != self.source_authority
            || destination_authority != self.destination_authority
        {
            return Err(ProgramError::InvalidAccountData);
        }
        let debited = self
            .source_before
            .checked_sub(source_after)
            .ok_or(ProgramError::InvalidAccountData)?;
        let credited = destination_after
            .checked_sub(self.destination_before)
            .ok_or(ProgramError::InvalidAccountData)?;
        if debited != self.amount || credited < self.min_received || credited > self.amount {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(TokenTransferOutcome { debited, credited })
    }
}
