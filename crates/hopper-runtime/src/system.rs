//! TEMPORARY: backend facade for System Program CPI builders.
//!
//! This module keeps Hopper-owned instruction semantics while execution still
//! flows through the active backend substrate. It will be replaced by
//! Hopper-native builders once the system-instruction surface is fully owned.
//!
//! Semantic CPI facades: the API is Hopper-owned (builder pattern over
//! `AccountView` / `Address` / `Signer`), while execution is delegated to the
//! active backend through Hopper's checked CPI semantics.
//!
//! Provides CreateAccount, Transfer, Assign, and Allocate builders.

use crate::account::AccountView;
use crate::address::Address;
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::ProgramResult;

/// System program address: 11111111111111111111111111111111
pub const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

// ── CreateAccount ────────────────────────────────────────────────────

/// Builder for the system program's CreateAccount instruction.
pub struct CreateAccount<'a, 'b> {
    pub from: &'a AccountView,
    pub to: &'a AccountView,
    pub lamports: u64,
    pub space: u64,
    pub owner: &'b Address,
}

impl CreateAccount<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 52];
        // index 0 = CreateAccount (already zero)
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());
        data[12..20].copy_from_slice(&self.space.to_le_bytes());
        data[20..52].copy_from_slice(self.owner.as_array());

        let accounts = [
            InstructionAccount::writable_signer(self.from.address()),
            InstructionAccount::writable_signer(self.to.address()),
        ];
        let views = [self.from, self.to];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── Transfer ─────────────────────────────────────────────────────────

/// Builder for the system program's Transfer instruction.
pub struct Transfer<'a> {
    pub from: &'a AccountView,
    pub to: &'a AccountView,
    pub lamports: u64,
}

impl Transfer<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 12];
        data[0] = 2;
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());

        let accounts = [
            InstructionAccount::writable_signer(self.from.address()),
            InstructionAccount::writable(self.to.address()),
        ];
        let views = [self.from, self.to];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── Assign ───────────────────────────────────────────────────────────

/// Builder for the system program's Assign instruction.
pub struct Assign<'a, 'b> {
    pub account: &'a AccountView,
    pub owner: &'b Address,
}

impl Assign<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 36];
        data[0] = 1;
        data[4..36].copy_from_slice(self.owner.as_array());

        let accounts = [InstructionAccount::writable_signer(self.account.address())];
        let views = [self.account];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── Allocate ─────────────────────────────────────────────────────────

/// Builder for the system program's Allocate instruction.
pub struct Allocate<'a> {
    pub account: &'a AccountView,
    pub space: u64,
}

impl Allocate<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 12];
        data[0] = 8;
        data[4..12].copy_from_slice(&self.space.to_le_bytes());

        let accounts = [InstructionAccount::writable_signer(self.account.address())];
        let views = [self.account];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Compatibility re-exports.
pub mod instructions {
    pub use super::{CreateAccount, Transfer, Assign, Allocate};
}
