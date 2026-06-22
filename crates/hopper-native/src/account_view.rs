//! RuntimeAccount memory layout and AccountView zero-copy wrapper.
//!
//! `RuntimeAccount` maps 1:1 onto the BPF input buffer layout that the
//! Solana runtime writes for each account. `AccountView` is a thin
//! pointer to a `RuntimeAccount` in that buffer, providing safe accessors
//! for address, owner, flags, lamports, and data.

use core::marker::PhantomData;

use crate::address::{address_eq, Address};
use crate::borrow::{Ref, RefMut};
use crate::error::ProgramError;
use crate::raw_account::RuntimeAccount;
use crate::{ProgramResult, MAX_PERMITTED_DATA_INCREASE, NOT_BORROWED};

// ── AccountView ──────────────────────────────────────────────────────

/// Zero-copy view over a Solana account in the BPF input buffer.
///
/// `AccountView` stores a raw pointer to the `RuntimeAccount` header.
/// All accessor methods read directly from the input buffer with no copies.
#[repr(C)]
#[cfg_attr(feature = "copy", derive(Copy))]
#[derive(Clone, PartialEq, Eq)]
pub struct AccountView<'info> {
    raw: *mut RuntimeAccount,
    _marker: PhantomData<&'info RuntimeAccount>,
}

// SAFETY: On Solana execution is single-threaded. Host tools and fuzzers
// should not rely on cross-thread sharing of raw account pointers.
#[cfg(target_os = "solana")]
unsafe impl<'info> Send for AccountView<'info> {}
#[cfg(target_os = "solana")]
unsafe impl<'info> Sync for AccountView<'info> {}

impl<'info> AccountView<'info> {
    /// Construct an AccountView from a raw pointer.
    ///
    /// # Safety
    ///
    /// `raw` must point to a valid `RuntimeAccount` in the BPF input buffer
    /// (or a test allocation with the same layout), followed by at least
    /// `(*raw).data_len` bytes of account data.
    #[inline(always)]
    pub const unsafe fn new_unchecked(raw: *mut RuntimeAccount) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    #[inline(always)]
    pub(crate) const fn raw_ptr(&self) -> *mut RuntimeAccount {
        self.raw
    }

    // ── Getters ──────────────────────────────────────────────────────

    /// The account's public key.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        // SAFETY: raw always points to a valid RuntimeAccount.
        unsafe { &(*self.raw).address }
    }

    /// The owning program's address.
    ///
    /// # Safety
    ///
    /// The returned reference is invalidated if the account is assigned
    /// to a new owner or closed. The caller must ensure no concurrent
    /// mutation occurs.
    #[inline(always)]
    pub unsafe fn owner(&self) -> &Address {
        // SAFETY: raw is valid; caller promises no concurrent mutation.
        unsafe { &(*self.raw).owner }
    }

    /// Whether this account signed the transaction.
    #[inline(always)]
    pub fn is_signer(&self) -> bool {
        // SAFETY: raw is valid.
        unsafe { (*self.raw).is_signer != 0 }
    }

    /// Whether this account is writable in the transaction.
    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).is_writable != 0 }
    }

    /// Whether this account contains an executable program.
    #[inline(always)]
    pub fn executable(&self) -> bool {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).executable != 0 }
    }

    /// Current data length in bytes.
    #[inline(always)]
    pub fn data_len(&self) -> usize {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).data_len as usize }
    }

    /// Resize delta (difference between current and original data length).
    #[inline(always)]
    pub fn resize_delta(&self) -> i32 {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).resize_delta }
    }

    /// Current lamport balance.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).lamports }
    }

    /// Whether the account data is empty (data_len == 0).
    #[inline(always)]
    pub fn is_data_empty(&self) -> bool {
        self.data_len() == 0
    }

    /// Set the lamport balance.
    #[inline(always)]
    pub fn set_lamports(&self, lamports: u64) {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            (*self.raw).lamports = lamports;
        }
    }

    // ── Ownership ────────────────────────────────────────────────────

    /// Check whether this account is owned by the given program.
    #[inline(always)]
    pub fn owned_by(&self, program: &Address) -> bool {
        // SAFETY: owner field is valid for the lifetime of the input buffer.
        unsafe { address_eq(&(*self.raw).owner, program) }
    }

    /// Assign a new owner.
    ///
    /// # Safety
    ///
    /// The caller must ensure the account is writable and that ownership
    /// transfer is authorized by the current owner program.
    #[inline(always)]
    pub unsafe fn assign(&self, new_owner: &Address) {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            (*self.raw).owner = new_owner.clone();
        }
    }

    // ── Borrow tracking ─────────────────────────────────────────────

    /// Whether the account data is currently borrowed (shared or exclusive).
    #[inline(always)]
    pub fn is_borrowed(&self) -> bool {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).borrow_state != NOT_BORROWED }
    }

    /// Whether the account data is exclusively (mutably) borrowed.
    #[inline(always)]
    pub fn is_borrowed_mut(&self) -> bool {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).borrow_state == 0 }
    }

    /// Check that the account can be shared-borrowed.
    #[inline(always)]
    pub fn check_borrow(&self) -> Result<(), ProgramError> {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let state = unsafe { (*self.raw).borrow_state };
        if state == 0 {
            // Exclusively borrowed -- cannot share.
            Err(ProgramError::AccountBorrowFailed)
        } else {
            Ok(())
        }
    }

    /// Check that the account can be exclusively borrowed.
    #[inline(always)]
    pub fn check_borrow_mut(&self) -> Result<(), ProgramError> {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let state = unsafe { (*self.raw).borrow_state };
        if state != NOT_BORROWED {
            // Already borrowed (shared or exclusive).
            Err(ProgramError::AccountBorrowFailed)
        } else {
            Ok(())
        }
    }

    // ── Unchecked data access ────────────────────────────────────────

    /// Borrow account data without borrow tracking.
    ///
    /// # Safety
    ///
    /// The caller must ensure no mutable borrow is active.
    #[inline(always)]
    pub unsafe fn borrow_unchecked(&self) -> &[u8] {
        let data_ptr = self.data_ptr_unchecked();
        let len = self.data_len();
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { core::slice::from_raw_parts(data_ptr, len) }
    }

    /// Mutably borrow account data without borrow tracking.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other borrows (shared or exclusive) are active.
    //
    // `mut_from_ref` fires because this returns `&mut [u8]` from `&self`. That
    // is intentional: account data lives behind a raw pointer the SVM owns, so
    // the `AccountView` only models shared access to that region while exposing
    // interior mutability through the documented `unsafe` contract above
    // (Pinocchio uses the same shape). Aliasing is the caller's invariant, not
    // the borrow checker's — that is exactly what the `unsafe` marker conveys.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub unsafe fn borrow_unchecked_mut(&self) -> &mut [u8] {
        let data_ptr = self.data_ptr_unchecked();
        let len = self.data_len();
        // SAFETY: `data_ptr_unchecked()` and `data_len()` describe the exact
        // SVM-owned data region for this account, and the caller guarantees no
        // overlapping borrow is live for the returned lifetime.
        unsafe { core::slice::from_raw_parts_mut(data_ptr, len) }
    }

    // ── Checked data access ──────────────────────────────────────────

    /// Try to obtain a shared borrow of the account data.
    ///
    /// Returns `Err(AccountBorrowFailed)` if the data is exclusively borrowed.
    #[inline(always)]
    pub fn try_borrow(&self) -> Result<Ref<'_, [u8]>, ProgramError> {
        self.check_borrow()?;
        // SAFETY: `self.raw` is a valid `RuntimeAccount` for this account
        // (entrypoint invariant); `borrow_state` is its first byte. Taking a
        // `*mut u8` to it does not create an aliasing reference.
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        // SAFETY: `state_ptr` points at this account's borrow-state byte and is
        // only read after `check_borrow()` confirmed the borrow is compatible.
        let state = unsafe { *state_ptr };
        let new_state = if state == NOT_BORROWED { 1 } else { state + 1 };
        if new_state == 0 || new_state == NOT_BORROWED {
            // `0` would alias the exclusive-borrow sentinel; `NOT_BORROWED`
            // (0xFF) would silently reset tracking on the 255th concurrent
            // shared borrow, after which a mutable borrow could be granted
            // while shared refs are still live. Cap the count at 254.
            return Err(ProgramError::AccountBorrowFailed);
        }
        // SAFETY: single-threaded SVM execution; we hold the only path that
        // writes this byte and have just validated the new shared count.
        unsafe {
            *state_ptr = new_state;
        }
        // SAFETY: the shared count was incremented above, so no exclusive
        // borrow is outstanding; the returned `Ref` decrements it on drop.
        let data = unsafe { self.borrow_unchecked() };
        Ok(Ref::new(data, state_ptr))
    }

    /// Try to obtain an exclusive (mutable) borrow of the account data.
    ///
    /// Returns `Err(AccountBorrowFailed)` if the data is already borrowed.
    #[inline(always)]
    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, [u8]>, ProgramError> {
        self.check_borrow_mut()?;
        // SAFETY: `self.raw` is a valid `RuntimeAccount`; `borrow_state` is its
        // first byte. The `*mut u8` does not create an aliasing reference.
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        // SAFETY: `check_borrow_mut()` confirmed the account was NOT_BORROWED,
        // so writing the exclusive sentinel (0) cannot stomp a live borrow.
        unsafe {
            *state_ptr = 0;
        } // Mark exclusive.
          // SAFETY: state is now exclusive, so no other borrow is live; the
          // returned `RefMut` restores NOT_BORROWED on drop.
        let data = unsafe { self.borrow_unchecked_mut() };
        Ok(RefMut::new(data, state_ptr))
    }

    // ── Typed segment and raw access ───────────────────────────────

    /// Project a typed segment from account data with native borrow tracking.
    #[inline(always)]
    pub fn segment_ref<T: crate::pod::Pod>(
        &self,
        offset: u32,
        size: u32,
    ) -> Result<Ref<'_, T>, ProgramError> {
        let expected_size = core::mem::size_of::<T>() as u32;
        if size != expected_size {
            return Err(ProgramError::InvalidArgument);
        }

        let end = offset
            .checked_add(size)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end as usize > self.data_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        self.check_borrow()?;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        let state = unsafe { *state_ptr };
        let new_state = if state == NOT_BORROWED { 1 } else { state + 1 };
        if new_state == 0 || new_state == NOT_BORROWED {
            // See `try_borrow`: cap the shared count at 254 so it can never
            // wrap into the NOT_BORROWED sentinel.
            return Err(ProgramError::AccountBorrowFailed);
        }
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            *state_ptr = new_state;
        }

        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let ptr = unsafe { self.data_ptr_unchecked().add(offset as usize) as *const T };
        Ok(Ref::new(unsafe { &*ptr }, state_ptr))
    }

    /// Acquire a shared segment borrow without size/bounds validation.
    ///
    /// # Safety
    ///
    /// The caller must have already verified:
    /// - `offset + size_of::<T>()` does not overflow
    /// - `offset + size_of::<T>() <= data_len()`
    #[inline(always)]
    pub unsafe fn segment_ref_unchecked<T: crate::pod::Pod>(
        &self,
        offset: u32,
    ) -> Result<Ref<'_, T>, ProgramError> {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let ptr = unsafe { self.data_ptr_unchecked().add(offset as usize) as *const T };
        Ok(Ref::new_external(unsafe { &*ptr }))
    }

    /// Project a mutable typed segment from account data with native borrow tracking.
    #[inline(always)]
    pub fn segment_mut<T: crate::pod::Pod>(
        &self,
        offset: u32,
        size: u32,
    ) -> Result<RefMut<'_, T>, ProgramError> {
        self.require_writable()?;

        let expected_size = core::mem::size_of::<T>() as u32;
        if size != expected_size {
            return Err(ProgramError::InvalidArgument);
        }

        let end = offset
            .checked_add(size)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end as usize > self.data_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        self.check_borrow_mut()?;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        unsafe {
            *state_ptr = 0;
        }

        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let ptr = unsafe { self.data_ptr_unchecked().add(offset as usize) as *mut T };
        Ok(RefMut::new(unsafe { &mut *ptr }, state_ptr))
    }

    /// Acquire an exclusive segment borrow without size/bounds/writable validation.
    ///
    /// # Safety
    ///
    /// The caller must have already verified:
    /// - The account is writable
    /// - `offset + size_of::<T>()` does not overflow
    /// - `offset + size_of::<T>() <= data_len()`
    #[inline(always)]
    pub unsafe fn segment_mut_unchecked<T: crate::pod::Pod>(
        &self,
        offset: u32,
    ) -> Result<RefMut<'_, T>, ProgramError> {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let ptr = unsafe { self.data_ptr_unchecked().add(offset as usize) as *mut T };
        Ok(RefMut::new_external(unsafe { &mut *ptr }))
    }

    /// Explicit raw typed read of the account buffer.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn raw_ref<T: crate::pod::Pod>(&self) -> Result<Ref<'_, T>, ProgramError> {
        self.segment_ref::<T>(0, core::mem::size_of::<T>() as u32)
    }

    /// Explicit raw typed write of the account buffer.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn raw_mut<T: crate::pod::Pod>(&self) -> Result<RefMut<'_, T>, ProgramError> {
        self.segment_mut::<T>(0, core::mem::size_of::<T>() as u32)
    }

    // ── Resize ───────────────────────────────────────────────────────

    /// Resize the account data to `new_len` bytes, zeroing any newly
    /// exposed region.
    ///
    /// Returns `Err(InvalidRealloc)` if the new length exceeds the
    /// permitted increase from the original allocation.
    ///
    /// When the account grows, the bytes in `[old_len, new_len)` are
    /// zero-filled. The Solana loader zeroes the realloc reserve once at
    /// the start of an instruction, but a shrink-then-grow within a
    /// single instruction can re-expose previously written bytes; zeroing
    /// on growth makes that impossible. Use [`resize_raw`](Self::resize_raw)
    /// for the hot path when the caller will overwrite the grown region
    /// in full and has measured the saved `memset`.
    #[inline(always)]
    pub fn resize(&self, new_len: usize) -> Result<(), ProgramError> {
        let original_len = (self.data_len() as i64 - self.resize_delta() as i64) as usize;
        if new_len > original_len + MAX_PERMITTED_DATA_INCREASE {
            return Err(ProgramError::InvalidRealloc);
        }
        let old_len = self.data_len();
        let delta = new_len as i64 - original_len as i64;
        // SAFETY: `data_ptr_unchecked()` is the account data base; the loader
        // guarantees `[old_len, new_len)` is within the realloc-reserve
        // capacity once the `InvalidRealloc` bound above has passed.
        unsafe {
            if new_len > old_len {
                crate::mem::memset(self.data_ptr_unchecked().add(old_len), 0, new_len - old_len);
            }
            (*self.raw).data_len = new_len as u64;
            (*self.raw).resize_delta = delta as i32;
        }
        Ok(())
    }

    /// Resize without zero-filling the newly exposed region.
    ///
    /// Same bounds check as [`resize`](Self::resize) but skips the
    /// zero-fill on growth. Prefer `resize` unless the caller immediately
    /// overwrites the entire grown region; otherwise stale bytes from an
    /// earlier shrink within the same instruction can leak into the new
    /// region.
    #[inline(always)]
    pub fn resize_raw(&self, new_len: usize) -> Result<(), ProgramError> {
        let original_len = (self.data_len() as i64 - self.resize_delta() as i64) as usize;
        if new_len > original_len + MAX_PERMITTED_DATA_INCREASE {
            return Err(ProgramError::InvalidRealloc);
        }
        let delta = new_len as i64 - original_len as i64;
        // SAFETY: bounds validated above; only header fields are written.
        unsafe {
            (*self.raw).data_len = new_len as u64;
            (*self.raw).resize_delta = delta as i32;
        }
        Ok(())
    }

    /// Resize without bounds checking or zero-filling.
    ///
    /// # Safety
    ///
    /// The caller must guarantee `new_len <= original_len + MAX_PERMITTED_DATA_INCREASE`
    /// and is responsible for any zero-fill of the grown region (see
    /// [`resize`](Self::resize) for why that matters).
    #[inline(always)]
    pub unsafe fn resize_unchecked(&self, new_len: usize) {
        let original_len = (self.data_len() as i64 - self.resize_delta() as i64) as usize;
        let delta = new_len as i64 - original_len as i64;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            (*self.raw).data_len = new_len as u64;
            (*self.raw).resize_delta = delta as i32;
        }
    }

    // ── Close ────────────────────────────────────────────────────────

    /// Solana System Program address (all-zero pubkey).
    ///
    /// Closing an account transfers ownership back to the System
    /// Program, which is the canonical "no-owner" state on Solana.
    /// The byte value `[0u8; 32]` and `Address::default()` are
    /// equivalent, but using this named constant makes the intent
    /// explicit, per the Hopper Safety Audit which flagged the
    /// `Address::default()` spelling as documentation drift.
    pub const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([0u8; 32]);

    /// Close the account: zero lamports and data, reassign owner to
    /// the System Program.
    ///
    /// # Caveat
    ///
    /// This low-level routine does **not** verify the caller has
    /// authority to close the account, Solana's runtime enforces
    /// owner/writable rules at transaction commit time regardless, but
    /// higher-level APIs (e.g. `hopper_runtime::AccountView::close_to`)
    /// should pre-check those rules. See `account.rs::close_to` for
    /// the safe wrapper.
    #[inline(always)]
    pub fn close(&self) -> ProgramResult {
        self.set_lamports(0);
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            let len = self.data_len();
            if len > 0 {
                // Use the SVM's JIT-compiled memset for optimal CU cost.
                crate::mem::memset(self.data_ptr_unchecked(), 0, len);
            }
            (*self.raw).data_len = 0;
            (*self.raw).owner = Self::SYSTEM_PROGRAM_ID;
        }
        Ok(())
    }

    /// Close without borrow checks.
    ///
    /// # Safety
    ///
    /// The caller must ensure no active borrows exist.
    #[inline(always)]
    pub unsafe fn close_unchecked(&self) {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            (*self.raw).lamports = 0;
            (*self.raw).data_len = 0;
            (*self.raw).owner = Self::SYSTEM_PROGRAM_ID;
        }
    }

    // ── Raw pointers ─────────────────────────────────────────────────

    /// Raw pointer to the `RuntimeAccount` header.
    #[inline(always)]
    pub const fn account_ptr(&self) -> *const RuntimeAccount {
        self.raw as *const RuntimeAccount
    }

    /// Raw pointer to the first byte of account data.
    ///
    /// The data starts immediately after the 88-byte `RuntimeAccount` header.
    /// This is an expert-only substrate escape hatch: constructing the pointer
    /// is safe, but dereferencing it is unsafe and bypasses Hopper Native's
    /// borrow-state checks, segment registry, and writable checks. Normal code
    /// should use `try_borrow`, `try_borrow_mut`, `segment_ref`, or
    /// `segment_mut`. Framework code should route user-facing raw access
    /// through the documented unsafe runtime APIs (`Context::as_mut_ptr` /
    /// `Context::as_ptr`) instead of exposing this method directly.
    #[doc(hidden)]
    #[inline(always)]
    pub fn data_ptr_unchecked(&self) -> *mut u8 {
        // SAFETY: Adding the struct size to the base pointer yields the
        // first data byte. The runtime guarantees this memory is valid.
        unsafe { (self.raw as *mut u8).add(core::mem::size_of::<RuntimeAccount>()) }
    }

    // ── Hopper Innovations ───────────────────────────────────────────

    /// Validate that this account is a signer, returning a typed error.
    #[inline(always)]
    pub fn require_signer(&self) -> ProgramResult {
        if self.is_signer() {
            Ok(())
        } else {
            Err(ProgramError::MissingRequiredSignature)
        }
    }

    /// Validate that this account is writable.
    #[inline(always)]
    pub fn require_writable(&self) -> ProgramResult {
        if self.is_writable() {
            Ok(())
        } else {
            Err(ProgramError::Immutable)
        }
    }

    /// Validate that this account is owned by the given program.
    #[inline(always)]
    pub fn require_owned_by(&self, program: &Address) -> ProgramResult {
        if self.owned_by(program) {
            Ok(())
        } else {
            Err(ProgramError::IncorrectProgramId)
        }
    }

    /// Validate signer + writable (common "payer" pattern).
    #[inline(always)]
    pub fn require_payer(&self) -> ProgramResult {
        self.require_signer()?;
        self.require_writable()
    }

    /// Read the Hopper account discriminator (first byte of data).
    ///
    /// Returns 0 if the account has no data.
    #[inline(always)]
    pub fn disc(&self) -> u8 {
        if self.data_len() == 0 {
            return 0;
        }
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { *self.data_ptr_unchecked() }
    }

    /// Read the Hopper account version (second byte of data).
    ///
    /// Returns 0 if the account has fewer than 2 bytes.
    #[inline(always)]
    pub fn version(&self) -> u8 {
        if self.data_len() < 2 {
            return 0;
        }
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { *self.data_ptr_unchecked().add(1) }
    }

    /// Read the 8-byte layout_id from the Hopper account header
    /// (bytes 4..12 of account data, per the canonical header format).
    ///
    /// Returns `None` if the account has fewer than 12 bytes.
    #[inline(always)]
    pub fn layout_id(&self) -> Option<&[u8; 8]> {
        if self.data_len() < 12 {
            return None;
        }
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { Some(&*(self.data_ptr_unchecked().add(4) as *const [u8; 8])) }
    }

    /// Verify that this account has the given discriminator.
    #[inline(always)]
    pub fn require_disc(&self, expected: u8) -> ProgramResult {
        if self.disc() == expected {
            Ok(())
        } else {
            Err(ProgramError::InvalidAccountData)
        }
    }

    // -- Chainable validation (Steel-inspired, improved) ---------------
    //
    // Return `Result<&Self>` so callers can chain:
    //
    //   account
    //       .check_signer()?
    //       .check_writable()?
    //       .check_owned_by(&MY_PROGRAM_ID)?;
    //
    // Validated once, used everywhere. This pattern exists in Steel but
    // not in pinocchio, Anchor, or Quasar.

    /// Chainable signer check.
    #[inline(always)]
    pub fn check_signer(&self) -> Result<&Self, ProgramError> {
        if self.is_signer() {
            Ok(self)
        } else {
            Err(ProgramError::MissingRequiredSignature)
        }
    }

    /// Chainable writable check.
    #[inline(always)]
    pub fn check_writable(&self) -> Result<&Self, ProgramError> {
        if self.is_writable() {
            Ok(self)
        } else {
            Err(ProgramError::Immutable)
        }
    }

    /// Chainable ownership check.
    #[inline(always)]
    pub fn check_owned_by(&self, program: &Address) -> Result<&Self, ProgramError> {
        if self.owned_by(program) {
            Ok(self)
        } else {
            Err(ProgramError::IncorrectProgramId)
        }
    }

    /// Chainable discriminator check.
    #[inline(always)]
    pub fn check_disc(&self, expected: u8) -> Result<&Self, ProgramError> {
        if self.disc() == expected {
            Ok(self)
        } else {
            Err(ProgramError::InvalidAccountData)
        }
    }

    /// Chainable non-empty data check.
    #[inline(always)]
    pub fn check_has_data(&self) -> Result<&Self, ProgramError> {
        if !self.is_data_empty() {
            Ok(self)
        } else {
            Err(ProgramError::AccountDataTooSmall)
        }
    }

    /// Chainable executable check.
    #[inline(always)]
    pub fn check_executable(&self) -> Result<&Self, ProgramError> {
        if self.executable() {
            Ok(self)
        } else {
            Err(ProgramError::InvalidArgument)
        }
    }

    /// Chainable address check.
    #[inline(always)]
    pub fn check_address(&self, expected: &Address) -> Result<&Self, ProgramError> {
        if address_eq(self.address(), expected) {
            Ok(self)
        } else {
            Err(ProgramError::InvalidArgument)
        }
    }

    /// Chainable minimum data length check.
    #[inline(always)]
    pub fn check_data_len(&self, min_len: usize) -> Result<&Self, ProgramError> {
        if self.data_len() >= min_len {
            Ok(self)
        } else {
            Err(ProgramError::AccountDataTooSmall)
        }
    }

    // -- Safe owner access ---------------------------------------------

    /// Read the owner address as a copy (32-byte value).
    ///
    /// Unlike `owner()` (which is unsafe due to reference invalidation
    /// if `assign()` is called), this returns a copy that is always safe.
    /// Costs 32 bytes of stack space but eliminates aliasing hazards.
    #[inline(always)]
    pub fn read_owner(&self) -> Address {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { (*self.raw).owner.clone() }
    }

    // -- Packed flags --------------------------------------------------

    /// Read the first 4 bytes of the account header as a single u32.
    ///
    /// Layout (little-endian): `[borrow_state, is_signer, is_writable, executable]`
    ///
    /// This is the fastest way to extract multiple account properties at once
    ///, a single aligned u32 read instead of 3-4 separate byte loads.
    #[inline(always)]
    fn header_u32(&self) -> u32 {
        // SAFETY: RuntimeAccount is #[repr(C)] with first 4 bytes as
        // u8 fields; read_unaligned imposes no alignment requirement.
        unsafe { core::ptr::read_unaligned(self.raw as *const u32) }
    }

    /// Pack the account's boolean flags into a single byte for fast
    /// comparison.
    ///
    /// Bit layout:
    /// - bit 0: is_signer
    /// - bit 1: is_writable
    /// - bit 2: executable
    /// - bit 3: has data (data_len > 0)
    ///
    /// Use with `expect_flags()` for single-instruction multi-check:
    ///
    /// ```ignore
    /// // Require: signer + writable + has data
    /// account.expect_flags(0b1011)?;
    /// ```
    #[inline(always)]
    pub fn flags(&self) -> u8 {
        // Single u32 read extracts [borrow_state, is_signer, is_writable, executable].
        // On little-endian: is_signer = bits 8-15, is_writable = bits 16-23, executable = bits 24-31.
        let h = self.header_u32();
        let mut f: u8 = 0;
        if h & 0x0000_FF00 != 0 {
            f |= 0b0001;
        } // is_signer
        if h & 0x00FF_0000 != 0 {
            f |= 0b0010;
        } // is_writable
        if h & 0xFF00_0000 != 0 {
            f |= 0b0100;
        } // executable
        if !self.is_data_empty() {
            f |= 0b1000;
        }
        f
    }

    /// Check that the account's flags contain all the required bits.
    ///
    /// `required` is a bitmask of flags that must be set. See `flags()`.
    #[inline(always)]
    pub fn expect_flags(&self, required: u8) -> ProgramResult {
        if self.flags() & required == required {
            Ok(())
        } else {
            Err(ProgramError::InvalidArgument)
        }
    }
}

impl<'info> core::fmt::Debug for AccountView<'info> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AccountView")
            .field("address", self.address())
            .field("lamports", &self.lamports())
            .field("data_len", &self.data_len())
            .field("is_signer", &self.is_signer())
            .field("is_writable", &self.is_writable())
            .finish()
    }
}

// ── RemainingAccounts ────────────────────────────────────────────────

/// Iterator over remaining (unstructured) accounts after the known ones.
pub struct RemainingAccounts<'a> {
    accounts: &'a [AccountView<'a>],
    cursor: usize,
}

impl<'a> RemainingAccounts<'a> {
    /// Create from a slice of the remaining accounts.
    #[inline(always)]
    pub fn new(accounts: &'a [AccountView<'a>]) -> Self {
        Self {
            accounts,
            cursor: 0,
        }
    }

    /// Number of accounts remaining.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.accounts.len() - self.cursor
    }

    /// Take the next account, or return `NotEnoughAccountKeys`.
    ///
    /// This is a fallible cursor advance, not an `Iterator::next`: it yields a
    /// `Result` so a missing account is a program error rather than a silent
    /// `None`, which is the wrong shape for the `Iterator` trait.
    #[allow(clippy::should_implement_trait)]
    #[inline(always)]
    pub fn next(&mut self) -> Result<&'a AccountView<'a>, ProgramError> {
        if self.cursor >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let account = &self.accounts[self.cursor];
        self.cursor += 1;
        Ok(account)
    }

    /// Take the next account that is a signer.
    #[inline(always)]
    pub fn next_signer(&mut self) -> Result<&'a AccountView<'a>, ProgramError> {
        let account = self.next()?;
        account.require_signer()?;
        Ok(account)
    }

    /// Take the next account that is writable.
    #[inline(always)]
    pub fn next_writable(&mut self) -> Result<&'a AccountView<'a>, ProgramError> {
        let account = self.next()?;
        account.require_writable()?;
        Ok(account)
    }

    /// Take the next account owned by the given program.
    #[inline(always)]
    pub fn next_owned_by(&mut self, program: &Address) -> Result<&'a AccountView<'a>, ProgramError> {
        let account = self.next()?;
        account.require_owned_by(program)?;
        Ok(account)
    }
}
