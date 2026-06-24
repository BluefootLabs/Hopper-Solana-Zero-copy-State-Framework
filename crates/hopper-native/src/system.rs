//! System program CPI instructions.
//!
//! Full coverage of the System program instruction set: account
//! creation (`CreateAccount`, `CreateAccountWithSeed`), ownership
//! (`Assign`, `AssignWithSeed`), allocation (`Allocate`,
//! `AllocateWithSeed`), transfers (`Transfer`, `TransferWithSeed`), and
//! the durable-nonce family (`AdvanceNonceAccount`,
//! `WithdrawNonceAccount`, `InitializeNonceAccount`,
//! `AuthorizeNonceAccount`, `UpgradeNonceAccount`). All builders invoke
//! via `sol_invoke_signed_c` with zero heap allocation.

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{CpiAccount, Signer};
use crate::ProgramResult;

/// System program address: 11111111111111111111111111111111
pub const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

/// Recent blockhashes sysvar address (`SysvarRecentB1ockHashes...`),
/// required by the nonce instructions.
pub const RECENT_BLOCKHASHES_ID: Address =
    crate::address!("SysvarRecentB1ockHashes11111111111111111111");

/// Rent sysvar address, required by some nonce instructions.
pub const RENT_SYSVAR_ID: Address = crate::address!("SysvarRent111111111111111111111111111111111");

/// Maximum byte length of a System-program seed string (Solana
/// `MAX_SEED_LEN`). `*WithSeed` builders reject longer seeds.
pub const MAX_SEED_LEN: usize = 32;

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
    /// Invoke the CreateAccount instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the CreateAccount instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        // Instruction data: u32(0) + u64(lamports) + u64(space) + [u8;32](owner)
        let mut data = [0u8; 52];
        // index 0 = CreateAccount (already zero)
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());
        data[12..20].copy_from_slice(&self.space.to_le_bytes());
        data[20..52].copy_from_slice(self.owner.as_array());

        let accounts = [CpiAccount::from(self.from), CpiAccount::from(self.to)];

        invoke_system(&data, &accounts, signers)
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
    /// Invoke the Transfer instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the Transfer instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        // Instruction data: u32(2) + u64(lamports)
        let mut data = [0u8; 12];
        data[0] = 2;
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());

        let accounts = [CpiAccount::from(self.from), CpiAccount::from(self.to)];

        invoke_system(&data, &accounts, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for the system program's Assign instruction.
pub struct Assign<'a, 'b> {
    pub account: &'a AccountView<'a>,
    pub owner: &'b Address,
}

impl Assign<'_, '_> {
    /// Invoke the Assign instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the Assign instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        // Instruction data: u32(1) + [u8;32](owner)
        let mut data = [0u8; 36];
        data[0] = 1;
        data[4..36].copy_from_slice(self.owner.as_array());

        let accounts = [CpiAccount::from(self.account)];

        invoke_system(&data, &accounts, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for the system program's Allocate instruction.
pub struct Allocate<'a> {
    pub account: &'a AccountView<'a>,
    pub space: u64,
}

impl Allocate<'_> {
    /// Invoke the Allocate instruction (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the Allocate instruction with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        // Instruction data: u32(8) + u64(space)
        let mut data = [0u8; 12];
        data[0] = 8;
        data[4..12].copy_from_slice(&self.space.to_le_bytes());

        let accounts = [CpiAccount::from(self.account)];

        invoke_system(&data, &accounts, signers)
    }
}

// ---------------------------------------------------------------------
//  Seeded variants (CreateAccountWithSeed / AllocateWithSeed /
//  AssignWithSeed / TransferWithSeed). The seed is a bincode `String`:
//  a u64-LE length prefix followed by the UTF-8 bytes.
// ---------------------------------------------------------------------

/// Builder for `CreateAccountWithSeed`.
///
/// `to` must equal `create_with_seed(base.key, seed, owner)`. The `base`
/// account is a read-only signer used to derive the new address.
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
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        // u32(3) + base[32] + u64(seed_len) + seed + u64(lamports)
        //   + u64(space) + owner[32]
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
            CpiAccount::from(self.from),
            CpiAccount::from(self.to),
            CpiAccount::from(self.base),
        ];
        invoke_system(&data[..n], &accounts, signers)
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
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        // u32(9) + base[32] + u64(seed_len) + seed + u64(space) + owner[32]
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

        let accounts = [CpiAccount::from(self.account), CpiAccount::from(self.base)];
        invoke_system(&data[..n], &accounts, signers)
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
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        // u32(10) + base[32] + u64(seed_len) + seed + owner[32]
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

        let accounts = [CpiAccount::from(self.account), CpiAccount::from(self.base)];
        invoke_system(&data[..n], &accounts, signers)
    }
}

/// Builder for `TransferWithSeed`.
///
/// Moves lamports from a `from` account that is itself derived from
/// `base` + `from_seed` + `from_owner`.
pub struct TransferWithSeed<'a, 'b> {
    pub from: &'a AccountView<'a>,
    pub base: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub lamports: u64,
    pub from_seed: &'b [u8],
    pub from_owner: &'b Address,
}

impl TransferWithSeed<'_, '_> {
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.from_seed.len() > MAX_SEED_LEN {
            return Err(ProgramError::MaxSeedLengthExceeded);
        }
        // u32(11) + u64(lamports) + u64(seed_len) + seed + from_owner[32]
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
            CpiAccount::from(self.from),
            CpiAccount::from(self.base),
            CpiAccount::from(self.to),
        ];
        invoke_system(&data[..n], &accounts, signers)
    }
}

// ---------------------------------------------------------------------
//  Durable nonce family.
// ---------------------------------------------------------------------

/// Builder for `AdvanceNonceAccount` (instruction 4).
///
/// Accounts: `[nonce (writable), recent_blockhashes_sysvar, authority (signer)]`.
pub struct AdvanceNonceAccount<'a> {
    pub nonce: &'a AccountView<'a>,
    pub recent_blockhashes: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
}

impl AdvanceNonceAccount<'_> {
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let data = [4u8, 0, 0, 0];
        let accounts = [
            CpiAccount::from(self.nonce),
            CpiAccount::from(self.recent_blockhashes),
            CpiAccount::from(self.authority),
        ];
        invoke_system(&data, &accounts, signers)
    }
}

/// Builder for `WithdrawNonceAccount` (instruction 5).
///
/// Accounts: `[nonce (writable), to (writable), recent_blockhashes_sysvar,
/// rent_sysvar, authority (signer)]`.
pub struct WithdrawNonceAccount<'a> {
    pub nonce: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub recent_blockhashes: &'a AccountView<'a>,
    pub rent: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub lamports: u64,
}

impl WithdrawNonceAccount<'_> {
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 12];
        data[0] = 5;
        data[4..12].copy_from_slice(&self.lamports.to_le_bytes());
        let accounts = [
            CpiAccount::from(self.nonce),
            CpiAccount::from(self.to),
            CpiAccount::from(self.recent_blockhashes),
            CpiAccount::from(self.rent),
            CpiAccount::from(self.authority),
        ];
        invoke_system(&data, &accounts, signers)
    }
}

/// Builder for `InitializeNonceAccount` (instruction 6).
///
/// Accounts: `[nonce (writable), recent_blockhashes_sysvar, rent_sysvar]`.
pub struct InitializeNonceAccount<'a, 'b> {
    pub nonce: &'a AccountView<'a>,
    pub recent_blockhashes: &'a AccountView<'a>,
    pub rent: &'a AccountView<'a>,
    pub authority: &'b Address,
}

impl InitializeNonceAccount<'_, '_> {
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 36];
        data[0] = 6;
        data[4..36].copy_from_slice(self.authority.as_array());
        let accounts = [
            CpiAccount::from(self.nonce),
            CpiAccount::from(self.recent_blockhashes),
            CpiAccount::from(self.rent),
        ];
        invoke_system(&data, &accounts, signers)
    }
}

/// Builder for `AuthorizeNonceAccount` (instruction 7).
///
/// Accounts: `[nonce (writable), current_authority (signer)]`.
pub struct AuthorizeNonceAccount<'a, 'b> {
    pub nonce: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub new_authority: &'b Address,
}

impl AuthorizeNonceAccount<'_, '_> {
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let mut data = [0u8; 36];
        data[0] = 7;
        data[4..36].copy_from_slice(self.new_authority.as_array());
        let accounts = [
            CpiAccount::from(self.nonce),
            CpiAccount::from(self.authority),
        ];
        invoke_system(&data, &accounts, signers)
    }
}

/// Builder for `UpgradeNonceAccount` (instruction 12).
///
/// Accounts: `[nonce (writable)]`.
pub struct UpgradeNonceAccount<'a> {
    pub nonce: &'a AccountView<'a>,
}

impl UpgradeNonceAccount<'_> {
    /// Invoke (no PDA signers).
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        let data = [12u8, 0, 0, 0];
        let accounts = [CpiAccount::from(self.nonce)];
        invoke_system(&data, &accounts, &[])
    }
}

// ---------------------------------------------------------------------

/// Build an InstructionView<'_, '_, '_, '_> to the system program and invoke.
#[inline]
fn invoke_system(
    data: &[u8],
    accounts: &[CpiAccount<'_>],
    signers: &[Signer<'_, '_>],
) -> ProgramResult {
    // Build an InstructionView<'_, '_, '_, '_> to the system program and invoke via C ABI.
    #[cfg(target_os = "solana")]
    {
        let ix = crate::instruction::InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data,
            accounts: &[], // Not used by the C ABI path
        };
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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

/// Compatibility re-exports matching `pinocchio_system::instructions::*`,
/// extended with Hopper's full WithSeed and durable-nonce coverage.
pub mod instructions {
    pub use super::{
        AdvanceNonceAccount, Allocate, AllocateWithSeed, Assign, AssignWithSeed,
        AuthorizeNonceAccount, CreateAccount, CreateAccountWithSeed, InitializeNonceAccount,
        Transfer, TransferWithSeed, UpgradeNonceAccount, WithdrawNonceAccount,
    };
}

// ---------------------------------------------------------------------
//  Typed durable-nonce account reader.
//
//  No other Solana framework ships a typed view over the durable-nonce
//  account. The account is a versioned enum wrapping a state enum; the
//  byte layout for the current (V1, Initialized) form is:
//
//    bytes 0..4    version tag  (u32 LE; 1 = Current)
//    bytes 4..8    state tag    (u32 LE; 1 = Initialized)
//    bytes 8..40   authority    (Pubkey)
//    bytes 40..72  durable nonce (the stored blockhash)
//    bytes 72..80  fee_calculator.lamports_per_signature (u64 LE)
// ---------------------------------------------------------------------

/// Minimum byte length of an initialized durable-nonce account.
pub const NONCE_ACCOUNT_LEN: usize = 80;

/// Nonce account version tag for the current format.
pub const NONCE_VERSION_CURRENT: u32 = 1;

/// Nonce state tag for the initialized form.
pub const NONCE_STATE_INITIALIZED: u32 = 1;

/// Typed, zero-copy view over an initialized durable-nonce account.
#[derive(Clone, Copy, Debug)]
pub struct NonceState<'a> {
    data: &'a [u8],
}

impl<'a> NonceState<'a> {
    /// Parse an initialized nonce account from raw account data.
    ///
    /// Returns `Err(InvalidAccountData)` if the buffer is too short, the
    /// version is not `Current`, or the state is not `Initialized`.
    #[inline]
    pub fn from_account_data(data: &'a [u8]) -> Result<Self, ProgramError> {
        if data.len() < NONCE_ACCOUNT_LEN {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let version = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let state = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != NONCE_VERSION_CURRENT || state != NONCE_STATE_INITIALIZED {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self { data })
    }

    /// The nonce authority allowed to advance/withdraw.
    #[inline]
    pub fn authority(&self) -> &'a Address {
        // SAFETY: `from_account_data` verified `data.len() >= 80`; bytes
        // 8..40 are a 32-byte address with alignment 1.
        unsafe { &*(self.data.as_ptr().add(8) as *const Address) }
    }

    /// The stored durable nonce (a recent blockhash, used as the tx nonce).
    #[inline]
    pub fn durable_nonce(&self) -> &'a [u8; 32] {
        // SAFETY: bytes 40..72 are present per the length check above.
        unsafe { &*(self.data.as_ptr().add(40) as *const [u8; 32]) }
    }

    /// The fee rate (`lamports_per_signature`) captured with the nonce.
    #[inline]
    pub fn lamports_per_signature(&self) -> u64 {
        let b = &self.data[72..80];
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}
