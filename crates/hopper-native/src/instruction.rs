//! CPI instruction types: InstructionView, InstructionAccount, Seed, Signer.
//!
//! These types match the Solana runtime's C ABI for cross-program invocation.
//! Wire-compatible with pinocchio/solana-instruction-view types.

use core::marker::PhantomData;
use crate::address::Address;
use crate::account_view::AccountView;

// ── InstructionAccount ───────────────────────────────────────────────

/// Metadata for an account referenced in a CPI instruction.
#[repr(C)]
#[derive(Debug, Clone)]
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
        Self { address, is_writable, is_signer }
    }

    /// Read-only, non-signer.
    #[inline(always)]
    pub const fn readonly(address: &'a Address) -> Self {
        Self { address, is_writable: false, is_signer: false }
    }

    /// Writable, non-signer.
    #[inline(always)]
    pub const fn writable(address: &'a Address) -> Self {
        Self { address, is_writable: true, is_signer: false }
    }

    /// Read-only signer.
    #[inline(always)]
    pub const fn readonly_signer(address: &'a Address) -> Self {
        Self { address, is_writable: false, is_signer: true }
    }

    /// Writable signer.
    #[inline(always)]
    pub const fn writable_signer(address: &'a Address) -> Self {
        Self { address, is_writable: true, is_signer: true }
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

// ── CpiAccount ───────────────────────────────────────────────────────

/// C-ABI account info passed to `sol_invoke_signed_c`.
///
/// This matches the Solana runtime's expected layout for CPI account infos.
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

impl<'a> From<&'a AccountView> for CpiAccount<'a> {
    #[inline]
    fn from(view: &'a AccountView) -> Self {
        let raw = view.account_ptr();
        Self {
            address: unsafe { &(*raw).address as *const Address },
            lamports: unsafe { &(*raw).lamports as *const u64 },
            data_len: view.data_len() as u64,
            data: view.data_ptr(),
            owner: unsafe { &(*raw).owner as *const Address },
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
