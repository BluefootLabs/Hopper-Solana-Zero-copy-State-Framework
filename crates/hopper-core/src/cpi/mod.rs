//! Const-generic CPI builder -- stack-only, zero-allocation CPI calls.
//!
//! Both account count and data size are const generics, ensuring everything
//! lives on the SBF stack (4096 bytes). No heap allocation ever.
//!
//! ## Design (inspired by Quasar's CpiCall<N, D>, improved)
//!
//! - `HopperCpi<A, D>` -- fully const-generic: accounts + data
//! - `HopperCpiBuf<A, MAX>` -- const accounts, runtime data length
//! - Uses `MaybeUninit` for zero-cost initialization
//! - Direct `sol_invoke_signed_c` syscall on SBF
//!
//! ```ignore
//! let cpi = HopperCpi::<3, 9>::new(token_program_id)
//!     .account(source, true, false)   // writable, not signer
//!     .account(dest, true, false)
//!     .account(authority, false, true) // not writable, signer
//!     .data(&[3, /* transfer discriminator + amount */]);
//! cpi.invoke()?;
//! ```

use core::mem::MaybeUninit;
use hopper_runtime::error::ProgramError;
use hopper_runtime::ProgramResult;

/// Stack-allocated CPI call with compile-time-known account count and data size.
///
/// Both `ACCTS` and `DATA` are const generics -- the compiler knows the
/// exact buffer sizes at compile time, enabling optimal stack allocation
/// and no runtime branching on sizes.
pub struct HopperCpi<'a, const ACCTS: usize, const DATA: usize> {
    /// The program to invoke.
    #[allow(dead_code)]
    program_id: &'a hopper_runtime::Address,
    /// Account metadata: (pubkey, is_writable, is_signer).
    account_keys: [&'a hopper_runtime::Address; ACCTS],
    account_flags: [(bool, bool); ACCTS], // (is_writable, is_signer)
    /// Source AccountViews for the CPI (needed by the runtime).
    /// Uses MaybeUninit to avoid UB from null/zeroed references.
    /// Slots 0..acct_cursor are initialized; the rest are uninit.
    account_views: [MaybeUninit<&'a hopper_runtime::AccountView>; ACCTS],
    /// Instruction data (fixed size, fully on stack).
    data: [u8; DATA],
    /// Number of accounts added so far.
    acct_cursor: usize,
}

impl<'a, const ACCTS: usize, const DATA: usize> HopperCpi<'a, ACCTS, DATA> {
    /// Begin building a CPI call to `program_id`.
    #[inline(always)]
    pub fn new(program_id: &'a hopper_runtime::Address) -> Self {
        Self {
            program_id,
            account_keys: [program_id; ACCTS], // init value; overwritten by add_account()
            account_flags: [(false, false); ACCTS],
            // SAFETY: MaybeUninit<T> does not require initialization.
            // Creating an array of MaybeUninit is always safe.
            account_views: unsafe { MaybeUninit::uninit().assume_init() },
            data: [0u8; DATA],
            acct_cursor: 0,
        }
    }

    /// Add an account to the CPI call.
    ///
    /// Must be called exactly `ACCTS` times before `invoke`.
    #[inline(always)]
    pub fn add_account(
        mut self,
        view: &'a hopper_runtime::AccountView,
        is_writable: bool,
        is_signer: bool,
    ) -> Self {
        let idx = self.acct_cursor;
        debug_assert!(idx < ACCTS, "Too many accounts added to CPI");
        self.account_keys[idx] = view.address();
        self.account_flags[idx] = (is_writable, is_signer);
        self.account_views[idx] = MaybeUninit::new(view);
        self.acct_cursor += 1;
        self
    }

    /// Set the instruction data. Must be exactly `DATA` bytes.
    #[inline(always)]
    pub fn set_data(mut self, src: &[u8; DATA]) -> Self {
        self.data = *src;
        self
    }

    /// Write instruction data from a slice (must be exactly DATA bytes).
    #[inline(always)]
    pub fn set_data_from_slice(mut self, src: &[u8]) -> Result<Self, ProgramError> {
        if src.len() != DATA {
            return Err(ProgramError::InvalidInstructionData);
        }
        self.data.copy_from_slice(src);
        Ok(self)
    }

    /// Invoke the CPI without signer seeds.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        debug_assert_eq!(self.acct_cursor, ACCTS, "Not all accounts added to CPI");
        self.invoke_signed(&[])
    }

    /// Invoke the CPI with PDA signer seeds.
    #[inline]
    pub fn invoke_signed(&self, seeds: &[&[&[u8]]]) -> ProgramResult {
        #[cfg(target_os = "solana")]
        {
            use hopper_runtime::instruction::{InstructionAccount, InstructionView, Seed, Signer};

            debug_assert_eq!(self.acct_cursor, ACCTS, "Not all accounts added to CPI");

            // SAFETY: All ACCTS slots have been initialized via add_account
            // (enforced by the debug_assert above). We transmute the
            // MaybeUninit array to the initialized reference array.
            let views: &[&hopper_runtime::AccountView; ACCTS] = unsafe {
                &*(&self.account_views as *const [MaybeUninit<&hopper_runtime::AccountView>; ACCTS]
                    as *const [&hopper_runtime::AccountView; ACCTS])
            };

            // Build InstructionAccount array on the stack
            let mut ix_accounts: [InstructionAccount; ACCTS] = unsafe { core::mem::zeroed() };
            let mut i = 0;
            while i < ACCTS {
                ix_accounts[i] = InstructionAccount {
                    address: self.account_keys[i],
                    is_writable: self.account_flags[i].0,
                    is_signer: self.account_flags[i].1,
                };
                i += 1;
            }

            let ix = InstructionView {
                program_id: self.program_id,
                accounts: &ix_accounts,
                data: &self.data,
            };

            if seeds.is_empty() {
                hopper_runtime::cpi::invoke(&ix, views)
            } else {
                let mut signers_buf: [Signer; 4] = unsafe { core::mem::zeroed() };
                let signer_count = seeds.len().min(4);
                let mut seed_bufs: [[Seed; 16]; 4] = unsafe { core::mem::zeroed() };
                let mut seed_lens = [0usize; 4];

                let mut s = 0;
                while s < signer_count {
                    let signer_seeds = seeds[s];
                    let num_seeds = signer_seeds.len().min(16);
                    let mut sd = 0;
                    while sd < num_seeds {
                        seed_bufs[s][sd] = Seed::from(signer_seeds[sd]);
                        sd += 1;
                    }
                    seed_lens[s] = num_seeds;
                    s += 1;
                }

                let mut s = 0;
                while s < signer_count {
                    signers_buf[s] = Signer::from(&seed_bufs[s][..seed_lens[s]]);
                    s += 1;
                }

                hopper_runtime::cpi::invoke_signed(&ix, views, &signers_buf[..signer_count])
            }
        }
        #[cfg(not(target_os = "solana"))]
        {
            let _ = seeds;
            Ok(())
        }
    }
}

/// Variable-data CPI builder -- const accounts, runtime data length.
///
/// For instructions where data size isn't known at compile time
/// (e.g., Borsh-serialized arguments), but bounded by `MAX`.
pub struct HopperCpiBuf<'a, const ACCTS: usize, const MAX: usize> {
    #[allow(dead_code)]
    program_id: &'a hopper_runtime::Address,
    account_keys: [&'a hopper_runtime::Address; ACCTS],
    account_flags: [(bool, bool); ACCTS],
    account_views: [MaybeUninit<&'a hopper_runtime::AccountView>; ACCTS],
    data: [u8; MAX],
    data_len: usize,
    acct_cursor: usize,
}

impl<'a, const ACCTS: usize, const MAX: usize> HopperCpiBuf<'a, ACCTS, MAX> {
    /// Begin building a variable-data CPI call.
    #[inline(always)]
    pub fn new(program_id: &'a hopper_runtime::Address) -> Self {
        Self {
            program_id,
            account_keys: [program_id; ACCTS],
            account_flags: [(false, false); ACCTS],
            // SAFETY: MaybeUninit<T> does not require initialization.
            account_views: unsafe { MaybeUninit::uninit().assume_init() },
            data: [0u8; MAX],
            data_len: 0,
            acct_cursor: 0,
        }
    }

    /// Add an account.
    #[inline(always)]
    pub fn add_account(
        mut self,
        view: &'a hopper_runtime::AccountView,
        is_writable: bool,
        is_signer: bool,
    ) -> Self {
        let idx = self.acct_cursor;
        debug_assert!(idx < ACCTS);
        self.account_keys[idx] = view.address();
        self.account_flags[idx] = (is_writable, is_signer);
        self.account_views[idx] = MaybeUninit::new(view);
        self.acct_cursor += 1;
        self
    }

    /// Write data into the buffer. Returns error if exceeds MAX.
    #[inline]
    pub fn write_data(mut self, src: &[u8]) -> Result<Self, ProgramError> {
        if src.len() > MAX {
            return Err(ProgramError::InvalidInstructionData);
        }
        self.data[..src.len()].copy_from_slice(src);
        self.data_len = src.len();
        Ok(self)
    }

    /// Invoke without signer seeds.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke with PDA signer seeds.
    #[inline]
    pub fn invoke_signed(&self, seeds: &[&[&[u8]]]) -> ProgramResult {
        #[cfg(target_os = "solana")]
        {
            use hopper_runtime::instruction::{InstructionAccount, InstructionView, Seed, Signer};

            debug_assert_eq!(self.acct_cursor, ACCTS, "Not all accounts added to CPI");

            // SAFETY: All ACCTS slots initialized via add_account.
            let views: &[&hopper_runtime::AccountView; ACCTS] = unsafe {
                &*(&self.account_views as *const [MaybeUninit<&hopper_runtime::AccountView>; ACCTS]
                    as *const [&hopper_runtime::AccountView; ACCTS])
            };

            let mut ix_accounts: [InstructionAccount; ACCTS] = unsafe { core::mem::zeroed() };
            let mut i = 0;
            while i < ACCTS {
                ix_accounts[i] = InstructionAccount {
                    address: self.account_keys[i],
                    is_writable: self.account_flags[i].0,
                    is_signer: self.account_flags[i].1,
                };
                i += 1;
            }

            let ix = InstructionView {
                program_id: self.program_id,
                accounts: &ix_accounts,
                data: &self.data[..self.data_len],
            };

            if seeds.is_empty() {
                hopper_runtime::cpi::invoke(&ix, views)
            } else {
                let mut signers_buf: [Signer; 4] = unsafe { core::mem::zeroed() };
                let signer_count = seeds.len().min(4);
                let mut seed_bufs: [[Seed; 16]; 4] = unsafe { core::mem::zeroed() };
                let mut seed_lens = [0usize; 4];

                let mut s = 0;
                while s < signer_count {
                    let signer_seeds = seeds[s];
                    let num_seeds = signer_seeds.len().min(16);
                    let mut sd = 0;
                    while sd < num_seeds {
                        seed_bufs[s][sd] = Seed::from(signer_seeds[sd]);
                        sd += 1;
                    }
                    seed_lens[s] = num_seeds;
                    s += 1;
                }

                let mut s = 0;
                while s < signer_count {
                    signers_buf[s] = Signer::from(&seed_bufs[s][..seed_lens[s]]);
                    s += 1;
                }

                hopper_runtime::cpi::invoke_signed(&ix, views, &signers_buf[..signer_count])
            }
        }
        #[cfg(not(target_os = "solana"))]
        {
            let _ = seeds;
            Ok(())
        }
    }
}

