//! TEMPORARY: backend facade for SPL Token CPI builders.
//!
//! This module keeps Hopper-owned instruction semantics while execution still
//! flows through the active backend substrate. It will be replaced by
//! Hopper-native instruction builders once the substrate-facing builders are
//! finalized.
//!
//! Semantic CPI facades: the API is Hopper-owned (builder pattern over
//! `AccountView` / `Signer`), while execution is delegated through Hopper's
//! checked CPI semantics.
//!
//! Provides Transfer, MintTo, Burn, CloseAccount, Approve, Revoke, and
//! InitializeAccount builders.

use crate::account::AccountView;
use crate::address::Address;
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::ProgramResult;

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
            InstructionAccount::writable(self.from.address()),
            InstructionAccount::writable(self.to.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.from, self.to, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
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
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly_signer(self.mint_authority.address()),
        ];
        let views = [self.mint, self.account, self.mint_authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
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
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.account, self.mint, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
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
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.destination.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.account, self.destination, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
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
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.delegate.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.source, self.delegate, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
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
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.source, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── InitializeAccount ────────────────────────────────────────────────

/// Builder for SPL Token InitializeAccount (instruction index 1).
pub struct InitializeAccount<'a> {
    pub account: &'a AccountView,
    pub mint: &'a AccountView,
    pub owner: &'a AccountView,
    pub rent_sysvar: &'a AccountView,
}

impl InitializeAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        let data = [1u8];
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.owner.address()),
            InstructionAccount::readonly(self.rent_sysvar.address()),
        ];
        let views = [self.account, self.mint, self.owner, self.rent_sysvar];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke(&instruction, &views)
    }
}

/// SPL Token program address.
pub const TOKEN_PROGRAM_ID: Address = Address::new_from_array(
    five8_const::decode_32_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
);

/// Compatibility re-exports.
pub mod instructions {
    pub use super::{Transfer, MintTo, Burn, CloseAccount, Approve, Revoke, InitializeAccount};
}
