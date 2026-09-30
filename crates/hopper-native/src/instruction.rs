//! CPI instruction types: InstructionView, InstructionAccount, Seed, Signer.
//!
//! These types match the Solana runtime's C ABI for cross-program invocation.
//! Matching the C descriptor ABI does not make Rust types interchangeable with
//! types defined in other crates. Construct Hopper descriptors explicitly.

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::raw_account::RuntimeAccount;
use crate::{ProgramResult, NOT_BORROWED};
use core::marker::PhantomData;

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

impl<'a> From<&'a AccountView<'a>> for InstructionAccount<'a> {
    #[inline(always)]
    fn from(view: &'a AccountView<'a>) -> Self {
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
    _account_view: PhantomData<&'a AccountView<'a>>,
}

// The flag bytes are copied as one word (see `From<&AccountView>` below):
// the three `bool` fields must be adjacent, in header order, with a
// padding byte after them for the fourth byte of the word.
const _: () = {
    use core::mem::{offset_of, size_of};
    let signer = offset_of!(CpiAccount<'static>, is_signer);
    assert!(offset_of!(CpiAccount<'static>, is_writable) == signer + 1);
    assert!(offset_of!(CpiAccount<'static>, executable) == signer + 2);
    assert!(signer + 4 <= size_of::<CpiAccount<'static>>());
};

impl<'a> From<&'a AccountView<'a>> for CpiAccount<'a> {
    #[inline(always)]
    fn from(view: &'a AccountView<'a>) -> Self {
        let raw = view.account_ptr();
        let mut out = core::mem::MaybeUninit::<Self>::uninit();
        let slot = out.as_mut_ptr();
        // The loader writes `is_signer`, `is_writable` and `executable` as 0
        // or 1 into header bytes 1..4, which are the byte values of `bool`,
        // and the three fields sit together here in the same order, so one
        // 4-byte copy moves all three (the fourth byte lands in this
        // struct's padding). Turning each flag into a `bool` separately
        // costs a branch per flag on SBF, which has no set-on-condition
        // instruction.
        // SAFETY: `raw` is the view's live header; `addr_of!` takes field
        // addresses without forming references. Every field of `slot` is
        // written before `assume_init`: the five pointers and two integers
        // one by one, the three flags by the word copy, whose bytes are 0 or
        // 1 as the loader wrote them, valid `bool`s.
        unsafe {
            core::ptr::addr_of_mut!((*slot).address).write(core::ptr::addr_of!((*raw).address));
            core::ptr::addr_of_mut!((*slot).lamports).write(core::ptr::addr_of!((*raw).lamports));
            core::ptr::addr_of_mut!((*slot).data_len).write(view.data_len() as u64);
            core::ptr::addr_of_mut!((*slot).data).write(view.data_ptr_unchecked());
            core::ptr::addr_of_mut!((*slot).owner).write(core::ptr::addr_of!((*raw).owner));
            core::ptr::addr_of_mut!((*slot).rent_epoch).write(0);
            let flags = core::ptr::read_unaligned((raw as *const u8).add(1) as *const u32);
            (core::ptr::addr_of_mut!((*slot).is_signer) as *mut u32).write_unaligned(flags);
            out.assume_init()
        }
    }
}

impl<'a> CpiAccount<'a> {
    /// Rebuild one instruction meta from protocol-declared flags.
    ///
    /// The flags deliberately do not come from the outer account view: a PDA
    /// may be a signer only for this CPI, and an outer-writable account may be
    /// intentionally read-only to the callee.
    #[inline(always)]
    pub(crate) fn instruction_account(
        &self,
        is_writable: bool,
        is_signer: bool,
    ) -> InstructionAccount<'a> {
        // SAFETY: `CpiAccount::from` captured this pointer from an
        // `AccountView<'a>` and the private fields prevent safe fabrication.
        let address = unsafe { &*self.address };
        InstructionAccount::new(address, is_writable, is_signer)
    }
}

/// Validate the borrow state and writable privilege encoded by specialized
/// CPI builders before entering a syscall.
///
/// `writable_mask` describes the callee instruction metas, not the outer
/// transaction privileges: bit `i` is set when account `i` will be writable
/// in the CPI. Read-only metas need shared-borrow compatibility; writable
/// metas need both outer writable privilege and exclusive-borrow compatibility.
#[inline(always)]
pub(crate) fn preflight_cpi_accounts(
    accounts: &[CpiAccount<'_>],
    writable_mask: usize,
    signer_mask: usize,
    has_pda_signers: bool,
) -> ProgramResult {
    let mut index = 0usize;
    while index < accounts.len() {
        let account = &accounts[index];
        // With no PDA signer seeds, a missing outer signature cannot be
        // satisfied by the runtime. Match the generic checked invoke path.
        // Nonempty seeds are NOT proof of authority: the SVM derives and
        // authenticates the instruction's PDA signers at the syscall boundary.
        if signer_mask & (1usize << index) != 0 && !account.is_signer && !has_pda_signers {
            return Err(ProgramError::MissingRequiredSignature);
        }
        let is_writable_meta = writable_mask & (1usize << index) != 0;
        if is_writable_meta && !account.is_writable {
            return Err(ProgramError::Immutable);
        }

        // `CpiAccount::from` always derives `data` from the byte immediately
        // after its RuntimeAccount header. The fields are private, so safe
        // callers cannot synthesize a CpiAccount with a different relation.
        let raw = unsafe { account.data.sub(RuntimeAccount::SIZE) as *const RuntimeAccount };
        // SAFETY: `raw` was recovered from the invariant above and remains
        // valid for the `CpiAccount` lifetime.
        let borrow_state = unsafe { (*raw).borrow_state };
        let compatible = if is_writable_meta {
            borrow_state == NOT_BORROWED
        } else {
            borrow_state != 0
        };
        if !compatible {
            return Err(ProgramError::AccountBorrowFailed);
        }

        index += 1;
    }
    Ok(())
}

// Pin the two C structures handed to `sol_invoke_signed_c`. Rust `bool` is one
// byte, matching the syscall ABI's byte flags; the tail padding rounds each
// record to pointer alignment.
const _: () = {
    assert!(core::mem::size_of::<InstructionAccount<'static>>() == 16);
    assert!(core::mem::align_of::<InstructionAccount<'static>>() == 8);
    assert!(core::mem::offset_of!(InstructionAccount<'static>, address) == 0);
    assert!(core::mem::offset_of!(InstructionAccount<'static>, is_writable) == 8);
    assert!(core::mem::offset_of!(InstructionAccount<'static>, is_signer) == 9);

    assert!(core::mem::size_of::<CpiAccount<'static>>() == 56);
    assert!(core::mem::align_of::<CpiAccount<'static>>() == 8);
    assert!(core::mem::offset_of!(CpiAccount<'static>, address) == 0);
    assert!(core::mem::offset_of!(CpiAccount<'static>, lamports) == 8);
    assert!(core::mem::offset_of!(CpiAccount<'static>, data_len) == 16);
    assert!(core::mem::offset_of!(CpiAccount<'static>, data) == 24);
    assert!(core::mem::offset_of!(CpiAccount<'static>, owner) == 32);
    assert!(core::mem::offset_of!(CpiAccount<'static>, rent_epoch) == 40);
    assert!(core::mem::offset_of!(CpiAccount<'static>, is_signer) == 48);
    assert!(core::mem::offset_of!(CpiAccount<'static>, is_writable) == 49);
    assert!(core::mem::offset_of!(CpiAccount<'static>, executable) == 50);
};

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
        // SAFETY: `seed` and `len` were taken from one `&'a [u8]` in the
        // constructor, and the `PhantomData` ties `self` to that borrow.
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
