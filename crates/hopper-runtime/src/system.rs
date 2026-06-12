//! Hopper-native System Program CPI builders.
//!
//! The API is Hopper-owned (builder pattern over `AccountView` / `Address` /
//! `Signer`) and execution flows through Hopper's checked native CPI semantics.
//!
//! Provides CreateAccount, Transfer, Assign, and Allocate builders.

use crate::account::AccountView;
use crate::address::Address;
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::ProgramResult;

/// System program address: 11111111111111111111111111111111
pub const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

// ---------------------------------------------------------------------

/// Builder for the system program's CreateAccount instruction.
pub struct CreateAccount<'a, 'b> {
    pub from: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

// ---------------------------------------------------------------------

/// Builder for the system program's Transfer instruction.
pub struct Transfer<'a> {
    pub from: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub lamports: u64,
}

impl Transfer<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

// ---------------------------------------------------------------------

/// Builder for the system program's Assign instruction.
pub struct Assign<'a, 'b> {
    pub account: &'a AccountView<'a>,
    pub owner: &'b Address,
}

impl Assign<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

// ---------------------------------------------------------------------

/// Builder for the system program's Allocate instruction.
pub struct Allocate<'a> {
    pub account: &'a AccountView<'a>,
    pub space: u64,
}

impl Allocate<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

/// Legacy module-path re-exports.
pub mod instructions {
    pub use super::{Allocate, Assign, CreateAccount, Transfer};
}
