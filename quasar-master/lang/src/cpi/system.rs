use {
    super::{CpiCall, InstructionAccount, Signer},
    crate::traits::{AsAccountView, Id},
    solana_account_view::AccountView,
    solana_address::{declare_id, Address},
    solana_program_error::ProgramResult,
};

declare_id!("11111111111111111111111111111111");
pub use ID as SYSTEM_PROGRAM_ID;

/// Create a new account via the System program.
///
/// ### Accounts:
///   0. `[WRITE, SIGNER]` Funding account
///   1. `[WRITE, SIGNER]` New account
///
/// ### Instruction data (52 bytes):
/// ```text
/// [0..4  ] discriminator (0)
/// [4..12 ] lamports      (u64 LE)
/// [12..20] space          (u64 LE)
/// [20..52] owner          (32-byte address)
/// ```
#[inline(always)]
pub fn create_account<'a>(
    from: &'a AccountView,
    to: &'a AccountView,
    lamports: impl Into<u64>,
    space: u64,
    owner: &'a Address,
) -> CpiCall<'a, 2, 52> {
    // SAFETY: All 52 bytes written before `assume_init`.
    let data = unsafe {
        let mut buf = core::mem::MaybeUninit::<[u8; 52]>::uninit();
        let ptr = buf.as_mut_ptr() as *mut u8;
        // discriminator 0 — four zero bytes
        core::ptr::write_bytes(ptr, 0, 4);
        core::ptr::copy_nonoverlapping(lamports.into().to_le_bytes().as_ptr(), ptr.add(4), 8);
        core::ptr::copy_nonoverlapping(space.to_le_bytes().as_ptr(), ptr.add(12), 8);
        core::ptr::copy_nonoverlapping(owner.as_ref().as_ptr(), ptr.add(20), 32);
        buf.assume_init()
    };

    CpiCall::new(
        &SYSTEM_PROGRAM_ID,
        [
            InstructionAccount::writable_signer(from.address()),
            InstructionAccount::writable_signer(to.address()),
        ],
        [from, to],
        data,
    )
}

/// Transfer lamports between accounts via the System program.
///
/// ### Accounts:
///   0. `[WRITE, SIGNER]` Source account
///   1. `[WRITE]` Destination account
///
/// ### Instruction data (12 bytes):
/// ```text
/// [0..4 ] discriminator (2)
/// [4..12] lamports      (u64 LE)
/// ```
#[inline(always)]
pub fn transfer<'a>(
    from: &'a AccountView,
    to: &'a AccountView,
    lamports: impl Into<u64>,
) -> CpiCall<'a, 2, 12> {
    // SAFETY: All 12 bytes written before `assume_init`.
    let data = unsafe {
        let mut buf = core::mem::MaybeUninit::<[u8; 12]>::uninit();
        let ptr = buf.as_mut_ptr() as *mut u8;
        core::ptr::copy_nonoverlapping(2u32.to_le_bytes().as_ptr(), ptr, 4);
        core::ptr::copy_nonoverlapping(lamports.into().to_le_bytes().as_ptr(), ptr.add(4), 8);
        buf.assume_init()
    };

    CpiCall::new(
        &SYSTEM_PROGRAM_ID,
        [
            InstructionAccount::writable_signer(from.address()),
            InstructionAccount::writable(to.address()),
        ],
        [from, to],
        data,
    )
}

/// Assign an account to a new owner program via the System program.
///
/// ### Accounts:
///   0. `[WRITE, SIGNER]` Account to assign
///
/// ### Instruction data (36 bytes):
/// ```text
/// [0..4 ] discriminator (1)
/// [4..36] owner          (32-byte address)
/// ```
#[inline(always)]
pub fn assign<'a>(account: &'a AccountView, owner: &'a Address) -> CpiCall<'a, 1, 36> {
    // SAFETY: All 36 bytes written before `assume_init`.
    let data = unsafe {
        let mut buf = core::mem::MaybeUninit::<[u8; 36]>::uninit();
        let ptr = buf.as_mut_ptr() as *mut u8;
        core::ptr::copy_nonoverlapping(1u32.to_le_bytes().as_ptr(), ptr, 4);
        core::ptr::copy_nonoverlapping(owner.as_ref().as_ptr(), ptr.add(4), 32);
        buf.assume_init()
    };

    CpiCall::new(
        &SYSTEM_PROGRAM_ID,
        [InstructionAccount::writable_signer(account.address())],
        [account],
        data,
    )
}

/// Allocate space in an account without transferring ownership.
///
/// System program instruction index: 8
///
/// ### Accounts:
///   0. `[WRITE, SIGNER]` Account to allocate
///
/// ### Instruction data (12 bytes):
/// ```text
/// [0..4 ] discriminator (8)
/// [4..12] space          (u64, little-endian)
/// ```
#[inline(always)]
pub fn allocate<'a>(account: &'a AccountView, space: u64) -> CpiCall<'a, 1, 12> {
    // SAFETY: All 12 bytes written before `assume_init`.
    let data = unsafe {
        let mut buf = core::mem::MaybeUninit::<[u8; 12]>::uninit();
        let ptr = buf.as_mut_ptr() as *mut u8;
        core::ptr::copy_nonoverlapping(8u32.to_le_bytes().as_ptr(), ptr, 4);
        core::ptr::copy_nonoverlapping(space.to_le_bytes().as_ptr(), ptr.add(4), 8);
        buf.assume_init()
    };

    CpiCall::new(
        &SYSTEM_PROGRAM_ID,
        [InstructionAccount::writable_signer(account.address())],
        [account],
        data,
    )
}

/// Initialize an account, handling both fresh and pre-funded cases.
///
/// If the account has zero lamports (fresh), issues a single `CreateAccount`
/// system CPI. If the account is pre-funded (e.g. someone sent SOL to the
/// PDA address before initialization), uses `Transfer` (top up rent delta
/// if needed) + `Allocate` (set space) + `Assign` (change owner).
///
/// This avoids the `CreateAccount` failure mode where the target account
/// already has a non-zero lamport balance.
///
/// ### Parameters
///
/// - `payer`: funding account (must be a signer)
/// - `account`: account to initialize (writable; PDA signer seeds in `signers`)
/// - `lamports`: minimum rent-exempt balance
/// - `space`: account data size in bytes
/// - `owner`: program to own the new account
/// - `signers`: PDA signer seeds (empty slice for keypair-signed accounts)
#[inline(always)]
pub fn init_account(
    payer: &AccountView,
    account: &mut AccountView,
    lamports: u64,
    space: u64,
    owner: &Address,
    signers: &[Signer],
) -> ProgramResult {
    if account.lamports() == 0 {
        create_account(payer, account, lamports, space, owner).invoke_with_signers(signers)
    } else {
        // Pre-funded path: the account already has lamports (someone sent SOL
        // to the PDA address). We can't use CreateAccount (it fails if the
        // account has a non-zero balance), so we use three system CPIs:
        //   1. Transfer  — top up to rent-exempt minimum (skip if already enough)
        //   2. Allocate  — set data space (must be a CPI, not raw resize — the SVM
        //      tracks original data_len from serialization and rejects direct mutations
        //      on accounts that were not program-owned at serialization time)
        //   3. Assign    — transfer ownership to the target program
        //
        // TODO: Replace with single `CreateAccountAllowPrefund` CPI (system
        // program instruction 13) once the `create_account_allow_prefund`
        // feature gate is activated on mainnet. That instruction is
        // CreateAccount without the "lamports must be 0" check — collapses
        // these 3 CPIs into 1. Ref: anza-xyz/pinocchio programs/system.
        let required = lamports.saturating_sub(account.lamports());
        if required > 0 {
            transfer(payer, account, required).invoke()?;
        }
        allocate(account, space).invoke_with_signers(signers)?;
        assign(account, owner).invoke_with_signers(signers)?;
        Ok(())
    }
}

// --- System program account type ---

/// Marker type for the system program.
///
/// Use with the `Program<T>` wrapper:
/// ```ignore
/// pub system_program: &'info Program<System>,
/// ```
pub struct System;

impl Id for System {
    const ID: Address = Address::new_from_array([0u8; 32]);
}

impl crate::accounts::Program<System> {
    /// Create a new account. See [`create_account`] for account and data
    /// layout.
    #[inline(always)]
    pub fn create_account<'a>(
        &'a self,
        from: &'a impl AsAccountView,
        to: &'a impl AsAccountView,
        lamports: impl Into<u64>,
        space: u64,
        owner: &'a Address,
    ) -> CpiCall<'a, 2, 52> {
        create_account(
            from.to_account_view(),
            to.to_account_view(),
            lamports,
            space,
            owner,
        )
    }

    /// Transfer lamports. See [`transfer`] for account and data layout.
    #[inline(always)]
    pub fn transfer<'a>(
        &'a self,
        from: &'a impl AsAccountView,
        to: &'a impl AsAccountView,
        lamports: impl Into<u64>,
    ) -> CpiCall<'a, 2, 12> {
        transfer(from.to_account_view(), to.to_account_view(), lamports)
    }

    /// Assign an account to a new owner. See [`assign`] for details.
    #[inline(always)]
    pub fn assign<'a>(
        &'a self,
        account: &'a impl AsAccountView,
        owner: &'a Address,
    ) -> CpiCall<'a, 1, 36> {
        assign(account.to_account_view(), owner)
    }

    /// Initialize an account, handling both fresh and pre-funded cases.
    /// See [`init_account`] for details.
    #[inline(always)]
    pub fn init_account(
        &self,
        payer: &impl AsAccountView,
        account: &mut AccountView,
        lamports: u64,
        space: u64,
        owner: &Address,
        signers: &[Signer],
    ) -> ProgramResult {
        init_account(
            payer.to_account_view(),
            account,
            lamports,
            space,
            owner,
            signers,
        )
    }
}
