//! Borrowed-state execution context.
//!
//! The `Frame` is Hopper's execution model. It wraps the instruction's accounts
//! and data, enforcing single-mutable-borrow discipline and phased execution.
//!
//! ## Execution Phases
//!
//! 1. **Resolve** -- Parse accounts from the input slice into named typed slots
//! 2. **Validate** -- Run the validation graph (account-local, cross-account, state-transition)
//! 3. **Borrow** -- Obtain zero-copy overlays with borrow discipline
//! 4. **Mutate** -- Execute state changes through verified mutable references
//! 5. **Emit** -- Fire events
//! 6. **Commit** -- (implicit: Solana runtime commits on success)
//!
//! The `Frame` ensures that:
//! - Each account is borrowed at most once mutably
//! - Immutable borrows can coexist
//! - Validation runs before mutation
//! - Events are emitted after state changes

pub mod phase;
pub mod args;

use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult, Ref, RefMut};
use hopper_runtime::segment_borrow::SegmentBorrowRegistry;
use crate::account::SliceCursor;
use crate::account::{Pod, FixedLayout, HEADER_LEN};

/// Maximum accounts in a single frame. Matches Solana's transaction limit.
pub const MAX_FRAME_ACCOUNTS: usize = 64;

/// Execution frame holding the instruction's accounts and data.
///
/// `Frame` is the entry point for Hopper's phased execution model.
/// It tracks which accounts have been borrowed (mutably or immutably)
/// to prevent aliasing violations at runtime.
pub struct Frame<'a> {
    /// Program ID that is executing.
    program_id: &'a Address,
    /// Raw account views.
    accounts: &'a [AccountView],
    /// Instruction data cursor.
    ix_data: SliceCursor<'a>,
    /// Borrow tracking: bit N = 1 means account N is mutably borrowed.
    /// This is a runtime check -- not as strong as the borrow checker, but
    /// catches the most dangerous pattern (double-mutable-borrow).
    mutable_borrows: u64,
    /// Segment-level borrow tracking for fine-grained conflict detection.
    /// Allows concurrent mutable access to non-overlapping regions of the
    /// same account — the key safety innovation over raw Pinocchio.
    segment_borrows: SegmentBorrowRegistry,
}

impl<'a> Frame<'a> {
    /// Create a new execution frame.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView],
        instruction_data: &'a [u8],
    ) -> Result<Self, ProgramError> {
        if accounts.len() > MAX_FRAME_ACCOUNTS {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(Self {
            program_id,
            accounts,
            ix_data: SliceCursor::new(instruction_data),
            mutable_borrows: 0,
            segment_borrows: SegmentBorrowRegistry::new(),
        })
    }

    /// Program ID.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        self.program_id
    }

    /// Number of accounts in this frame.
    #[inline(always)]
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Get raw account view by index.
    #[inline(always)]
    pub fn account_view(&self, index: usize) -> Result<&AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get instruction data cursor.
    #[inline(always)]
    pub fn ix_data(&mut self) -> &mut SliceCursor<'a> {
        &mut self.ix_data
    }

    /// Get raw instruction data.
    #[inline(always)]
    pub fn ix_data_raw(&self) -> &[u8] {
        self.ix_data.data_from_position()
    }

    // --- Immutable Account Access -----------------------------------

    /// Get an immutable account view (no borrow tracking needed for reads).
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<FrameAccount<'_>, ProgramError> {
        let view = self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)?;
        Ok(FrameAccount { view })
    }

    // --- Mutable Account Access (with borrow tracking) -------------

    /// Get a mutable account view with runtime borrow checking.
    ///
    /// Returns an error if this account is already borrowed mutably.
    /// This prevents the most dangerous aliasing pattern in Solana programs.
    #[inline]
    pub fn account_mut(
        &mut self,
        index: usize,
    ) -> Result<FrameAccountMut<'_>, ProgramError> {
        if index >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }

        let bit = 1u64 << (index as u32);
        if self.mutable_borrows & bit != 0 {
            // Already mutably borrowed -- prevent aliasing.
            return Err(ProgramError::AccountBorrowFailed);
        }

        self.mutable_borrows |= bit;
        let view = &self.accounts[index];

        Ok(FrameAccountMut {
            view,
            borrow_mask: &mut self.mutable_borrows,
            bit,
        })
    }

    // --- Segment-Level Access (fine-grained borrow tracking) --------

    /// Get the segment borrow registry for direct manipulation.
    #[inline(always)]
    pub fn segment_borrows(&self) -> &SegmentBorrowRegistry {
        &self.segment_borrows
    }

    /// Get the mutable segment borrow registry.
    #[inline(always)]
    pub fn segment_borrows_mut(&mut self) -> &mut SegmentBorrowRegistry {
        &mut self.segment_borrows
    }

    /// Read a typed value from a segment of an account's data region.
    ///
    /// Registers a **read** borrow for the given byte range, then projects
    /// the pointer through the live byte-borrow guard into a `Ref<'_, T>`.
    /// Returns an error if the range conflicts with an existing write
    /// borrow on the same account.
    ///
    /// `offset` is relative to the layout body (after the 16-byte header).
    ///
    /// # Safety Contract
    ///
    /// - T must be `Pod + FixedLayout` (safe to interpret from any bit pattern,
    ///   alignment-1, no padding).
    /// - Bounds are checked at runtime.
    /// - Borrow conflicts are checked at runtime.
    /// - The returned guard owns the underlying borrow; dropping it
    ///   releases the read borrow on the account state byte. **Earlier
    ///   versions of this method dropped the byte-slice guard before
    ///   returning the typed reference, leaving a dangling pointer
    ///   tracked by stale borrow state — this version fixes that.**
    #[inline]
    pub fn segment_ref<T: Pod + FixedLayout>(
        &mut self,
        index: usize,
        offset: u32,
    ) -> Result<Ref<'_, T>, ProgramError> {
        let view = self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)?;
        let data = view.try_borrow()?;

        let abs_offset = (HEADER_LEN as u32)
            .checked_add(offset)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let end = abs_offset
            .checked_add(T::SIZE as u32)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end as usize > data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        self.segment_borrows.register_read(
            view.address(),
            abs_offset,
            T::SIZE as u32,
        )?;

        // SAFETY: T is Pod + FixedLayout (all bit patterns valid, align-1).
        // Bounds checked above. The pointer lives inside the byte slice
        // owned by `data`; `Ref::project` consumes `data` and produces a
        // `Ref<T>` that keeps the underlying account borrow alive for the
        // returned reference's lifetime.
        let ptr = unsafe { data.as_bytes_ptr().add(abs_offset as usize) as *const T };
        Ok(unsafe { data.project(ptr) })
    }

    /// Get a mutable typed reference to a segment of an account's data.
    ///
    /// Registers a **write** borrow for the given byte range, then projects
    /// the pointer through the live byte-borrow guard into a `RefMut<'_, T>`.
    /// Returns an error if the range overlaps any existing borrow (read or
    /// write) on the same account.
    ///
    /// This is the core primitive that makes Hopper strictly better than
    /// raw Pinocchio: you get the same pointer arithmetic, but with
    /// segment-level conflict detection that prevents aliasing bugs.
    ///
    /// `offset` is relative to the layout body (after the 16-byte header).
    ///
    /// # Safety Contract
    ///
    /// - T must be `Pod + FixedLayout`.
    /// - Bounds are checked at runtime.
    /// - Borrow conflicts are checked at runtime.
    /// - The returned `RefMut<T>` keeps the exclusive borrow alive for
    ///   its full lifetime — no naked `&mut T` over a released guard.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Only borrows the "balance" region [32..40), not the entire account.
    /// {
    ///     let mut balance = frame.segment_mut::<WireU64>(0, 32)?;
    ///     balance.set(balance.get() + amount);
    /// } // RefMut drops here, releasing the borrow.
    ///
    /// // Now we can re-borrow another segment safely.
    /// let mut metadata = frame.segment_mut::<VaultMetadata>(0, 40)?;
    /// ```
    #[inline]
    pub fn segment_mut<T: Pod + FixedLayout>(
        &mut self,
        index: usize,
        offset: u32,
    ) -> Result<RefMut<'_, T>, ProgramError> {
        let view = self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)?;

        // Check writable before doing anything else.
        if !view.is_writable() {
            return Err(ProgramError::InvalidAccountData);
        }

        let data = view.try_borrow_mut()?;
        let abs_offset = (HEADER_LEN as u32)
            .checked_add(offset)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let end = abs_offset
            .checked_add(T::SIZE as u32)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end as usize > data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        self.segment_borrows.register_write(
            view.address(),
            abs_offset,
            T::SIZE as u32,
        )?;

        // SAFETY: as above; `RefMut::project` consumes the byte-slice
        // RefMut and produces a `RefMut<T>` that holds the exclusive
        // account borrow for its lifetime.
        let bytes_ptr = (&*data) as *const [u8] as *mut [u8] as *mut u8;
        let ptr = unsafe { bytes_ptr.add(abs_offset as usize) as *mut T };
        Ok(unsafe { data.project(ptr) })
    }

    /// Unsafe escape hatch for performance-critical paths.
    ///
    /// Skips borrow tracking entirely. The caller takes full responsibility
    /// for aliasing safety. Returns a `RefMut<T>` so the borrow guard is
    /// still tied to the returned value's lifetime — the "unchecked" part
    /// is only the conflict-detection skip, not the lifetime tying.
    ///
    /// # Safety
    ///
    /// The caller must guarantee no other mutable reference to the same
    /// byte range exists for the duration of the returned reference, and
    /// that no overlapping segment borrow has been registered.
    #[inline(always)]
    pub unsafe fn segment_mut_unchecked<T: Pod + FixedLayout>(
        &self,
        index: usize,
        offset: u32,
    ) -> Result<RefMut<'_, T>, ProgramError> {
        let view = self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)?;
        let data = view.try_borrow_mut()?;

        let abs_offset = (HEADER_LEN as u32)
            .checked_add(offset)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let end = abs_offset
            .checked_add(T::SIZE as u32)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end as usize > data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        let bytes_ptr = (&*data) as *const [u8] as *mut [u8] as *mut u8;
        let ptr = unsafe { bytes_ptr.add(abs_offset as usize) as *mut T };
        Ok(unsafe { data.project(ptr) })
    }

    // --- Validation Helpers -----------------------------------------

    /// Validate that account at `index` is a signer.
    #[inline(always)]
    pub fn require_signer(&self, index: usize) -> ProgramResult {
        crate::check::check_signer(self.account_view(index)?)
    }

    /// Validate that account at `index` is writable.
    #[inline(always)]
    pub fn require_writable(&self, index: usize) -> ProgramResult {
        crate::check::check_writable(self.account_view(index)?)
    }

    /// Validate that account at `index` is owned by this program.
    #[inline(always)]
    pub fn require_owned(&self, index: usize) -> ProgramResult {
        crate::check::check_owner(self.account_view(index)?, self.program_id)
    }

    /// Validate signer + writable (common pattern for authority accounts).
    #[inline(always)]
    pub fn require_authority(&self, index: usize) -> ProgramResult {
        let view = self.account_view(index)?;
        crate::check::check_signer(view)?;
        crate::check::check_writable(view)?;
        Ok(())
    }

    /// Validate two accounts are unique.
    #[inline(always)]
    pub fn require_unique(&self, a: usize, b: usize) -> ProgramResult {
        let va = self.account_view(a)?;
        let vb = self.account_view(b)?;
        crate::check::check_accounts_unique(va, vb)
    }

    /// Require an account matches a specific program address.
    #[inline(always)]
    pub fn require_program(&self, index: usize, program: &Address) -> ProgramResult {
        crate::check::check_address(self.account_view(index)?, program)
    }
}

/// Immutable account view within a Frame.
pub struct FrameAccount<'a> {
    view: &'a AccountView,
}

impl<'a> FrameAccount<'a> {
    /// The underlying AccountView.
    #[inline(always)]
    pub fn view(&self) -> &AccountView {
        self.view
    }

    /// The account's address.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        self.view.address()
    }

    /// Borrow account data (read-only).
    #[inline(always)]
    pub fn data(&self) -> Result<Ref<'a, [u8]>, ProgramError> {
        self.view.try_borrow()
    }

    /// Lamports balance.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.view.lamports()
    }

    /// Is this account a signer?
    #[inline(always)]
    pub fn is_signer(&self) -> bool {
        self.view.is_signer()
    }

    /// Is this account writable?
    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        self.view.is_writable()
    }
}

/// Mutable account view within a Frame.
///
/// When this is dropped, the mutable borrow tracking bit is cleared,
/// allowing the account to be re-borrowed.
pub struct FrameAccountMut<'a> {
    view: &'a AccountView,
    borrow_mask: &'a mut u64,
    bit: u64,
}

impl<'a> FrameAccountMut<'a> {
    /// The underlying AccountView.
    #[inline(always)]
    pub fn view(&self) -> &AccountView {
        self.view
    }

    /// The account's address.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        self.view.address()
    }

    /// Borrow account data (read-only).
    #[inline(always)]
    pub fn data(&self) -> Result<Ref<'a, [u8]>, ProgramError> {
        self.view.try_borrow()
    }

    /// Borrow account data (mutable).
    #[inline(always)]
    pub fn data_mut(&self) -> Result<RefMut<'a, [u8]>, ProgramError> {
        self.view.try_borrow_mut()
    }

    /// Lamports balance.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.view.lamports()
    }
}

impl<'a> Drop for FrameAccountMut<'a> {
    fn drop(&mut self) {
        // Release the borrow tracking bit.
        *self.borrow_mask &= !self.bit;
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Audit regression tests
// ══════════════════════════════════════════════════════════════════════
//
// Lock in the Hopper Safety Audit's top-priority fix: Frame's segment
// accessors now hand back `Ref<T>` / `RefMut<T>` that keep the
// underlying account borrow alive for their full lifetime. The
// pre-audit version dropped the byte-slice guard before returning the
// typed reference, which is silent UB. These tests prove the guard is
// still live at use time.
#[cfg(all(test, feature = "hopper-native-backend"))]
mod audit_tests {
    use super::*;
    use hopper_native::{
        Address as NativeAddress, NOT_BORROWED, RuntimeAccount,
        AccountView as NativeAccountView,
    };

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Counter {
        value: u64,
    }

    unsafe impl hopper_runtime::Pod for Counter {}

    impl crate::account::FixedLayout for Counter {
        const SIZE: usize = 8;
    }

    fn make_account(data_len: usize, seed: u8) -> (std::vec::Vec<u8>, AccountView) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([seed; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports: 42,
                data_len: data_len as u64,
            });
        }
        // Zero the Hopper header region so the frame doesn't trip on
        // uninitialized bytes later.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let view = unsafe {
            core::mem::transmute::<NativeAccountView, AccountView>(backend)
        };
        (backing, view)
    }

    fn new_frame<'a>(
        program_id: &'a Address,
        accounts: &'a [AccountView],
    ) -> Frame<'a> {
        Frame::new(program_id, accounts, &[]).unwrap()
    }

    #[test]
    fn frame_segment_mut_writes_through_ref_mut() {
        // This test is the ground-truth for the audit fix: the fact
        // that we can write through `RefMut<Counter>` returned by
        // `Frame::segment_mut` and see the write persist proves the
        // projection and guard release are now correctly tied together.
        // Pre-audit this same code compiled but the byte-slice guard
        // had already been dropped when `segment_mut` returned — any
        // overlapping borrow tracking was racing against stale state.
        let (_backing, account) = make_account(HEADER_LEN + 8, 1);
        let program_id = NativeAddress::new_from_array([9; 32]);
        let hopper_program_id =
            unsafe { core::mem::transmute::<NativeAddress, Address>(program_id) };
        let accounts = [account];
        let mut frame = new_frame(&hopper_program_id, &accounts);

        {
            let mut counter: RefMut<'_, Counter> =
                frame.segment_mut::<Counter>(0, 0).unwrap();
            counter.value = 7;
            // counter (and its held byte-slice RefMut) drops here.
        }

        // Reopen the account through the account-view path; the
        // segment registry already recorded the write for the whole
        // instruction, so we confirm persistence by rereading the raw
        // bytes via the underlying account view.
        let bytes = frame.account(0).unwrap().data().unwrap();
        let slice: &[u8] = &*bytes;
        let raw_u64 = unsafe {
            core::ptr::read_unaligned(slice.as_ptr().add(HEADER_LEN) as *const u64)
        };
        assert_eq!(raw_u64, 7);
    }

    #[test]
    fn frame_segment_ref_returns_live_guard() {
        // Seed the counter via direct byte access, then verify a
        // `segment_ref` returned guard lets us read that value. The
        // crucial property this exercises: `Ref<'_, Counter>` deref
        // into `Counter` after `segment_ref` returns, which pre-audit
        // would have been reading through a dropped byte-slice guard.
        let (_backing, account) = make_account(HEADER_LEN + 8, 2);
        {
            let mut bytes = account.try_borrow_mut().unwrap();
            let slot = unsafe {
                bytes.as_bytes_mut_ptr().add(HEADER_LEN) as *mut u64
            };
            unsafe { core::ptr::write_unaligned(slot, 99) };
        }
        let program_id = NativeAddress::new_from_array([9; 32]);
        let hopper_program_id =
            unsafe { core::mem::transmute::<NativeAddress, Address>(program_id) };
        let accounts = [account];
        let mut frame = new_frame(&hopper_program_id, &accounts);

        let reader: Ref<'_, Counter> = frame.segment_ref::<Counter>(0, 0).unwrap();
        assert_eq!(reader.value, 99);
    }
}
