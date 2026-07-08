//! Lazy account parser -- on-demand account deserialization.
//!
//! The standard entrypoint parses every account upfront, burning CU even
//! for accounts the instruction never touches. The lazy parser gives you
//! instruction data and program ID immediately, then hands back an
//! iterator that parses accounts one at a time ON DEMAND.
//!
//! Hopper's lazy path is distinct not because Pinocchio lacks lazy parsing,
//! but because Hopper preserves canonical duplicate-account handling *and*
//! keeps `instruction_data()` / `program_id()` available at any time -- even
//! before a single account is consumed. Pinocchio's lazy entrypoint, by
//! contrast, errors (`InvalidInstructionData`) if you read instruction data
//! before draining every account, because it locates the tail purely by
//! where the account cursor lands. Hopper instead runs a memoized skip-walk
//! to the instruction tail the *first* time `instruction_data()` /
//! `program_id()` is requested, then caches it. If a program never asks for
//! the tail, the walk never runs; if it consumes `k` accounts first, only the
//! remaining records are walked. There is no upfront pre-scan and no
//! zero-fill of the resolved-account array -- both were pure overhead the
//! pre-fusion shape paid on every invocation.
//!
//! # CU Savings
//!
//! Programs that dispatch on `instruction_data[0]` and only need a subset
//! of accounts save measurable CU. A vault program that routes 8 instruction
//! variants through a single entrypoint might only parse 2-3 of 10 accounts
//! for a given variant.
//!
//! # Usage
//!
//! ```ignore
//! use hopper_native::lazy::LazyContext;
//! use hopper_native::hopper_lazy_entrypoint;
//!
//! hopper_lazy_entrypoint!(process);
//!
//! fn process(ctx: LazyContext) -> ProgramResult {
//!     let disc = ctx.instruction_data().first().copied().unwrap_or(0);
//!     match disc {
//!         0 => {
//!             let payer = ctx.next_account()?;
//!             let vault = ctx.next_account()?;
//!             // Remaining accounts are never parsed.
//!             do_deposit(payer, vault, &ctx.instruction_data()[1..])
//!         }
//!         _ => Err(ProgramError::InvalidInstructionData),
//!     }
//! }
//! ```

use core::cell::Cell;
use core::mem::MaybeUninit;

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::raw_account::RuntimeAccount;
use crate::MAX_PERMITTED_DATA_INCREASE;

const BPF_ALIGN_OF_U128: usize = 8;

/// Byte stride from one non-duplicate account record to the next.
///
/// This is the folded, straight-line form of the per-account advance: an
/// 88-byte `RuntimeAccount` header, `data_len` bytes of account data, the
/// `MAX_PERMITTED_DATA_INCREASE` realloc reserve, u128 alignment padding, and
/// the 8-byte rent-epoch tail. It is the pointer-delta specialization of
/// `raw_input::next_record_offset` (private there; this replica should be
/// hoisted into a shared helper -- see followups):
///
/// `next_record_offset(off, dl) - off`
///   `= SIZE + MAX_PERMITTED_DATA_INCREASE + 8 + round_up_8(dl)`
///
/// which is independent of `off` because `SIZE` (88), the reserve (10240),
/// and the rent-epoch tail (8) are all multiples of 8. Correctness of
/// rounding the *relative* `data_len` instead of the absolute address rests
/// on every record start being 8-aligned: the loader serializes the input at
/// `MM_INPUT_START` (8-aligned), and each stride here is a multiple of 8, so
/// `(base + off) % 8 == off % 8` and the folded mask lands on the same byte
/// the old `<*mut u8>::align_offset` did. This compiles to adds + one
/// `and`-mask, dropping the ~6 instructions per account `align_offset` cost
/// (the same class of fix that took `deserialize_accounts` from ~30 to ~8
/// instructions/account).
#[inline(always)]
const fn non_dup_stride(data_len: usize) -> usize {
    RuntimeAccount::SIZE
        + MAX_PERMITTED_DATA_INCREASE
        + 8
        + ((data_len + (BPF_ALIGN_OF_U128 - 1)) & !(BPF_ALIGN_OF_U128 - 1))
}

/// Pre-parsed header from the BPF input buffer: a cursor positioned at the
/// first account, plus enough state to locate instruction data + program ID
/// on demand.
///
/// Accounts are parsed lazily as you call `next_account()`. Instruction data
/// and program ID are found by a memoized skip-walk the first time they are
/// requested (see [`LazyContext::instruction_data`]).
pub struct LazyContext<'info> {
    /// Raw pointer into the BPF input buffer, positioned at the record for
    /// account `parsed_count` (or past the account count if none remain).
    /// Advanced by `next_account()` / `skip()` / `drain_remaining()`.
    cursor: *mut u8,
    /// Number of accounts exposed to the lazy iterator, clamped to the
    /// 254-slot addressable/encoding limit (matches the pre-fusion
    /// `account_count` cap). Bounds `next_account()` and `remaining()`.
    total_accounts: usize,
    /// Real loader account count (may exceed 254). Used only by the tail
    /// skip-walk so the instruction tail is located even when the transaction
    /// declares more accounts than the lazy iterator exposes.
    declared_accounts: usize,
    /// Number of accounts already consumed. Also the initialization
    /// high-water mark for `resolved`: every consume path
    /// (`next_account` / `skip` / `drain_remaining`) writes
    /// `resolved[parsed_count]` before incrementing, so slots
    /// `0..parsed_count` are always initialized and slots
    /// `parsed_count..254` are always uninitialized.
    parsed_count: usize,
    /// Memoized pointer to the instruction-data length prefix (the byte just
    /// past the last account record). Null until the first
    /// `instruction_data()` / `program_id()` call triggers the skip-walk.
    tail: Cell<*const u8>,
    /// Stack of already-parsed AccountViews so we can resolve duplicates
    /// that reference earlier accounts. Fixed size = MAX_TX_ACCOUNTS.
    ///
    /// `MaybeUninit` (not zeroed): the pre-fusion shape `mem::zeroed`'d all
    /// 254 slots (a ~2KB memset) on every invocation. Uninitialized slots are
    /// unreachable: `resolved[i]` is read only for `i < parsed_count`, either
    /// by `get()` / `drain_remaining()` (bounded by `parsed_count`) or by the
    /// duplicate branch of `parse_one_account`, which traps unless
    /// `original_idx < parsed_count`.
    resolved: [MaybeUninit<AccountView<'info>>; 254],
}

// SAFETY: On Solana execution is single-threaded, so the raw account/input
// pointers (and the interior-mutable `tail` cache) in `LazyContext` are never
// shared across threads. Gated to the SVM target (matching `AccountView`) so
// host tools and fuzzers do not rely on cross-thread sharing of these raw
// pointers.
#[cfg(target_os = "solana")]
unsafe impl<'info> Send for LazyContext<'info> {}
#[cfg(target_os = "solana")]
unsafe impl<'info> Sync for LazyContext<'info> {}

impl<'info> LazyContext<'info> {
    /// Walk the remaining account records to the instruction tail, memoizing
    /// the result. Returns a pointer to the 8-byte instruction-data length
    /// prefix.
    ///
    /// Runs at most once: the first `instruction_data()` / `program_id()`
    /// call pays for it, every later call reads the cached pointer. The walk
    /// starts at the current `cursor` (record `parsed_count`) and advances
    /// through `declared_accounts - parsed_count` records, so already-consumed
    /// accounts are never re-walked -- pay-as-you-go. Duplicate markers are
    /// validated exactly as the account-parse path validates them.
    #[inline]
    fn tail_ptr(&self) -> *const u8 {
        let cached = self.tail.get();
        if !cached.is_null() {
            return cached;
        }
        let mut scan = self.cursor as *const u8;
        let mut slot = self.parsed_count;
        // SAFETY: `cursor` sits on the record boundary for account
        // `parsed_count` in the loader input buffer (the consume paths keep
        // that invariant). Each iteration reads the marker byte in bounds and
        // advances by the loader-defined record stride, so `scan` stays on
        // record boundaries until it reaches the instruction tail after
        // `declared_accounts` records. Record starts stay 8-aligned, so
        // `non_dup_stride` is exact (see its docs).
        unsafe {
            while slot < self.declared_accounts {
                let marker = *scan;
                if marker == u8::MAX {
                    let raw = scan as *const RuntimeAccount;
                    let data_len = (*raw).data_len as usize;
                    scan = scan.add(non_dup_stride(data_len));
                } else {
                    let duplicate_of = marker as usize;
                    // Same forward-reference well-formedness rule the parse
                    // path enforces: a marker must point strictly earlier.
                    if duplicate_of >= slot {
                        crate::raw_input::malformed_duplicate_marker(marker, slot);
                    }
                    scan = scan.add(8);
                }
                slot += 1;
            }
        }
        self.tail.set(scan);
        scan
    }

    /// Instruction data for this invocation.
    ///
    /// Available at any time, including before any account is consumed. The
    /// first call (or the first `program_id()` call) runs the memoized
    /// skip-walk to the instruction tail; later calls are cache reads.
    #[inline(always)]
    pub fn instruction_data(&self) -> &[u8] {
        let tail = self.tail_ptr();
        // SAFETY: `tail` points at the 8-byte instruction-data length prefix
        // in the BPF input buffer, immediately followed by that many data
        // bytes. `read_unaligned` avoids assuming pointer alignment (the tail
        // is in fact 8-aligned). The buffer outlives the whole instruction.
        unsafe {
            let len = core::ptr::read_unaligned(tail as *const u64) as usize;
            core::slice::from_raw_parts(tail.add(8), len)
        }
    }

    /// The program ID of this invocation.
    ///
    /// Available at any time (see [`instruction_data`](Self::instruction_data)).
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        let tail = self.tail_ptr();
        // SAFETY: `tail` points at the instruction-data length prefix; the
        // 32-byte program id trails `len` data bytes after it. `Address` is
        // `#[repr(transparent)]` over `[u8; 32]` (alignment 1), so the cast is
        // valid at any offset, and the buffer outlives the returned borrow.
        unsafe {
            let len = core::ptr::read_unaligned(tail as *const u64) as usize;
            &*(tail.add(8 + len) as *const Address)
        }
    }

    /// Number of accounts declared in the transaction.
    #[inline(always)]
    pub fn total_accounts(&self) -> usize {
        self.total_accounts
    }

    /// Number of accounts parsed so far.
    #[inline(always)]
    pub fn parsed_count(&self) -> usize {
        self.parsed_count
    }

    /// Number of accounts remaining to be parsed.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.total_accounts - self.parsed_count
    }

    /// Parse and return the next account from the input buffer.
    ///
    /// Each call advances the internal cursor by one account. Returns
    /// `Err(NotEnoughAccountKeys)` if all accounts have been consumed.
    #[inline]
    pub fn next_account(&mut self) -> Result<AccountView<'info>, ProgramError> {
        if self.parsed_count >= self.total_accounts {
            return Err(ProgramError::NotEnoughAccountKeys);
        }

        // SAFETY: `parsed_count < total_accounts <= declared_accounts`, so the
        // cursor sits on a valid loader-produced account record.
        let view = unsafe { self.parse_one_account() };
        // Initialize slot `parsed_count` before bumping the counter, upholding
        // the `resolved[0..parsed_count]` initialization invariant.
        self.resolved[self.parsed_count] = MaybeUninit::new(view.clone());
        self.parsed_count += 1;
        Ok(view)
    }

    /// Parse the next account and validate it is a signer.
    #[inline]
    pub fn next_signer(&mut self) -> Result<AccountView<'info>, ProgramError> {
        let acct = self.next_account()?;
        acct.require_signer()?;
        Ok(acct)
    }

    /// Parse the next account and validate it is writable.
    #[inline]
    pub fn next_writable(&mut self) -> Result<AccountView<'info>, ProgramError> {
        let acct = self.next_account()?;
        acct.require_writable()?;
        Ok(acct)
    }

    /// Parse the next account and validate it is a writable signer (payer).
    #[inline]
    pub fn next_payer(&mut self) -> Result<AccountView<'info>, ProgramError> {
        let acct = self.next_account()?;
        acct.require_payer()?;
        Ok(acct)
    }

    /// Parse the next account and validate it is owned by `program`.
    #[inline]
    pub fn next_owned_by(&mut self, program: &Address) -> Result<AccountView<'info>, ProgramError> {
        let acct = self.next_account()?;
        acct.require_owned_by(program)?;
        Ok(acct)
    }

    /// Skip `n` accounts without returning them.
    ///
    /// Advances the cursor through the raw buffer, resolving each skipped slot
    /// so `get()` and `drain_remaining()` stay consistent with `parsed_count`.
    /// Constructing an `AccountView` is a single pointer wrap, so this is
    /// materially the same cost as advancing past the record; the duplicate
    /// well-formedness trap fires here too.
    #[inline]
    pub fn skip(&mut self, n: usize) -> Result<(), ProgramError> {
        for _ in 0..n {
            if self.parsed_count >= self.total_accounts {
                return Err(ProgramError::NotEnoughAccountKeys);
            }
            // SAFETY: `parsed_count < total_accounts <= declared_accounts`, so
            // the cursor sits on a valid loader-produced account record.
            let view = unsafe { self.parse_one_account() };
            self.resolved[self.parsed_count] = MaybeUninit::new(view);
            self.parsed_count += 1;
        }
        Ok(())
    }

    /// Collect all remaining accounts into a slice of the internal buffer.
    ///
    /// Parses all remaining accounts eagerly and returns them as a slice.
    /// After this call, `remaining()` returns 0.
    #[inline]
    pub fn drain_remaining(&mut self) -> Result<&[AccountView<'info>], ProgramError> {
        let start = self.parsed_count;
        while self.parsed_count < self.total_accounts {
            // SAFETY: `parsed_count < total_accounts <= declared_accounts`, so
            // the cursor sits on a valid loader-produced account record.
            let view = unsafe { self.parse_one_account() };
            self.resolved[self.parsed_count] = MaybeUninit::new(view);
            self.parsed_count += 1;
        }
        // SAFETY: every slot in `start..parsed_count` was just initialized
        // above (and `0..start` earlier), so this range of `resolved` is fully
        // initialized. `MaybeUninit<AccountView>` has the same layout as
        // `AccountView`, so the reinterpretation as `&[AccountView]` is sound.
        unsafe {
            Ok(core::slice::from_raw_parts(
                self.resolved.as_ptr().add(start) as *const AccountView<'info>,
                self.parsed_count - start,
            ))
        }
    }

    /// Get an already-parsed account by index.
    ///
    /// Returns `None` if `index >= parsed_count`.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&AccountView<'info>> {
        if index < self.parsed_count {
            // SAFETY: `index < parsed_count`, and every slot below
            // `parsed_count` was initialized by a consume path before the
            // counter advanced past it.
            Some(unsafe { self.resolved[index].assume_init_ref() })
        } else {
            None
        }
    }

    /// Parse one account at the current cursor position and advance cursor.
    ///
    /// # Safety
    ///
    /// Caller must ensure `parsed_count < total_accounts` and that `cursor`
    /// points to valid BPF input buffer data.
    #[inline(always)]
    unsafe fn parse_one_account(&mut self) -> AccountView<'info> {
        // SAFETY: caller guarantees `cursor` is on a valid loader-produced
        // account record; the marker byte selects canonical vs duplicate
        // framing and each branch advances by the loader-defined stride.
        unsafe {
            let dup_marker = *self.cursor;

            if dup_marker == u8::MAX {
                // Non-duplicate: RuntimeAccount header starts here.
                let raw = self.cursor as *mut RuntimeAccount;
                let view = AccountView::new_unchecked(raw);
                let data_len = (*raw).data_len as usize;
                // Folded straight-line stride (see `non_dup_stride`), replacing
                // the pre-fusion `align_offset` walk.
                self.cursor = self.cursor.add(non_dup_stride(data_len));
                view
            } else {
                // Duplicate: references an earlier account.
                let original_idx = dup_marker as usize;
                self.cursor = self.cursor.add(8); // skip 8-byte padding
                                                  // The loader guarantees duplicate markers refer to
                                                  // **previously parsed** slots. A marker that points at
                                                  // ourselves or forward is malformed loader input -
                                                  // pre-audit we returned `self.resolved[0]` which is a
                                                  // zeroed `AccountView` until a real account has been
                                                  // parsed, silently handing out a null-pointer view. The
                                                  // Hopper Safety Audit flagged this; we now trap.
                if original_idx >= self.parsed_count {
                    crate::raw_input::malformed_duplicate_marker(dup_marker, self.parsed_count);
                }
                // SAFETY: `original_idx < parsed_count`, so `resolved[original_idx]`
                // was initialized by an earlier consume path.
                self.resolved[original_idx].assume_init_ref().clone()
            }
        }
    }
}

/// Deserialize a BPF input buffer into a `LazyContext`.
///
/// Reads the account count and positions a cursor at the first account.
/// Instruction data and program ID are NOT located here; they are found by a
/// memoized skip-walk the first time [`LazyContext::instruction_data`] /
/// [`LazyContext::program_id`] is called. Individual accounts are parsed on
/// demand by [`LazyContext::next_account`]. There is no upfront account
/// pre-scan and no zero-fill of the resolved-account array.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
#[inline(always)]
pub unsafe fn lazy_deserialize<'info>(input: *mut u8) -> LazyContext<'info> {
    // SAFETY: the first 8 bytes of the BPF input buffer are the account count;
    // `read_unaligned` avoids assuming 8-byte pointer alignment.
    let num_accounts = unsafe { core::ptr::read_unaligned(input as *const u64) as usize };
    // SAFETY: the account records begin immediately after the 8-byte count.
    let accounts_start = unsafe { input.add(8) };
    // Preserve the pre-fusion 254-slot clamp for the iterator-visible count
    // (marker encoding addresses indices 0..=254; slot 254 is skip-only).
    let total_accounts = if num_accounts > 254 {
        254
    } else {
        num_accounts
    };
    // SAFETY: an array of `MaybeUninit` is valid in the uninitialized state by
    // definition; individual slots are initialized before they are read (see
    // the `resolved` field invariant). This replaces the pre-fusion
    // `core::mem::zeroed` memset of all 254 slots.
    let resolved: [MaybeUninit<AccountView<'info>>; 254] =
        unsafe { MaybeUninit::uninit().assume_init() };

    LazyContext {
        cursor: accounts_start,
        total_accounts,
        declared_accounts: num_accounts,
        parsed_count: 0,
        tail: Cell::new(core::ptr::null()),
        resolved,
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec;
    use std::vec::Vec;

    use super::*;
    use crate::raw_input::parse_instruction_frame_checked;

    /// One account slot description for the frame builder.
    enum Slot {
        /// Canonical account: 0xFF marker, header, `data` bytes, realloc
        /// reserve, alignment padding, rent epoch.
        Fresh {
            data_len: usize,
            lamports: u64,
            signer: bool,
        },
        /// Duplicate reference: 1 marker byte + 7 padding bytes.
        Dup(u8),
    }

    fn fresh(data_len: usize, lamports: u64) -> Slot {
        Slot::Fresh {
            data_len,
            lamports,
            signer: true,
        }
    }

    /// 8-aligned loader-input fixture (u64 backing => 8-aligned base, matching
    /// the loader's `MM_INPUT_START` guarantee the folded stride relies on).
    struct Frame {
        words: Vec<u64>,
        byte_len: usize,
    }

    impl Frame {
        fn as_mut_ptr(&mut self) -> *mut u8 {
            self.words.as_mut_ptr() as *mut u8
        }
        fn as_bytes(&self) -> &[u8] {
            // SAFETY: `words` owns at least `byte_len` initialized bytes.
            unsafe { core::slice::from_raw_parts(self.words.as_ptr() as *const u8, self.byte_len) }
        }
    }

    /// Serialize a loader input frame per the Solana BPF loader layout.
    fn build_frame(slots: &[Slot], ix_data: &[u8], program_id: [u8; 32]) -> Frame {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&(slots.len() as u64).to_le_bytes());

        for (i, slot) in slots.iter().enumerate() {
            match slot {
                Slot::Fresh {
                    data_len,
                    lamports,
                    signer,
                } => {
                    let mut header = [0u8; RuntimeAccount::SIZE];
                    header[0] = 0xFF; // canonical marker / borrow_state
                    header[1] = if *signer { 1 } else { 0 }; // is_signer
                    header[2] = 1; // is_writable
                                   // address: recognizable per-slot pattern
                    header[8..40].copy_from_slice(&[i as u8 + 1; 32]);
                    // owner
                    header[40..72].copy_from_slice(&[0x55; 32]);
                    // lamports at offset 72
                    header[72..80].copy_from_slice(&lamports.to_le_bytes());
                    // data_len at offset 80
                    header[80..88].copy_from_slice(&(*data_len as u64).to_le_bytes());
                    buf.extend_from_slice(&header);
                    buf.extend_from_slice(&vec![0xABu8; *data_len]);
                    buf.extend_from_slice(&vec![0u8; MAX_PERMITTED_DATA_INCREASE]);
                    while buf.len() % BPF_ALIGN_OF_U128 != 0 {
                        buf.push(0);
                    }
                    buf.extend_from_slice(&u64::MAX.to_le_bytes()); // rent epoch
                }
                Slot::Dup(of) => {
                    buf.push(*of);
                    buf.extend_from_slice(&[0u8; 7]);
                }
            }
        }

        buf.extend_from_slice(&(ix_data.len() as u64).to_le_bytes());
        buf.extend_from_slice(ix_data);
        buf.extend_from_slice(&program_id);

        let byte_len = buf.len();
        let mut words = vec![0u64; buf.len().div_ceil(8)];
        // SAFETY: `words` has at least `buf.len()` bytes of capacity and the
        // regions do not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), words.as_mut_ptr() as *mut u8, buf.len());
        }
        Frame { words, byte_len }
    }

    const PID: [u8; 32] = [0xC4; 32];

    fn assert_base_aligned(frame: &mut Frame) {
        assert_eq!(
            frame.as_mut_ptr() as usize % 8,
            0,
            "fixture base must be 8-aligned"
        );
    }

    // ── Basic tail resolution (ix-data-anytime) ──────────────────────────

    #[test]
    fn zero_accounts_serves_ix_and_pid_before_any_consume() {
        let mut frame = build_frame(&[], &[9, 8, 7], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        assert_eq!(ctx.total_accounts(), 0);
        assert_eq!(ctx.remaining(), 0);
        assert_eq!(ctx.instruction_data(), &[9, 8, 7]);
        assert_eq!(ctx.program_id().as_array(), &PID);
    }

    #[test]
    fn one_account_ix_before_consume_then_account() {
        let mut frame = build_frame(&[fresh(11, 42)], &[1, 2, 3, 4], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        // ix-data BEFORE consuming any account (the DX edge over Pinocchio).
        assert_eq!(ctx.instruction_data(), &[1, 2, 3, 4]);
        assert_eq!(ctx.program_id().as_array(), &PID);
        // The tail scan must not have disturbed the account cursor.
        let a = ctx.next_account().expect("one account");
        assert_eq!(a.data_len(), 11);
        assert_eq!(a.lamports(), 42);
        assert!(a.is_signer());
        assert_eq!(ctx.remaining(), 0);
        assert!(ctx.next_account().is_err());
    }

    #[test]
    fn ix_after_consuming_all_accounts() {
        let slots = [fresh(3, 1), fresh(0, 2), fresh(9, 3)];
        let mut frame = build_frame(&slots, &[0xEE, 0xEF], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        for _ in 0..3 {
            ctx.next_account().unwrap();
        }
        // Tail scan starts from a fully-advanced cursor (remaining == 0).
        assert_eq!(ctx.instruction_data(), &[0xEE, 0xEF]);
        assert_eq!(ctx.program_id().as_array(), &PID);
    }

    #[test]
    fn ix_after_consuming_some_accounts() {
        let slots = [fresh(5, 1), fresh(6, 2), fresh(7, 3), fresh(8, 4)];
        let mut frame = build_frame(&slots, &[0xD1, 0xD2, 0xD3], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        ctx.next_account().unwrap();
        ctx.next_account().unwrap();
        // Tail scan walks only the remaining two records.
        assert_eq!(ctx.instruction_data(), &[0xD1, 0xD2, 0xD3]);
        // Cursor undisturbed: the next two accounts still parse.
        assert_eq!(ctx.next_account().unwrap().data_len(), 7);
        assert_eq!(ctx.next_account().unwrap().data_len(), 8);
        assert!(ctx.next_account().is_err());
    }

    #[test]
    fn memoized_tail_is_stable_across_calls_and_consumes() {
        let slots = [fresh(4, 1), fresh(5, 2)];
        let mut frame = build_frame(&slots, &[7, 7, 7], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        let d1 = ctx.instruction_data().as_ptr();
        ctx.next_account().unwrap();
        let d2 = ctx.instruction_data().as_ptr();
        ctx.next_account().unwrap();
        let d3 = ctx.instruction_data().as_ptr();
        assert_eq!(d1, d2);
        assert_eq!(d2, d3);
        assert_eq!(ctx.instruction_data(), &[7, 7, 7]);
    }

    // ── Differential against the checked parser ──────────────────────────

    /// Lazy on-demand resolution must agree with the bounds-checked parser on
    /// every canonical record location, duplicate aliasing, the instruction
    /// data span, and the program id offset -- the lazy analog of raw_input's
    /// `fused_walk_agrees_with_checked_parser`.
    fn assert_lazy_agrees(slots: &[Slot], ix_data: &[u8]) {
        let mut frame = build_frame(slots, ix_data, PID);
        assert_base_aligned(&mut frame);
        let bytes = frame.as_bytes().to_vec();
        let checked = parse_instruction_frame_checked(&bytes).expect("well-formed");
        let base = frame.as_mut_ptr() as usize;

        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };

        let visible = checked.account_count.min(254);
        assert_eq!(ctx.total_accounts(), visible);

        for i in 0..visible {
            let view = ctx.next_account().expect("account in range");
            let off = checked.slot_offsets[i];
            let ptr_off = view.account_ptr() as usize - base;
            if bytes[off] == 0xFF {
                assert_eq!(ptr_off, off, "canonical slot {i} pointer mismatch");
            } else {
                let dup_of = bytes[off] as usize;
                assert_eq!(
                    ptr_off, checked.slot_offsets[dup_of],
                    "dup slot {i} must alias canonical {dup_of}"
                );
            }
            // `get()` returns the same materialized view.
            assert_eq!(ctx.get(i).unwrap().account_ptr(), view.account_ptr());
        }

        assert_eq!(
            ctx.instruction_data(),
            &bytes[checked.instruction_data_range.clone()]
        );
        assert_eq!(
            ctx.program_id().as_array().as_slice(),
            &bytes[checked.program_id_offset..checked.program_id_offset + 32]
        );
    }

    #[test]
    fn agrees_zero_accounts() {
        assert_lazy_agrees(&[], &[]);
        assert_lazy_agrees(&[], &[1, 2, 3]);
    }

    #[test]
    fn agrees_one_account() {
        assert_lazy_agrees(&[fresh(0, 1)], &[]); // short frame: empty data + empty ix
        assert_lazy_agrees(&[fresh(1, 1)], &[9]);
    }

    #[test]
    fn agrees_with_duplicates() {
        let slots = [
            fresh(9, 7),
            Slot::Dup(0),
            fresh(3, 8),
            Slot::Dup(2),
            Slot::Dup(0),
        ];
        assert_lazy_agrees(&slots, &[0x11, 0x22]);
    }

    #[test]
    fn agrees_every_data_len_residue() {
        // data_len 0..=7 covers every alignment residue; 8..=15 repeats them.
        for base in [0usize, 8] {
            let slots: Vec<Slot> = (0..8).map(|r| fresh(base + r, r as u64)).collect();
            assert_lazy_agrees(&slots, &[0x42; 5]);
        }
    }

    #[test]
    fn agrees_huge_data_len() {
        let big = 100_003usize; // residue 3 forces nonzero padding
        assert_lazy_agrees(&[fresh(big, 5), fresh(2, 6)], &[0x77, 0x66]);
    }

    #[test]
    fn agrees_exactly_max_254_accounts() {
        // 1 canonical + 253 duplicates = 254 declared, all iterator-visible.
        let mut slots: Vec<Slot> = vec![fresh(4, 9)];
        slots.extend((0..253).map(|_| Slot::Dup(0)));
        assert_eq!(slots.len(), 254);
        assert_lazy_agrees(&slots, &[0x0F; 3]);
    }

    #[test]
    fn max_plus_accounts_clamp_and_tail_still_found() {
        // 1 canonical + 259 duplicates = 260 declared. Iterator exposes 254;
        // the remaining records are skip-only but the tail scan must still
        // walk past them to reach ix data + program id.
        let mut slots: Vec<Slot> = vec![fresh(4, 9)];
        slots.extend((0..259).map(|_| Slot::Dup(0)));
        let mut frame = build_frame(&slots, &[0x0F; 3], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        assert_eq!(ctx.total_accounts(), 254);
        // ix-data available before consuming, even though 260 > 254 records
        // must be skip-walked past to find the tail.
        assert_eq!(ctx.instruction_data(), &[0x0F; 3]);
        assert_eq!(ctx.program_id().as_array(), &PID);
        // Consume up to the clamp.
        for _ in 0..254 {
            ctx.next_account().unwrap();
        }
        assert_eq!(ctx.remaining(), 0);
        assert!(ctx.next_account().is_err());
    }

    // ── skip / drain / get ───────────────────────────────────────────────

    #[test]
    fn skip_advances_and_populates_get() {
        let slots = [fresh(1, 1), fresh(2, 2), fresh(3, 3)];
        let mut frame = build_frame(&slots, &[0xAB], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        ctx.skip(2).unwrap();
        assert_eq!(ctx.parsed_count(), 2);
        // Skipped slots are materialized (sound `get`, not a zeroed view).
        assert_eq!(ctx.get(0).unwrap().data_len(), 1);
        assert_eq!(ctx.get(1).unwrap().data_len(), 2);
        assert!(ctx.get(2).is_none());
        assert_eq!(ctx.next_account().unwrap().data_len(), 3);
        assert_eq!(ctx.instruction_data(), &[0xAB]);
    }

    #[test]
    fn skip_past_end_errors() {
        let slots = [fresh(1, 1)];
        let mut frame = build_frame(&slots, &[], PID);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        assert!(ctx.skip(2).is_err());
    }

    #[test]
    fn drain_remaining_returns_all() {
        let slots = [fresh(4, 1), fresh(5, 2), fresh(6, 3)];
        let mut frame = build_frame(&slots, &[0x01], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        ctx.next_account().unwrap();
        let rest = ctx.drain_remaining().unwrap();
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].data_len(), 5);
        assert_eq!(rest[1].data_len(), 6);
        assert_eq!(ctx.remaining(), 0);
        assert_eq!(ctx.instruction_data(), &[0x01]);
    }

    #[test]
    fn drain_with_duplicates_aliases_canonical() {
        let slots = [fresh(9, 1), Slot::Dup(0), fresh(3, 2)];
        let mut frame = build_frame(&slots, &[], PID);
        assert_base_aligned(&mut frame);
        // SAFETY: well-formed 8-aligned loader-layout fixture.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        let all = ctx.drain_remaining().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            all[0].account_ptr(),
            all[1].account_ptr(),
            "dup aliases canonical"
        );
        assert_ne!(all[0].account_ptr(), all[2].account_ptr());
        assert_eq!(all[1].data_len(), 9);
    }

    // ── malformed input traps ────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "malformed duplicate marker")]
    fn forward_duplicate_traps_on_consume() {
        let slots = [fresh(1, 1), Slot::Dup(1)]; // self-reference at slot 1
        let mut frame = build_frame(&slots, &[], PID);
        // SAFETY: buffer layout is loader-shaped; the malformed marker is the
        // condition under test and traps before any OOB access.
        let mut ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        ctx.next_account().unwrap();
        let _ = ctx.next_account();
    }

    #[test]
    #[should_panic(expected = "malformed duplicate marker")]
    fn forward_duplicate_traps_in_tail_scan() {
        // ix-data requested before consuming: the tail skip-walk must catch a
        // forward duplicate marker just as the parse path would.
        let slots = [fresh(1, 1), Slot::Dup(5)];
        let mut frame = build_frame(&slots, &[], PID);
        // SAFETY: buffer layout is loader-shaped; the malformed marker is the
        // condition under test and traps before any OOB access.
        let ctx = unsafe { lazy_deserialize(frame.as_mut_ptr()) };
        let _ = ctx.instruction_data();
    }
}
