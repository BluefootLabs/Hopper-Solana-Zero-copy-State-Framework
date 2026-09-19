//! SPL Token program CPI instructions.
//!
//! Provides Transfer, MintTo, Burn, CloseAccount, Approve, and Revoke
//! builders that invoke the SPL Token program via `sol_invoke_signed_c`.

use crate::account_view::AccountView;
use crate::address::Address;
use crate::instruction::{CpiAccount, Signer};
use crate::ProgramResult;

/// SPL Token program address.
pub const TOKEN_PROGRAM_ID: Address =
    crate::address!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// ---------------------------------------------------------------------

/// Builder for SPL Token Transfer (instruction index 3).
pub struct Transfer<'a> {
    pub from: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: u64,
}

impl Transfer<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 3;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.from),
            CpiAccount::from(self.to),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, 0b011, 0b100, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token MintTo (instruction index 7).
pub struct MintTo<'a> {
    pub mint: &'a AccountView<'a>,
    pub account: &'a AccountView<'a>,
    pub mint_authority: &'a AccountView<'a>,
    pub amount: u64,
}

impl MintTo<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 7;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.mint),
            CpiAccount::from(self.account),
            CpiAccount::from(self.mint_authority),
        ];

        invoke_token(&data, &accounts, 0b011, 0b100, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token Burn (instruction index 8).
pub struct Burn<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: u64,
}

impl Burn<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 8;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.account),
            CpiAccount::from(self.mint),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, 0b011, 0b100, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token CloseAccount (instruction index 9).
pub struct CloseAccount<'a> {
    pub account: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
}

impl CloseAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let data = [9u8];

        let accounts = [
            CpiAccount::from(self.account),
            CpiAccount::from(self.destination),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, 0b011, 0b100, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token Approve (instruction index 4).
pub struct Approve<'a> {
    pub source: &'a AccountView<'a>,
    pub delegate: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: u64,
}

impl Approve<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 4;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.source),
            CpiAccount::from(self.delegate),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, 0b001, 0b100, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token Revoke (instruction index 5).
pub struct Revoke<'a> {
    pub source: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
}

impl Revoke<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let data = [5u8];

        let accounts = [
            CpiAccount::from(self.source),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, 0b01, 0b10, signers)
    }
}

// ---------------------------------------------------------------------

#[inline]
fn invoke_token<'a, const ACCOUNTS: usize>(
    data: &[u8],
    accounts: &[CpiAccount<'a>; ACCOUNTS],
    writable_mask: usize,
    signer_mask: usize,
    signers: &[Signer<'_, '_>],
) -> ProgramResult {
    crate::cpi::invoke_specialized_signed(
        &TOKEN_PROGRAM_ID,
        data,
        accounts,
        writable_mask,
        signer_mask,
        signers,
    )
}

/// Compatibility re-exports matching `pinocchio_token::instructions::*`.
pub mod instructions {
    pub use super::{Approve, Burn, CloseAccount, MintTo, Revoke, Transfer};
}
