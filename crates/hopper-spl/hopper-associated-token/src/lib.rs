//! Hopper-owned associated token account helpers and CPI builders.
//!
//! Thin first-class Hopper wrappers over ATA derivation/verification helpers
//! and ATA program instruction builders.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use hopper_runtime::instruction::{InstructionAccount, InstructionView, Signer};
use hopper_runtime::{AccountView, ProgramResult};

pub use hopper_solana::constants::ATA_PROGRAM_ID;

#[cfg(target_os = "solana")]
pub use hopper_solana::ata::{
    derive_ata,
    derive_ata_2022,
    derive_ata_for_program,
    verify_ata,
    verify_ata_2022,
    verify_ata_any,
};

/// Builder for Associated Token Account `Create` (instruction 0).
pub struct Create<'a> {
    pub payer: &'a AccountView,
    pub associated_account: &'a AccountView,
    pub wallet: &'a AccountView,
    pub mint: &'a AccountView,
    pub system_program: &'a AccountView,
    pub token_program: &'a AccountView,
}

impl Create<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let data = [0u8];
        let accounts = [
            InstructionAccount::writable_signer(self.payer.address()),
            InstructionAccount::writable(self.associated_account.address()),
            InstructionAccount::readonly(self.wallet.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.system_program.address()),
            InstructionAccount::readonly(self.token_program.address()),
        ];
        let views = [
            self.payer,
            self.associated_account,
            self.wallet,
            self.mint,
            self.system_program,
            self.token_program,
        ];
        let instruction = InstructionView {
            program_id: &ATA_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for Associated Token Account `CreateIdempotent` (instruction 1).
pub struct CreateIdempotent<'a> {
    pub payer: &'a AccountView,
    pub associated_account: &'a AccountView,
    pub wallet: &'a AccountView,
    pub mint: &'a AccountView,
    pub system_program: &'a AccountView,
    pub token_program: &'a AccountView,
}

impl CreateIdempotent<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let data = [1u8];
        let accounts = [
            InstructionAccount::writable_signer(self.payer.address()),
            InstructionAccount::writable(self.associated_account.address()),
            InstructionAccount::readonly(self.wallet.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.system_program.address()),
            InstructionAccount::readonly(self.token_program.address()),
        ];
        let views = [
            self.payer,
            self.associated_account,
            self.wallet,
            self.mint,
            self.system_program,
            self.token_program,
        ];
        let instruction = InstructionView {
            program_id: &ATA_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for ATA `RecoverNested` (instruction 2).
pub struct RecoverNested<'a> {
    pub nested_associated_account: &'a AccountView,
    pub nested_token_mint: &'a AccountView,
    pub destination_associated_account: &'a AccountView,
    pub owner_associated_account: &'a AccountView,
    pub owner_token_mint: &'a AccountView,
    pub wallet: &'a AccountView,
    pub token_program: &'a AccountView,
}

impl RecoverNested<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let data = [2u8];
        let accounts = [
            InstructionAccount::writable(self.nested_associated_account.address()),
            InstructionAccount::readonly(self.nested_token_mint.address()),
            InstructionAccount::writable(self.destination_associated_account.address()),
            InstructionAccount::readonly(self.owner_associated_account.address()),
            InstructionAccount::readonly(self.owner_token_mint.address()),
            InstructionAccount::writable_signer(self.wallet.address()),
            InstructionAccount::readonly(self.token_program.address()),
        ];
        let views = [
            self.nested_associated_account,
            self.nested_token_mint,
            self.destination_associated_account,
            self.owner_associated_account,
            self.owner_token_mint,
            self.wallet,
            self.token_program,
        ];
        let instruction = InstructionView {
            program_id: &ATA_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

pub mod instructions {
    pub use super::{Create, CreateIdempotent, RecoverNested};
}