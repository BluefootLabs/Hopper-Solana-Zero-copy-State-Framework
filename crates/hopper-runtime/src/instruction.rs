//! Hopper-owned CPI instruction types.
//!
//! These types form the instruction ABI for Hopper cross-program invocation.
//! They are Hopper-owned and reference Hopper's `Address` and `AccountView`
//! types, giving the framework full control over its public API surface.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use core::marker::PhantomData;
use core::mem::MaybeUninit;

// ── InstructionAccount ───────────────────────────────────────────────

/// Metadata for an account referenced in a CPI instruction.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InstructionAccount<'a> {
    /// Public key of the account.
    pub address: &'a Address,
    /// Whether the account should be writable.
    pub is_writable: bool,
    /// Whether the account should sign.
    pub is_signer: bool,
}

impl<'a> InstructionAccount<'a> {
    /// Construct with explicit flags.
    #[inline(always)]
    pub const fn new(address: &'a Address, is_writable: bool, is_signer: bool) -> Self {
        Self {
            address,
            is_writable,
            is_signer,
        }
    }

    /// Read-only, non-signer.
    #[inline(always)]
    pub const fn readonly(address: &'a Address) -> Self {
        Self {
            address,
            is_writable: false,
            is_signer: false,
        }
    }

    /// Writable, non-signer.
    #[inline(always)]
    pub const fn writable(address: &'a Address) -> Self {
        Self {
            address,
            is_writable: true,
            is_signer: false,
        }
    }

    /// Read-only signer.
    #[inline(always)]
    pub const fn readonly_signer(address: &'a Address) -> Self {
        Self {
            address,
            is_writable: false,
            is_signer: true,
        }
    }

    /// Writable signer.
    #[inline(always)]
    pub const fn writable_signer(address: &'a Address) -> Self {
        Self {
            address,
            is_writable: true,
            is_signer: true,
        }
    }
}

impl<'a> From<&'a AccountView> for InstructionAccount<'a> {
    #[inline(always)]
    fn from(view: &'a AccountView) -> Self {
        Self {
            address: view.address(),
            is_writable: view.is_writable(),
            is_signer: view.is_signer(),
        }
    }
}

/// Stored account metadata for governance/proposal-style arbitrary CPI.
///
/// The wire form is compact and owner-agnostic: a public key plus explicit
/// signer/writable flags. It can be stored inside dynamic tails and converted
/// to [`InstructionAccount`] without allocation when executing the proposal.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredAccountMeta {
    /// Account public key.
    pub pubkey: Address,
    /// Bit 0 = signer, bit 1 = writable.
    pub flags: u8,
}

impl StoredAccountMeta {
    /// Signer flag bit.
    pub const SIGNER: u8 = 0b0000_0001;
    /// Writable flag bit.
    pub const WRITABLE: u8 = 0b0000_0010;

    /// Construct account metadata from explicit booleans.
    #[inline(always)]
    pub const fn new(pubkey: Address, is_signer: bool, is_writable: bool) -> Self {
        let mut flags = 0u8;
        if is_signer {
            flags |= Self::SIGNER;
        }
        if is_writable {
            flags |= Self::WRITABLE;
        }
        Self { pubkey, flags }
    }

    /// Read-only, non-signer account metadata.
    #[inline(always)]
    pub const fn readonly(pubkey: Address) -> Self {
        Self::new(pubkey, false, false)
    }

    /// Writable, non-signer account metadata.
    #[inline(always)]
    pub const fn writable(pubkey: Address) -> Self {
        Self::new(pubkey, false, true)
    }

    /// Read-only signer account metadata.
    #[inline(always)]
    pub const fn readonly_signer(pubkey: Address) -> Self {
        Self::new(pubkey, true, false)
    }

    /// Writable signer account metadata.
    #[inline(always)]
    pub const fn writable_signer(pubkey: Address) -> Self {
        Self::new(pubkey, true, true)
    }

    /// Whether this account must sign.
    #[inline(always)]
    pub const fn is_signer(&self) -> bool {
        self.flags & Self::SIGNER != 0
    }

    /// Whether this account must be writable.
    #[inline(always)]
    pub const fn is_writable(&self) -> bool {
        self.flags & Self::WRITABLE != 0
    }

    /// Convert this stored meta to CPI metadata.
    #[inline(always)]
    pub fn to_instruction_account(&self) -> InstructionAccount<'_> {
        InstructionAccount::new(&self.pubkey, self.is_writable(), self.is_signer())
    }
}

/// Borrowed stored instruction payload for proposal/governance execution.
#[derive(Debug, Clone, Copy)]
pub struct StoredInstruction<'a> {
    /// Program to invoke.
    pub program_id: Address,
    /// Stored account metas in CPI order.
    pub account_metas: &'a [StoredAccountMeta],
    /// Stored instruction data.
    pub instruction_data: &'a [u8],
}

impl<'a> StoredInstruction<'a> {
    /// Construct a stored instruction with CPI account-count bounds.
    #[inline]
    pub fn new(
        program_id: Address,
        account_metas: &'a [StoredAccountMeta],
        instruction_data: &'a [u8],
    ) -> Result<Self, ProgramError> {
        if account_metas.len() > crate::cpi::MAX_CPI_ACCOUNTS {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(Self {
            program_id,
            account_metas,
            instruction_data,
        })
    }

    /// Number of account metas in this stored instruction.
    #[inline(always)]
    pub const fn account_count(&self) -> usize {
        self.account_metas.len()
    }

    /// Convert stored metas into a caller-provided CPI account buffer.
    #[inline]
    pub fn write_instruction_accounts<const N: usize>(
        &'a self,
        out: &'a mut [MaybeUninit<InstructionAccount<'a>>; N],
    ) -> Result<&'a [InstructionAccount<'a>], ProgramError> {
        if self.account_metas.len() > N {
            return Err(ProgramError::InvalidArgument);
        }
        let mut index = 0;
        while index < self.account_metas.len() {
            out[index].write(self.account_metas[index].to_instruction_account());
            index += 1;
        }
        Ok(unsafe {
            core::slice::from_raw_parts(
                out.as_ptr() as *const InstructionAccount<'a>,
                self.account_metas.len(),
            )
        })
    }

    /// Build an [`InstructionView`] using a caller-provided account buffer.
    #[inline]
    pub fn to_instruction_view<const N: usize>(
        &'a self,
        out: &'a mut [MaybeUninit<InstructionAccount<'a>>; N],
    ) -> Result<InstructionView<'a, 'a, 'a, 'a>, ProgramError> {
        let accounts = self.write_instruction_accounts(out)?;
        Ok(InstructionView {
            program_id: &self.program_id,
            data: self.instruction_data,
            accounts,
        })
    }
}

// ── InstructionView ──────────────────────────────────────────────────

/// A cross-program instruction to invoke.
#[derive(Debug, Clone)]
pub struct InstructionView<'a, 'b, 'c, 'd>
where
    'a: 'b,
{
    /// Program to call.
    pub program_id: &'c Address,
    /// Instruction data.
    pub data: &'d [u8],
    /// Account metadata.
    pub accounts: &'b [InstructionAccount<'a>],
}

// ── CpiAccount (hopper-native backend) ───────────────────────────────

/// C-ABI account info passed to `sol_invoke_signed_c`.
///
/// This matches the Solana runtime's expected layout for CPI account infos.
/// Only available with hopper-native-backend because it requires direct
/// pointer access into the runtime's account memory.
#[cfg(feature = "hopper-native-backend")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CpiAccount<'a> {
    address: *const Address,
    lamports: *const u64,
    data_len: u64,
    data: *const u8,
    owner: *const Address,
    rent_epoch: u64,
    is_signer: bool,
    is_writable: bool,
    executable: bool,
    _account_view: PhantomData<&'a AccountView>,
}

#[cfg(feature = "hopper-native-backend")]
impl<'a> From<&'a AccountView> for CpiAccount<'a> {
    #[inline]
    fn from(view: &'a AccountView) -> Self {
        let raw = view.account_ptr();
        // SAFETY: account_ptr() returns a valid pointer to the runtime
        // account struct. The address and owner fields have the same binary
        // layout as hopper_runtime::Address (#[repr(transparent)] over [u8; 32]).
        Self {
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            address: unsafe { core::ptr::addr_of!((*raw).address) as *const Address },
            lamports: unsafe { core::ptr::addr_of!((*raw).lamports) },
            data_len: view.data_len() as u64,
            data: view.data_ptr_unchecked(),
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            owner: unsafe { core::ptr::addr_of!((*raw).owner) as *const Address },
            rent_epoch: 0,
            is_signer: view.is_signer(),
            is_writable: view.is_writable(),
            executable: view.executable(),
            _account_view: PhantomData,
        }
    }
}

// ── Seed ─────────────────────────────────────────────────────────────

/// A single PDA seed for CPI signing.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct Seed<'a> {
    pub(crate) seed: *const u8,
    pub(crate) len: u64,
    _bytes: PhantomData<&'a [u8]>,
}

impl<'a> From<&'a [u8]> for Seed<'a> {
    #[inline(always)]
    fn from(bytes: &'a [u8]) -> Self {
        Self {
            seed: bytes.as_ptr(),
            len: bytes.len() as u64,
            _bytes: PhantomData,
        }
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for Seed<'a> {
    #[inline(always)]
    fn from(bytes: &'a [u8; N]) -> Self {
        Self {
            seed: bytes.as_ptr(),
            len: N as u64,
            _bytes: PhantomData,
        }
    }
}

impl core::ops::Deref for Seed<'_> {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { core::slice::from_raw_parts(self.seed, self.len as usize) }
    }
}

// ── Signer ───────────────────────────────────────────────────────────

/// A PDA signer: a set of seeds that derive the signing PDA.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct Signer<'a, 'b> {
    pub(crate) seeds: *const Seed<'a>,
    pub(crate) len: u64,
    _seeds: PhantomData<&'b [Seed<'a>]>,
}

impl<'a, 'b> From<&'b [Seed<'a>]> for Signer<'a, 'b> {
    #[inline(always)]
    fn from(seeds: &'b [Seed<'a>]) -> Self {
        Self {
            seeds: seeds.as_ptr(),
            len: seeds.len() as u64,
            _seeds: PhantomData,
        }
    }
}

impl<'a, 'b, const N: usize> From<&'b [Seed<'a>; N]> for Signer<'a, 'b> {
    #[inline(always)]
    fn from(seeds: &'b [Seed<'a>; N]) -> Self {
        Self {
            seeds: seeds.as_ptr(),
            len: N as u64,
            _seeds: PhantomData,
        }
    }
}

/// Convenience macro for building an array of `Seed` from expressions.
///
/// Usage: `let seeds = seeds!(b"vault", mint_key.as_ref(), &[bump]);`
#[macro_export]
macro_rules! seeds {
    ( $($seed:expr),* $(,)? ) => {
        [$(
            $crate::instruction::Seed::from($seed),
        )*]
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_account_meta_flags_round_trip() {
        let key = Address::new_from_array([3; 32]);
        let meta = StoredAccountMeta::writable_signer(key);
        assert!(meta.is_signer());
        assert!(meta.is_writable());

        let ix_meta = meta.to_instruction_account();
        assert_eq!(ix_meta.address, &key);
        assert!(ix_meta.is_signer);
        assert!(ix_meta.is_writable);
    }

    #[test]
    fn stored_instruction_builds_instruction_view_without_alloc() {
        let program = Address::new_from_array([9; 32]);
        let first = Address::new_from_array([1; 32]);
        let second = Address::new_from_array([2; 32]);
        let metas = [
            StoredAccountMeta::readonly(first),
            StoredAccountMeta::writable(second),
        ];
        let data = [7u8, 8, 9];
        let stored = StoredInstruction::new(program, &metas, &data).unwrap();
        let mut out: [MaybeUninit<InstructionAccount<'_>>; 2] =
            [MaybeUninit::uninit(), MaybeUninit::uninit()];

        let view = stored.to_instruction_view(&mut out).unwrap();
        assert_eq!(view.program_id, &program);
        assert_eq!(view.data, &data);
        assert_eq!(view.accounts.len(), 2);
        assert_eq!(view.accounts[0].address, &first);
        assert!(!view.accounts[0].is_writable);
        assert_eq!(view.accounts[1].address, &second);
        assert!(view.accounts[1].is_writable);
    }

    #[test]
    fn stored_instruction_rejects_small_output_buffer() {
        let program = Address::new_from_array([9; 32]);
        let first = Address::new_from_array([1; 32]);
        let second = Address::new_from_array([2; 32]);
        let metas = [
            StoredAccountMeta::readonly(first),
            StoredAccountMeta::writable(second),
        ];
        let stored = StoredInstruction::new(program, &metas, &[]).unwrap();
        let mut out: [MaybeUninit<InstructionAccount<'_>>; 1] = [MaybeUninit::uninit()];

        assert_eq!(
            stored.to_instruction_view(&mut out).unwrap_err(),
            ProgramError::InvalidArgument
        );
    }
}
