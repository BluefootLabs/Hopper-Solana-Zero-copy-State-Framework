//! Hopper-native System Program CPI builders.
//!
//! The API is Hopper-owned (builder pattern over `AccountView` / `Address` /
//! `Signer`) and execution flows through Hopper's checked native CPI semantics.
//!
//! Provides the full System program surface: CreateAccount, Transfer,
//! Assign, Allocate, the `*WithSeed` variants, and the durable-nonce
//! family, plus a typed [`NonceState`] reader.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::ProgramResult;

/// System program address: 11111111111111111111111111111111
pub const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

pub use hopper_native::system::{
    NonceState, MAX_SEED_LEN, NONCE_ACCOUNT_LEN, NONCE_STATE_INITIALIZED, NONCE_VERSION_CURRENT,
    RECENT_BLOCKHASHES_ID, RENT_SYSVAR_ID,
};

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

// ---------------------------------------------------------------------
//  WithSeed variants
// ---------------------------------------------------------------------

/// Builder for `CreateAccountWithSeed`.
pub struct CreateAccountWithSeed<'a, 'b> {
    pub from: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub base: &'a AccountView<'a>,
    pub seed: &'b [u8],
    pub lamports: u64,
    pub space: u64,
    pub owner: &'b Address,
}

impl CreateAccountWithSeed<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        let mut data = [0u8; 4 + 32 + 8 + MAX_SEED_LEN + 8 + 8 + 32];
        data[0] = 3;
        let mut n = 4;
        data[n..n + 32].copy_from_slice(self.base.address().as_array());
        n += 32;
        data[n..n + 8].copy_from_slice(&(self.seed.len() as u64).to_le_bytes());
        n += 8;
        data[n..n + self.seed.len()].copy_from_slice(self.seed);
        n += self.seed.len();
        data[n..n + 8].copy_from_slice(&self.lamports.to_le_bytes());
        n += 8;
        data[n..n + 8].copy_from_slice(&self.space.to_le_bytes());
        n += 8;
        data[n..n + 32].copy_from_slice(self.owner.as_array());
        n += 32;

        let accounts = [
            InstructionAccount::writable_signer(self.from.address()),
            InstructionAccount::writable(self.to.address()),
            InstructionAccount::readonly_signer(self.base.address()),
        ];
        let views = [self.from, self.to, self.base];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data[..n],
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for `AllocateWithSeed`.
pub struct AllocateWithSeed<'a, 'b> {
    pub account: &'a AccountView<'a>,
    pub base: &'a AccountView<'a>,
    pub seed: &'b [u8],
    pub space: u64,
    pub owner: &'b Address,
}

impl AllocateWithSeed<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        let mut data = [0u8; 4 + 32 + 8 + MAX_SEED_LEN + 8 + 32];
        data[0] = 9;
        let mut n = 4;
        data[n..n + 32].copy_from_slice(self.base.address().as_array());
        n += 32;
        data[n..n + 8].copy_from_slice(&(self.seed.len() as u64).to_le_bytes());
        n += 8;
        data[n..n + self.seed.len()].copy_from_slice(self.seed);
        n += self.seed.len();
        data[n..n + 8].copy_from_slice(&self.space.to_le_bytes());
        n += 8;
        data[n..n + 32].copy_from_slice(self.owner.as_array());
        n += 32;

        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly_signer(self.base.address()),
        ];
        let views = [self.account, self.base];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data[..n],
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for `AssignWithSeed`.
pub struct AssignWithSeed<'a, 'b> {
    pub account: &'a AccountView<'a>,
    pub base: &'a AccountView<'a>,
    pub seed: &'b [u8],
    pub owner: &'b Address,
}

impl AssignWithSeed<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        let mut data = [0u8; 4 + 32 + 8 + MAX_SEED_LEN + 32];
        data[0] = 10;
        let mut n = 4;
        data[n..n + 32].copy_from_slice(self.base.address().as_array());
        n += 32;
        data[n..n + 8].copy_from_slice(&(self.seed.len() as u64).to_le_bytes());
        n += 8;
        data[n..n + self.seed.len()].copy_from_slice(self.seed);
        n += self.seed.len();
        data[n..n + 32].copy_from_slice(self.owner.as_array());
        n += 32;

        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly_signer(self.base.address()),
        ];
        let views = [self.account, self.base];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data[..n],
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for `TransferWithSeed`.
pub struct TransferWithSeed<'a, 'b> {
    pub from: &'a AccountView<'a>,
    pub base: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub lamports: u64,
    pub from_seed: &'b [u8],
    pub from_owner: &'b Address,
}

impl TransferWithSeed<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.from_seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        let mut data = [0u8; 4 + 8 + 8 + MAX_SEED_LEN + 32];
        data[0] = 11;
        let mut n = 4;
        data[n..n + 8].copy_from_slice(&self.lamports.to_le_bytes());
        n += 8;
        data[n..n + 8].copy_from_slice(&(self.from_seed.len() as u64).to_le_bytes());
        n += 8;
        data[n..n + self.from_seed.len()].copy_from_slice(self.from_seed);
        n += self.from_seed.len();
        data[n..n + 32].copy_from_slice(self.from_owner.as_array());
        n += 32;

        let accounts = [
            InstructionAccount::writable(self.from.address()),
            InstructionAccount::readonly_signer(self.base.address()),
            InstructionAccount::writable(self.to.address()),
        ];
        let views = [self.from, self.base, self.to];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data[..n],
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ---------------------------------------------------------------------
//  Durable nonce family
// ---------------------------------------------------------------------

/// Builder for `AdvanceNonceAccount`.
pub struct AdvanceNonceAccount<'a> {
    pub nonce: &'a AccountView<'a>,
    pub recent_blockhashes: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
}

impl AdvanceNonceAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let data = [4u8, 0, 0, 0];
        let accounts = [
            InstructionAccount::writable(self.nonce.address()),
            InstructionAccount::readonly(self.recent_blockhashes.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.nonce, self.recent_blockhashes, self.authority];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for `WithdrawNonceAccount`.
pub struct WithdrawNonceAccount<'a> {
    pub nonce: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub recent_blockhashes: &'a AccountView<'a>,
    pub rent: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub lamports: u64,
}

impl WithdrawNonceAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 12];
        data[0] = 5;
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());
        let accounts = [
            InstructionAccount::writable(self.nonce.address()),
            InstructionAccount::writable(self.to.address()),
            InstructionAccount::readonly(self.recent_blockhashes.address()),
            InstructionAccount::readonly(self.rent.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [
            self.nonce,
            self.to,
            self.recent_blockhashes,
            self.rent,
            self.authority,
        ];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for `InitializeNonceAccount`.
pub struct InitializeNonceAccount<'a, 'b> {
    pub nonce: &'a AccountView<'a>,
    pub recent_blockhashes: &'a AccountView<'a>,
    pub rent: &'a AccountView<'a>,
    pub authority: &'b Address,
}

impl InitializeNonceAccount<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 36];
        data[0] = 6;
        data[4..36].copy_from_slice(self.authority.as_array());
        let accounts = [
            InstructionAccount::writable(self.nonce.address()),
            InstructionAccount::readonly(self.recent_blockhashes.address()),
            InstructionAccount::readonly(self.rent.address()),
        ];
        let views = [self.nonce, self.recent_blockhashes, self.rent];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for `AuthorizeNonceAccount`.
pub struct AuthorizeNonceAccount<'a, 'b> {
    pub nonce: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub new_authority: &'b Address,
}

impl AuthorizeNonceAccount<'_, '_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 36];
        data[0] = 7;
        data[4..36].copy_from_slice(self.new_authority.as_array());
        let accounts = [
            InstructionAccount::writable(self.nonce.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.nonce, self.authority];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

/// Builder for `UpgradeNonceAccount`.
pub struct UpgradeNonceAccount<'a> {
    pub nonce: &'a AccountView<'a>,
}

impl UpgradeNonceAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        let data = [12u8, 0, 0, 0];
        let accounts = [InstructionAccount::writable(self.nonce.address())];
        let views = [self.nonce];
        let instruction = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };
        crate::cpi::invoke_signed(&instruction, &views, &[])
    }
}

/// Legacy module-path re-exports.
pub mod instructions {
    pub use super::{
        AdvanceNonceAccount, Allocate, AllocateWithSeed, Assign, AssignWithSeed,
        AuthorizeNonceAccount, CreateAccount, CreateAccountWithSeed, InitializeNonceAccount,
        Transfer, TransferWithSeed, UpgradeNonceAccount, WithdrawNonceAccount,
    };
}
