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

/// Byte-exact instruction-data encoders for the System program CPI wire
/// format. System instruction discriminators are 4-byte `u32`
/// little-endian tags (only the low byte is nonzero for these instructions).
///
/// # Why this module is `pub`
///
/// The fixed-size System builders — [`CreateAccount`], [`Transfer`],
/// [`Assign`], [`Allocate`], and the entire durable-nonce family
/// ([`AdvanceNonceAccount`], [`WithdrawNonceAccount`],
/// [`InitializeNonceAccount`], [`AuthorizeNonceAccount`],
/// [`UpgradeNonceAccount`]) — each construct their instruction-data buffer by
/// calling exactly one of these functions before handing the bytes to
/// [`crate::cpi`]. They are the shipped source of truth for the fixed-size
/// System wire formats — the exact bytes that leave the program on a CPI.
///
/// They are exposed as `#[doc(hidden)] pub` for one reason: so the Kani layout
/// proofs in the `hopper-token` crate can call the shipped encoders directly
/// and prove — over fully symbolic inputs — that the emitted bytes carry the
/// canonical 4-byte discriminator, field offsets, endianness, and total
/// length. This is deliberately **not** a stability surface: the module is
/// `#[doc(hidden)]` and may change at any time. Each function is
/// `#[inline(always)]`, so delegating to it is zero-cost and byte-identical to
/// the previous inline construction.
///
/// Only the variable-length `*WithSeed` instructions
/// ([`CreateAccountWithSeed`], [`AllocateWithSeed`], [`AssignWithSeed`],
/// [`TransferWithSeed`]) are outside this proof surface: each splices a
/// runtime `seed` slice of caller-chosen length into the middle of its
/// buffer, so its total length is not fixed and it is built inline by its
/// builder. The durable-nonce family, by contrast, is entirely fixed-size and
/// seed-free (its authority pubkeys and lamports travel at fixed offsets), so
/// it is modelled here and proven byte-for-byte like the other fixed-size
/// instructions.
#[doc(hidden)]
pub mod encoders {
    /// `CreateAccount { lamports, space, owner }` —
    /// `[0u32 LE][lamports: u64 LE][space: u64 LE][owner: 32 bytes]`
    /// (52 bytes).
    #[inline(always)]
    pub fn encode_create_account(lamports: u64, space: u64, owner: &[u8; 32]) -> [u8; 52] {
        let mut data = [0u8; 52];
        data[0..4].copy_from_slice(&0u32.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data[12..20].copy_from_slice(&space.to_le_bytes());
        data[20..52].copy_from_slice(owner);
        data
    }

    /// `Transfer { lamports }` — `[2u32 LE][lamports: u64 LE]` (12 bytes).
    #[inline(always)]
    pub fn encode_transfer(lamports: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&2u32.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data
    }

    /// `Assign { owner }` — `[1u32 LE][owner: 32 bytes]` (36 bytes).
    #[inline(always)]
    pub fn encode_assign(owner: &[u8; 32]) -> [u8; 36] {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..36].copy_from_slice(owner);
        data
    }

    /// `Allocate { space }` — `[8u32 LE][space: u64 LE]` (12 bytes).
    #[inline(always)]
    pub fn encode_allocate(space: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&8u32.to_le_bytes());
        data[4..12].copy_from_slice(&space.to_le_bytes());
        data
    }

    // ── Durable-nonce family (fixed-size, seed-free) ────────────────
    // Canonical `SystemInstruction` tags: AdvanceNonceAccount = 4,
    // WithdrawNonceAccount = 5, InitializeNonceAccount = 6,
    // AuthorizeNonceAccount = 7, UpgradeNonceAccount = 12.

    /// `AdvanceNonceAccount` — `[4u32 LE]` (4 bytes). No instruction-data
    /// fields; the nonce/blockhashes/authority travel in the account-meta
    /// list.
    #[inline(always)]
    pub fn encode_advance_nonce_account() -> [u8; 4] {
        4u32.to_le_bytes()
    }

    /// `WithdrawNonceAccount { lamports }` — `[5u32 LE][lamports: u64 LE]`
    /// (12 bytes).
    #[inline(always)]
    pub fn encode_withdraw_nonce_account(lamports: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&5u32.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data
    }

    /// `InitializeNonceAccount { authority }` —
    /// `[6u32 LE][authority: 32 bytes]` (36 bytes).
    #[inline(always)]
    pub fn encode_initialize_nonce_account(authority: &[u8; 32]) -> [u8; 36] {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&6u32.to_le_bytes());
        data[4..36].copy_from_slice(authority);
        data
    }

    /// `AuthorizeNonceAccount { new_authority }` —
    /// `[7u32 LE][new_authority: 32 bytes]` (36 bytes).
    #[inline(always)]
    pub fn encode_authorize_nonce_account(new_authority: &[u8; 32]) -> [u8; 36] {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&7u32.to_le_bytes());
        data[4..36].copy_from_slice(new_authority);
        data
    }

    /// `UpgradeNonceAccount` — `[12u32 LE]` (4 bytes). No instruction-data
    /// fields; the nonce account travels in the account-meta list.
    #[inline(always)]
    pub fn encode_upgrade_nonce_account() -> [u8; 4] {
        12u32.to_le_bytes()
    }
}

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
        let data =
            encoders::encode_create_account(self.lamports, self.space, self.owner.as_array());

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
        let data = encoders::encode_transfer(self.lamports);

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
        let data = encoders::encode_assign(self.owner.as_array());

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
        let data = encoders::encode_allocate(self.space);

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
        let data = encoders::encode_advance_nonce_account();
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
        let data = encoders::encode_withdraw_nonce_account(self.lamports);
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
        let data = encoders::encode_initialize_nonce_account(self.authority.as_array());
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
        let data = encoders::encode_authorize_nonce_account(self.new_authority.as_array());
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
        let data = encoders::encode_upgrade_nonce_account();
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

#[cfg(test)]
mod tests {
    //! Byte-identity guard for the extracted [`encoders`] module: each shipped
    //! System encoder must reproduce the exact bytes the builders wrote inline
    //! before the refactor. The System program dispatches on the 4-byte `u32`
    //! LE discriminator, so a wrong tag or offset silently routes to a
    //! different instruction.
    use super::*;

    const OWNER: [u8; 32] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31,
    ];

    #[test]
    fn create_account_encoder_matches_pre_refactor_golden_bytes() {
        // Pre-refactor inline bytes: [0,0,0,0][lamports LE][space LE][owner].
        let d = encoders::encode_create_account(1, 165, &OWNER);
        assert_eq!(d.len(), 52);
        assert_eq!(&d[0..4], &[0, 0, 0, 0]);
        assert_eq!(&d[4..12], &1u64.to_le_bytes());
        assert_eq!(&d[12..20], &165u64.to_le_bytes());
        assert_eq!(&d[20..52], &OWNER);
    }

    #[test]
    fn transfer_encoder_matches_pre_refactor_golden_bytes() {
        // disc 2 (u32 LE) then lamports = 1 (u64 LE).
        assert_eq!(
            encoders::encode_transfer(1),
            [2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn assign_encoder_matches_pre_refactor_golden_bytes() {
        let d = encoders::encode_assign(&OWNER);
        assert_eq!(d.len(), 36);
        assert_eq!(&d[0..4], &[1, 0, 0, 0]);
        assert_eq!(&d[4..36], &OWNER);
    }

    #[test]
    fn allocate_encoder_matches_pre_refactor_golden_bytes() {
        // disc 8 (u32 LE) then space = 1 (u64 LE).
        assert_eq!(
            encoders::encode_allocate(1),
            [8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn advance_nonce_account_encoder_matches_pre_refactor_golden_bytes() {
        // Pre-refactor inline bytes: [4, 0, 0, 0].
        assert_eq!(encoders::encode_advance_nonce_account(), [4, 0, 0, 0]);
    }

    #[test]
    fn withdraw_nonce_account_encoder_matches_pre_refactor_golden_bytes() {
        // disc 5 (u32 LE) then lamports = 1 (u64 LE).
        assert_eq!(
            encoders::encode_withdraw_nonce_account(1),
            [5, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn initialize_nonce_account_encoder_matches_pre_refactor_golden_bytes() {
        // Pre-refactor inline bytes: [6,0,0,0][authority: 32 bytes].
        let d = encoders::encode_initialize_nonce_account(&OWNER);
        assert_eq!(d.len(), 36);
        assert_eq!(&d[0..4], &[6, 0, 0, 0]);
        assert_eq!(&d[4..36], &OWNER);
    }

    #[test]
    fn authorize_nonce_account_encoder_matches_pre_refactor_golden_bytes() {
        // Pre-refactor inline bytes: [7,0,0,0][new_authority: 32 bytes].
        let d = encoders::encode_authorize_nonce_account(&OWNER);
        assert_eq!(d.len(), 36);
        assert_eq!(&d[0..4], &[7, 0, 0, 0]);
        assert_eq!(&d[4..36], &OWNER);
    }

    #[test]
    fn upgrade_nonce_account_encoder_matches_pre_refactor_golden_bytes() {
        // Pre-refactor inline bytes: [12, 0, 0, 0].
        assert_eq!(encoders::encode_upgrade_nonce_account(), [12, 0, 0, 0]);
    }
}
