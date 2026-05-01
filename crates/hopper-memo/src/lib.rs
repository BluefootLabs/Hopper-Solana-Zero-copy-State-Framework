//! Hopper-owned SPL Memo program builder.
//!
//! The SPL Memo program records arbitrary UTF-8 byte payloads in
//! transaction logs and asserts that a list of accounts signed the
//! containing transaction. It is the canonical primitive for on-chain
//! metadata stamping (off-chain reference numbers, orderbook IDs,
//! arbitrary protocol tags) without spinning up program-owned state.
//!
//! ## Programs
//!
//! - `MEMO_PROGRAM_ID` — Memo v2, the default and overwhelming majority case.
//! - [`v1::MEMO_V1_PROGRAM_ID`] — legacy Memo v1, kept available for
//!   protocols still pinned to the original program. New code should
//!   prefer Memo v2.
//!
//! ## Quick start
//!
//! ```ignore
//! use hopper_memo::Memo;
//!
//! Memo {
//!     signers: &[user_view],
//!     memo: b"order=42",
//!     program_id: None,
//! }
//! .invoke()?;
//! ```
//!
//! Memo strings can be empty; the program enforces only the signer
//! constraints. The memo body is passed verbatim as the instruction
//! data. UTF-8 framing is the caller's responsibility.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use core::mem::MaybeUninit;

use hopper_runtime::account::AccountView;
use hopper_runtime::address::Address;
use hopper_runtime::error::ProgramError;
use hopper_runtime::instruction::{InstructionAccount, InstructionView, Signer};
use hopper_runtime::ProgramResult;

/// SPL Memo v2 program id: `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`.
///
/// This is the default Memo program. Use [`v1::MEMO_V1_PROGRAM_ID`] only
/// for legacy compatibility.
pub const MEMO_PROGRAM_ID: Address =
    hopper_runtime::address!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

/// Maximum signer accounts a single memo invocation may cite.
///
/// Matches Pinocchio's `MAX_STATIC_CPI_ACCOUNTS` ceiling. The Memo
/// program itself accepts an unbounded list, but heap-free CPI on
/// SBF requires a static cap.
pub const MAX_MEMO_SIGNERS: usize = 16;

/// Legacy SPL Memo v1 helpers.
///
/// The v1 program (`Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo`) is
/// frozen and only kept here for protocols anchored to it. New code
/// should prefer v2 via [`MEMO_PROGRAM_ID`].
pub mod v1 {
    use hopper_runtime::address::Address;

    /// SPL Memo v1 program id: `Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo`.
    pub const MEMO_V1_PROGRAM_ID: Address =
        hopper_runtime::address!("Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo");
}

/// SPL Memo CPI builder.
///
/// `signers` are the accounts the memo program will assert signed the
/// surrounding transaction; pass an empty slice for unauthenticated
/// memos (the program then only logs the bytes). `memo` is the raw
/// payload — UTF-8 framing is the caller's responsibility.
///
/// `program_id` selects the target program. Default (`None`) uses
/// [`MEMO_PROGRAM_ID`] (Memo v2). Pass `Some(&v1::MEMO_V1_PROGRAM_ID)`
/// for the legacy program.
///
/// The struct holds borrowed references only; nothing is allocated on
/// the heap.
pub struct Memo<'a, 'b, 'c> {
    /// Signing accounts the Memo program will validate.
    pub signers: &'a [&'a AccountView],
    /// Raw memo payload.
    pub memo: &'b [u8],
    /// Target program. `None` = Memo v2 (default).
    pub program_id: Option<&'c Address>,
}

impl Memo<'_, '_, '_> {
    /// Invoke the Memo program with no PDA signer seeds.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Invoke the Memo program, supplying PDA signer seeds.
    ///
    /// Any signer in `self.signers` whose address is a PDA must have
    /// its derivation seeds in `signers_seeds`; the runtime will sign
    /// the inner CPI on its behalf.
    pub fn invoke_signed(&self, signers_seeds: &[Signer]) -> ProgramResult {
        let n = self.signers.len();
        if n > MAX_MEMO_SIGNERS {
            return Err(ProgramError::InvalidArgument);
        }

        // Build the InstructionAccount array on the stack. We use
        // MaybeUninit so we don't need a Default / Copy bound on
        // InstructionAccount, mirroring the Pinocchio shape.
        let mut accounts: [MaybeUninit<InstructionAccount>; MAX_MEMO_SIGNERS] =
            [const { MaybeUninit::uninit() }; MAX_MEMO_SIGNERS];

        let mut i = 0;
        while i < n {
            accounts[i].write(InstructionAccount::readonly_signer(
                self.signers[i].address(),
            ));
            i += 1;
        }

        // SAFETY: the first `n` slots have been initialised in the
        // loop above; we hand only that prefix to InstructionView.
        let accounts_slice: &[InstructionAccount] = unsafe {
            core::slice::from_raw_parts(accounts.as_ptr() as *const InstructionAccount, n)
        };

        let pid = self.program_id.unwrap_or(&MEMO_PROGRAM_ID);
        let instruction = InstructionView {
            program_id: pid,
            data: self.memo,
            accounts: accounts_slice,
        };

        macro_rules! invoke_with_signers {
            ($n:literal, [$($idx:literal),*]) => {{
                let account_views: [&AccountView; $n] = [$(self.signers[$idx]),*];
                hopper_runtime::cpi::invoke_signed::<$n>(&instruction, &account_views, signers_seeds)
            }};
        }

        match n {
            0 => invoke_with_signers!(0, []),
            1 => invoke_with_signers!(1, [0]),
            2 => invoke_with_signers!(2, [0, 1]),
            3 => invoke_with_signers!(3, [0, 1, 2]),
            4 => invoke_with_signers!(4, [0, 1, 2, 3]),
            5 => invoke_with_signers!(5, [0, 1, 2, 3, 4]),
            6 => invoke_with_signers!(6, [0, 1, 2, 3, 4, 5]),
            7 => invoke_with_signers!(7, [0, 1, 2, 3, 4, 5, 6]),
            8 => invoke_with_signers!(8, [0, 1, 2, 3, 4, 5, 6, 7]),
            9 => invoke_with_signers!(9, [0, 1, 2, 3, 4, 5, 6, 7, 8]),
            10 => invoke_with_signers!(10, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
            11 => invoke_with_signers!(11, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
            12 => invoke_with_signers!(12, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
            13 => invoke_with_signers!(13, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]),
            14 => invoke_with_signers!(14, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]),
            15 => invoke_with_signers!(15, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]),
            16 => invoke_with_signers!(16, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
            _ => Err(ProgramError::InvalidArgument),
        }
    }
}
