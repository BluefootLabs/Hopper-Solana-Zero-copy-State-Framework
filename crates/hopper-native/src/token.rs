//! SPL Token program CPI instructions.
//!
//! Provides Transfer, MintTo, Burn, CloseAccount, Approve, and Revoke
//! builders that invoke the SPL Token program via `sol_invoke_signed_c`.

use crate::account_view::AccountView;
use crate::address::Address;
use crate::instruction::{CpiAccount, Signer};
use crate::ProgramResult;

/// SPL Token program address.
pub const TOKEN_PROGRAM_ID: Address = crate::address!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// ── Transfer ─────────────────────────────────────────────────────────

/// Builder for SPL Token Transfer (instruction index 3).
pub struct Transfer<'a> {
    pub from: &'a AccountView,
    pub to: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

impl Transfer<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 3;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.from),
            CpiAccount::from(self.to),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, signers)
    }
}

// ── MintTo ───────────────────────────────────────────────────────────

/// Builder for SPL Token MintTo (instruction index 7).
pub struct MintTo<'a> {
    pub mint: &'a AccountView,
    pub account: &'a AccountView,
    pub mint_authority: &'a AccountView,
    pub amount: u64,
}

impl MintTo<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 7;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.mint),
            CpiAccount::from(self.account),
            CpiAccount::from(self.mint_authority),
        ];

        invoke_token(&data, &accounts, signers)
    }
}

// ── Burn ─────────────────────────────────────────────────────────────

/// Builder for SPL Token Burn (instruction index 8).
pub struct Burn<'a> {
    pub account: &'a AccountView,
    pub mint: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

impl Burn<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 8;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.account),
            CpiAccount::from(self.mint),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, signers)
    }
}

// ── CloseAccount ─────────────────────────────────────────────────────

/// Builder for SPL Token CloseAccount (instruction index 9).
pub struct CloseAccount<'a> {
    pub account: &'a AccountView,
    pub destination: &'a AccountView,
    pub authority: &'a AccountView,
}

impl CloseAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let data = [9u8];

        let accounts = [
            CpiAccount::from(self.account),
            CpiAccount::from(self.destination),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, signers)
    }
}

// ── Approve ──────────────────────────────────────────────────────────

/// Builder for SPL Token Approve (instruction index 4).
pub struct Approve<'a> {
    pub source: &'a AccountView,
    pub delegate: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

impl Approve<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 4;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            CpiAccount::from(self.source),
            CpiAccount::from(self.delegate),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, signers)
    }
}

// ── Revoke ───────────────────────────────────────────────────────────

/// Builder for SPL Token Revoke (instruction index 5).
pub struct Revoke<'a> {
    pub source: &'a AccountView,
    pub authority: &'a AccountView,
}

impl Revoke<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let data = [5u8];

        let accounts = [
            CpiAccount::from(self.source),
            CpiAccount::from(self.authority),
        ];

        invoke_token(&data, &accounts, signers)
    }
}

// ── Internal helper ──────────────────────────────────────────────────

#[inline]
fn invoke_token(
    data: &[u8],
    accounts: &[CpiAccount],
    signers: &[Signer],
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let ix = crate::instruction::InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data,
            accounts: &[],
        };
        let result = unsafe {
            crate::syscalls::sol_invoke_signed_c(
                &ix as *const _ as *const u8,
                accounts.as_ptr() as *const u8,
                accounts.len() as u64,
                signers.as_ptr() as *const u8,
                signers.len() as u64,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(crate::ProgramError::from(result))
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (data, accounts, signers);
        Ok(())
    }
}

/// Compatibility re-exports matching `pinocchio_token::instructions::*`.
pub mod instructions {
    pub use super::{Transfer, MintTo, Burn, CloseAccount, Approve, Revoke};
}
