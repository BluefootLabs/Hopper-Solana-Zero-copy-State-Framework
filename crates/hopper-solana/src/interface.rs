//! `InterfaceAccount` / `InterfaceMint` — Token + Token-2022 polymorphism.
//!
//! Quasar's headline DX win is `InterfaceAccount<Token>`: a single wrapper
//! that accepts an account owned by either the SPL Token program or the
//! Token-2022 program, performs one owner check at parse time, and
//! exposes the unified mint / owner / amount / state surface.
//!
//! Hopper exposes the equivalent shape here:
//!
//! - [`InterfaceTokenAccount`] — token-account-shaped overlay for either
//!   SPL Token or Token-2022.
//! - [`InterfaceMint`] — mint-shaped overlay for either program.
//! - [`TokenProgramKind`] — discriminates which program owns the account.
//!
//! The first 165 bytes of an SPL Token Account and the first 165 bytes
//! of a Token-2022 token account share the same on-disk layout (mint,
//! owner, amount, delegate, state, …), so the existing zero-copy
//! readers in [`crate::token`] and [`crate::mint`] work for both. This
//! module adds the validation gate (owner ∈ {Token, Token-2022}) plus
//! a polymorphic `transfer_checked` CPI helper that dispatches to the
//! correct program.

use hopper_runtime::account::AccountView;
use hopper_runtime::address::Address;
use hopper_runtime::error::ProgramError;
use hopper_runtime::instruction::{InstructionAccount, InstructionView, Signer};
use hopper_runtime::ProgramResult;

use crate::constants::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};

/// Which token program owns this account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenProgramKind {
    /// SPL Token (`TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`).
    Spl,
    /// SPL Token-2022 (`TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`).
    Token2022,
}

impl TokenProgramKind {
    /// Resolve the program-id address backing this kind.
    #[inline(always)]
    pub const fn program_id(self) -> &'static Address {
        match self {
            TokenProgramKind::Spl => &TOKEN_PROGRAM_ID,
            TokenProgramKind::Token2022 => &TOKEN_2022_PROGRAM_ID,
        }
    }

    /// Match an account's owner against the two supported programs.
    ///
    /// Returns `Err(IncorrectProgramId)` if the account is owned by
    /// any other program.
    #[inline(always)]
    pub fn from_owner(owner: &Address) -> Result<Self, ProgramError> {
        if owner == &TOKEN_PROGRAM_ID {
            Ok(TokenProgramKind::Spl)
        } else if owner == &TOKEN_2022_PROGRAM_ID {
            Ok(TokenProgramKind::Token2022)
        } else {
            Err(ProgramError::IncorrectProgramId)
        }
    }

    /// Resolve the kind from an [`AccountView`] using its owner.
    ///
    /// Wrapper over [`AccountView::owned_by`] that stays on the safe
    /// (no-unsafe) side of the runtime API surface.
    #[inline(always)]
    pub fn for_account(view: &AccountView) -> Result<Self, ProgramError> {
        if view.owned_by(&TOKEN_PROGRAM_ID) {
            Ok(TokenProgramKind::Spl)
        } else if view.owned_by(&TOKEN_2022_PROGRAM_ID) {
            Ok(TokenProgramKind::Token2022)
        } else {
            Err(ProgramError::IncorrectProgramId)
        }
    }
}

/// Polymorphic SPL Token / Token-2022 token-account overlay.
///
/// Construct via [`InterfaceTokenAccount::from_data`] using a borrowed
/// view of an account body that has already been ownership-checked
/// via [`TokenProgramKind::for_account`]. The constructor validates
/// the body is at least [`crate::token::TOKEN_ACCOUNT_LEN`] (165) bytes.
///
/// The reader methods delegate to [`crate::token`], which is correct
/// for both programs because the first 165 bytes of a Token-2022
/// account match the SPL Token layout exactly.
///
/// ```ignore
/// let kind = TokenProgramKind::for_account(&view)?;
/// let data = view.try_borrow()?;
/// let token = InterfaceTokenAccount::from_data(&data, kind)?;
/// let mint = token.mint()?;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct InterfaceTokenAccount<'a> {
    /// Raw account body. Always at least 165 bytes.
    data: &'a [u8],
    /// Which program owns the account.
    pub kind: TokenProgramKind,
}

impl<'a> InterfaceTokenAccount<'a> {
    /// Wrap a previously-borrowed account body.
    ///
    /// Caller is responsible for confirming `kind` matches the
    /// account's actual owner — usually by calling
    /// [`TokenProgramKind::for_account`] beforehand.
    pub fn from_data(
        data: &'a [u8],
        kind: TokenProgramKind,
    ) -> Result<Self, ProgramError> {
        if data.len() < crate::token::TOKEN_ACCOUNT_LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self { data, kind })
    }

    /// The raw account body. Always at least 165 bytes.
    #[inline(always)]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The mint pubkey.
    #[inline(always)]
    pub fn mint(&self) -> Result<&'a Address, ProgramError> {
        crate::token::token_account_mint(self.data)
    }

    /// The owner pubkey of the token account (the user wallet, not the
    /// program owning the account).
    #[inline(always)]
    pub fn owner(&self) -> Result<&'a Address, ProgramError> {
        crate::token::token_account_owner(self.data)
    }

    /// The token amount.
    #[inline(always)]
    pub fn amount(&self) -> Result<u64, ProgramError> {
        crate::token::token_account_amount(self.data)
    }

    /// The state byte (`0` = uninitialised, `1` = initialised, `2` = frozen).
    #[inline(always)]
    pub fn state(&self) -> Result<u8, ProgramError> {
        crate::token::token_account_state(self.data)
    }

    /// Convenience: assert the account is initialised.
    #[inline(always)]
    pub fn assert_initialized(&self) -> Result<(), ProgramError> {
        crate::token::check_token_initialized(self.data)
    }

    /// Convenience: assert the wallet owner matches.
    #[inline(always)]
    pub fn assert_owner(&self, expected: &Address) -> Result<(), ProgramError> {
        crate::token::check_token_owner(self.data, expected)
    }

    /// Convenience: assert the mint matches.
    #[inline(always)]
    pub fn assert_mint(&self, expected: &Address) -> Result<(), ProgramError> {
        crate::token::check_token_mint(self.data, expected)
    }
}

/// Polymorphic SPL Token / Token-2022 mint overlay.
///
/// SPL Mint and Token-2022 base mint share the same first 82 bytes
/// (mint authority COption, supply, decimals, is_init flag, freeze
/// authority COption). Token-2022 extension bytes begin at offset
/// 165; this wrapper exposes only the base layout. Use
/// [`crate::token2022_ext`] helpers for extension parsing.
#[derive(Debug, Clone, Copy)]
pub struct InterfaceMint<'a> {
    data: &'a [u8],
    /// Which program owns the mint.
    pub kind: TokenProgramKind,
}

impl<'a> InterfaceMint<'a> {
    /// Wrap a previously-borrowed mint body. Caller verifies `kind`
    /// using [`TokenProgramKind::for_account`].
    pub fn from_data(
        data: &'a [u8],
        kind: TokenProgramKind,
    ) -> Result<Self, ProgramError> {
        if data.len() < crate::mint::MINT_LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self { data, kind })
    }

    /// The raw mint bytes.
    #[inline(always)]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The mint supply.
    #[inline(always)]
    pub fn supply(&self) -> Result<u64, ProgramError> {
        crate::mint::mint_supply(self.data)
    }

    /// The mint decimals.
    #[inline(always)]
    pub fn decimals(&self) -> Result<u8, ProgramError> {
        crate::mint::mint_decimals(self.data)
    }

    /// The mint authority, if set.
    #[inline(always)]
    pub fn authority(&self) -> Result<Option<&'a Address>, ProgramError> {
        crate::mint::mint_authority(self.data)
    }

    /// The freeze authority, if set.
    #[inline(always)]
    pub fn freeze_authority(&self) -> Result<Option<&'a Address>, ProgramError> {
        crate::mint::mint_freeze_authority(self.data)
    }

    /// Convenience: assert the mint is initialised.
    #[inline(always)]
    pub fn assert_initialized(&self) -> Result<(), ProgramError> {
        crate::mint::check_mint_initialized(self.data)
    }
}

// ── Polymorphic CPI helpers ──────────────────────────────────────────

/// Polymorphic `TransferChecked` CPI that dispatches to the program
/// that owns the source token account.
///
/// The instruction layout is shared between SPL Token and Token-2022:
/// `[12u8, amount: u64 LE, decimals: u8]` with three accounts (source,
/// mint, destination, authority). This helper picks the right program
/// id based on the source account's owner and forwards through the
/// runtime's checked CPI path.
#[inline]
pub fn interface_transfer_checked<'a>(
    source: &'a AccountView,
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
) -> ProgramResult {
    interface_transfer_checked_signed(
        source,
        mint,
        destination,
        authority,
        amount,
        decimals,
        &[],
    )
}

/// PDA-signing variant of [`interface_transfer_checked`].
#[inline]
pub fn interface_transfer_checked_signed<'a>(
    source: &'a AccountView,
    mint: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
    signers: &[Signer],
) -> ProgramResult {
    let kind = TokenProgramKind::for_account(source)?;

    let mut data = [0u8; 10];
    data[0] = 12; // TransferChecked discriminator (shared between Token and Token-2022)
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data[9] = decimals;

    let accounts = [
        InstructionAccount::writable(source.address()),
        InstructionAccount::readonly(mint.address()),
        InstructionAccount::writable(destination.address()),
        InstructionAccount::readonly_signer(authority.address()),
    ];
    let views = [source, mint, destination, authority];
    let instruction = InstructionView {
        program_id: kind.program_id(),
        data: &data,
        accounts: &accounts,
    };

    hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_program_kind_from_owner_matches_known_programs() {
        assert_eq!(
            TokenProgramKind::from_owner(&TOKEN_PROGRAM_ID).unwrap(),
            TokenProgramKind::Spl,
        );
        assert_eq!(
            TokenProgramKind::from_owner(&TOKEN_2022_PROGRAM_ID).unwrap(),
            TokenProgramKind::Token2022,
        );
    }

    #[test]
    fn token_program_kind_from_owner_rejects_other_programs() {
        let other = Address::new_from_array([7u8; 32]);
        assert!(matches!(
            TokenProgramKind::from_owner(&other),
            Err(ProgramError::IncorrectProgramId),
        ));
    }

    #[test]
    fn token_program_kind_program_id_is_stable() {
        assert_eq!(
            TokenProgramKind::Spl.program_id(),
            &TOKEN_PROGRAM_ID,
        );
        assert_eq!(
            TokenProgramKind::Token2022.program_id(),
            &TOKEN_2022_PROGRAM_ID,
        );
    }
}
