//! Hopper-owned Token-2022 builder and screening surface.
//!
//! Thin first-class Hopper wrappers over the canonical runtime builders,
//! plus Token-2022 extension screening re-exports from `hopper-solana`.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use hopper_runtime::instruction::{InstructionAccount, InstructionView, Signer};
use hopper_runtime::{AccountView, ProgramResult};

pub use hopper_solana::constants::TOKEN_2022_PROGRAM_ID;
pub use hopper_solana::mint::{
    check_mint_authority,
    check_mint_initialized,
    mint_authority,
    mint_decimals,
    mint_freeze_authority,
    mint_supply,
    MINT_LEN,
};
pub use hopper_solana::token::{
    check_not_frozen,
    check_token_balance_gte,
    check_token_initialized,
    check_token_mint,
    check_token_owner,
    token_account_amount,
    token_account_mint,
    token_account_owner,
    token_account_state,
    TOKEN_ACCOUNT_LEN,
};
pub use hopper_solana::token2022_ext::{
    check_no_confidential_transfer,
    check_no_permanent_delegate,
    check_no_transfer_fee,
    check_no_transfer_hook,
    check_safe_token_2022_mint,
    check_transferable,
    find_extension_data,
    mint_has_extension,
    read_transfer_fee_config,
    token_has_extension,
    TransferFeeConfig,
    EXT_CONFIDENTIAL_TRANSFER_ACCOUNT,
    EXT_CONFIDENTIAL_TRANSFER_MINT,
    EXT_CPI_GUARD,
    EXT_DEFAULT_ACCOUNT_STATE,
    EXT_GROUP_MEMBER_POINTER,
    EXT_GROUP_POINTER,
    EXT_IMMUTABLE_OWNER,
    EXT_INTEREST_BEARING,
    EXT_MEMO_TRANSFER,
    EXT_METADATA_POINTER,
    EXT_MINT_CLOSE_AUTHORITY,
    EXT_NON_TRANSFERABLE,
    EXT_PERMANENT_DELEGATE,
    EXT_TOKEN_METADATA,
    EXT_TRANSFER_FEE_AMOUNT,
    EXT_TRANSFER_FEE_CONFIG,
    EXT_TRANSFER_HOOK,
    MINT_BASE_SIZE,
    TOKEN_ACCOUNT_BASE_SIZE,
};

/// Builder for Token-2022 Transfer (instruction index 3).
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
            program_id: &TOKEN_2022_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for Token-2022 MintTo (instruction index 7).
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
            program_id: &TOKEN_2022_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for Token-2022 Burn (instruction index 8).
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
            program_id: &TOKEN_2022_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for Token-2022 CloseAccount (instruction index 9).
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
            program_id: &TOKEN_2022_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for Token-2022 Approve (instruction index 4).
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
            program_id: &TOKEN_2022_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for Token-2022 Revoke (instruction index 5).
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
            program_id: &TOKEN_2022_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for Token-2022 InitializeAccount (instruction index 1).
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
            program_id: &TOKEN_2022_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        hopper_runtime::cpi::invoke(&instruction, &views)
    }
}

pub mod instructions {
    pub use super::{Approve, Burn, CloseAccount, InitializeAccount, MintTo, Revoke, Transfer};
}