//! RuntimeAccount memory layout and AccountView zero-copy wrapper.
//!
//! `RuntimeAccount` maps 1:1 onto the BPF input buffer layout that the
//! Solana runtime writes for each account. `AccountView` is a thin
//! pointer to a `RuntimeAccount` in that buffer, providing safe accessors
//! for address, owner, flags, lamports, and data.

use crate::address::{Address, address_eq};
use crate::borrow::{Ref, RefMut};
use crate::error::ProgramError;
use crate::raw_account::RuntimeAccount;
use crate::{MAX_PERMITTED_DATA_INCREASE, NOT_BORROWED, ProgramResult};

// ── AccountView ──────────────────────────────────────────────────────

/// Zero-copy view over a Solana account in the BPF input buffer.
///
/// `AccountView` stores a raw pointer to the `RuntimeAccount` header.
/// All accessor methods read directly from the input buffer with no copies.
#[repr(C)]
#[cfg_attr(feature = "copy", derive(Copy))]
#[derive(Clone, PartialEq, Eq)]
pub struct AccountView {
    raw: *mut RuntimeAccount,
}

// SAFETY: AccountView is safe to send between threads in test contexts.
// On BPF there is only one thread.
unsafe impl Send for AccountView {}
unsafe impl Sync for AccountView {}

impl AccountView {
    /// Construct an AccountView from a raw pointer.
    ///
    /// # Safety
    ///
    /// `raw` must point to a valid `RuntimeAccount` in the BPF input buffer
    /// (or a test allocation with the same layout), followed by at least
    /// `(*raw).data_len` bytes of account data.
    #[inline(always)]
    pub const unsafe fn new_unchecked(raw: *mut RuntimeAccount) -> Self {
        Self { raw }
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
        unsafe { (*self.raw).is_writable != 0 }
    }

    /// Whether this account contains an executable program.
    #[inline(always)]
    pub fn executable(&self) -> bool {
        unsafe { (*self.raw).executable != 0 }
    }

    /// Current data length in bytes.
    #[inline(always)]
    pub fn data_len(&self) -> usize {
        unsafe { (*self.raw).data_len as usize }
    }

    /// Resize delta (difference between current and original data length).
    #[inline(always)]
    pub fn resize_delta(&self) -> i32 {
        unsafe { (*self.raw).resize_delta }
    }

    /// Current lamport balance.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
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
        unsafe { (*self.raw).lamports = lamports; }
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
        unsafe { (*self.raw).owner = new_owner.clone(); }
    }

    // ── Borrow tracking ─────────────────────────────────────────────

    /// Whether the account data is currently borrowed (shared or exclusive).
    #[inline(always)]
    pub fn is_borrowed(&self) -> bool {
        unsafe { (*self.raw).borrow_state != NOT_BORROWED }
    }

    /// Whether the account data is exclusively (mutably) borrowed.
    #[inline(always)]
    pub fn is_borrowed_mut(&self) -> bool {
        unsafe { (*self.raw).borrow_state == 0 }
    }

    /// Check that the account can be shared-borrowed.
    #[inline(always)]
    pub fn check_borrow(&self) -> Result<(), ProgramError> {
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
        let data_ptr = self.data_ptr();
        let len = self.data_len();
        unsafe { core::slice::from_raw_parts(data_ptr, len) }
    }

    /// Mutably borrow account data without borrow tracking.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other borrows (shared or exclusive) are active.
    #[inline(always)]
    pub unsafe fn borrow_unchecked_mut(&self) -> &mut [u8] {
        let data_ptr = self.data_ptr();
        let len = self.data_len();
        unsafe { core::slice::from_raw_parts_mut(data_ptr, len) }
    }

    // ── Checked data access ──────────────────────────────────────────

    /// Try to obtain a shared borrow of the account data.
    ///
    /// Returns `Err(AccountBorrowFailed)` if the data is exclusively borrowed.
    #[inline(always)]
    pub fn try_borrow(&self) -> Result<Ref<'_, [u8]>, ProgramError> {
        self.check_borrow()?;
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        let state = unsafe { *state_ptr };
        let new_state = if state == NOT_BORROWED { 1 } else { state + 1 };
        if new_state == 0 {
            // Overflow into exclusive-borrow sentinel.
            return Err(ProgramError::AccountBorrowFailed);
        }
        unsafe { *state_ptr = new_state; }
        let data = unsafe { self.borrow_unchecked() };
        Ok(Ref::new(data, state_ptr))
    }

    /// Try to obtain an exclusive (mutable) borrow of the account data.
    ///
    /// Returns `Err(AccountBorrowFailed)` if the data is already borrowed.
    #[inline(always)]
    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, [u8]>, ProgramError> {
        self.check_borrow_mut()?;
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        unsafe { *state_ptr = 0; } // Mark exclusive.
        let data = unsafe { self.borrow_unchecked_mut() };
        Ok(RefMut::new(data, state_ptr))
    }

    // ── Typed segment and raw access ───────────────────────────────

    /// Project a typed segment from account data with native borrow tracking.
    #[inline(always)]
    pub fn segment_ref<T: Copy>(&self, offset: u32, size: u32) -> Result<Ref<'_, T>, ProgramError> {
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
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        let state = unsafe { *state_ptr };
        let new_state = if state == NOT_BORROWED { 1 } else { state + 1 };
        if new_state == 0 {
            return Err(ProgramError::AccountBorrowFailed);
        }
        unsafe { *state_ptr = new_state; }

        let ptr = unsafe { self.data_ptr().add(offset as usize) as *const T };
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
    pub unsafe fn segment_ref_unchecked<T: Copy>(&self, offset: u32) -> Result<Ref<'_, T>, ProgramError> {
        self.check_borrow()?;
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        let state = unsafe { *state_ptr };
        let new_state = if state == NOT_BORROWED { 1 } else { state + 1 };
        if new_state == 0 {
            return Err(ProgramError::AccountBorrowFailed);
        }
        unsafe { *state_ptr = new_state; }

        let ptr = unsafe { self.data_ptr().add(offset as usize) as *const T };
        Ok(Ref::new(unsafe { &*ptr }, state_ptr))
    }

    /// Project a mutable typed segment from account data with native borrow tracking.
    #[inline(always)]
    pub fn segment_mut<T: Copy>(&self, offset: u32, size: u32) -> Result<RefMut<'_, T>, ProgramError> {
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
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        unsafe { *state_ptr = 0; }

        let ptr = unsafe { self.data_ptr().add(offset as usize) as *mut T };
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
    pub unsafe fn segment_mut_unchecked<T: Copy>(&self, offset: u32) -> Result<RefMut<'_, T>, ProgramError> {
        self.check_borrow_mut()?;
        let state_ptr = unsafe { &mut (*self.raw).borrow_state as *mut u8 };
        unsafe { *state_ptr = 0; }

        let ptr = unsafe { self.data_ptr().add(offset as usize) as *mut T };
        Ok(RefMut::new(unsafe { &mut *ptr }, state_ptr))
    }

    /// Explicit raw typed read of the account buffer.
    #[inline(always)]
    pub unsafe fn raw_ref<T: Copy>(&self) -> Result<Ref<'_, T>, ProgramError> {
        self.segment_ref::<T>(0, core::mem::size_of::<T>() as u32)
    }

    /// Explicit raw typed write of the account buffer.
    #[inline(always)]
    pub unsafe fn raw_mut<T: Copy>(&self) -> Result<RefMut<'_, T>, ProgramError> {
        self.segment_mut::<T>(0, core::mem::size_of::<T>() as u32)
    }

    // ── Resize ───────────────────────────────────────────────────────

    /// Resize the account data to `new_len` bytes.
    ///
    /// Returns `Err(InvalidRealloc)` if the new length exceeds the
    /// permitted increase from the original allocation.
    #[inline(always)]
    pub fn resize(&self, new_len: usize) -> Result<(), ProgramError> {
        let original_len = (self.data_len() as i64 - self.resize_delta() as i64) as usize;
        if new_len > original_len + MAX_PERMITTED_DATA_INCREASE {
            return Err(ProgramError::InvalidRealloc);
        }
        let delta = new_len as i64 - original_len as i64;
        unsafe {
            (*self.raw).data_len = new_len as u64;
            (*self.raw).resize_delta = delta as i32;
        }
        Ok(())
    }

    /// Resize without bounds checking.
    ///
    /// # Safety
    ///
    /// The caller must guarantee `new_len <= original_len + MAX_PERMITTED_DATA_INCREASE`.
    #[inline(always)]
    pub unsafe fn resize_unchecked(&self, new_len: usize) {
        let original_len = (self.data_len() as i64 - self.resize_delta() as i64) as usize;
        let delta = new_len as i64 - original_len as i64;
        unsafe {
            (*self.raw).data_len = new_len as u64;
            (*self.raw).resize_delta = delta as i32;
        }
    }

    // ── Close ────────────────────────────────────────────────────────

    /// Close the account: zero lamports and data, set owner to system program.
    #[inline(always)]
    pub fn close(&self) -> ProgramResult {
        self.set_lamports(0);
        unsafe {
            let len = self.data_len();
            if len > 0 {
                // Use the SVM's JIT-compiled memset for optimal CU cost.
                crate::mem::memset(self.data_ptr(), 0, len);
            }
            (*self.raw).data_len = 0;
            (*self.raw).owner = Address::default();
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
        unsafe {
            (*self.raw).lamports = 0;
            (*self.raw).data_len = 0;
            (*self.raw).owner = Address::default();
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
    #[inline(always)]
    pub fn data_ptr(&self) -> *mut u8 {
        // SAFETY: Adding the struct size to the base pointer yields the
        // first data byte. The runtime guarantees this memory is valid.
        unsafe {
            (self.raw as *mut u8).add(core::mem::size_of::<RuntimeAccount>())
        }
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
        unsafe { *self.data_ptr() }
    }

    /// Read the Hopper account version (second byte of data).
    ///
    /// Returns 0 if the account has fewer than 2 bytes.
    #[inline(always)]
    pub fn version(&self) -> u8 {
        if self.data_len() < 2 {
            return 0;
        }
        unsafe { *self.data_ptr().add(1) }
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
        unsafe {
            Some(&*(self.data_ptr().add(4) as *const [u8; 8]))
        }
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
        unsafe { (*self.raw).owner.clone() }
    }

    // -- Packed flags --------------------------------------------------

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
        let mut f: u8 = 0;
        if self.is_signer() { f |= 0b0001; }
        if self.is_writable() { f |= 0b0010; }
        if self.executable() { f |= 0b0100; }
        if !self.is_data_empty() { f |= 0b1000; }
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

impl core::fmt::Debug for AccountView {
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
    accounts: &'a [AccountView],
    cursor: usize,
}

impl<'a> RemainingAccounts<'a> {
    /// Create from a slice of the remaining accounts.
    #[inline(always)]
    pub fn new(accounts: &'a [AccountView]) -> Self {
        Self { accounts, cursor: 0 }
    }

    /// Number of accounts remaining.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.accounts.len() - self.cursor
    }

    /// Take the next account, or return `NotEnoughAccountKeys`.
    #[inline]
    pub fn next(&mut self) -> Result<&'a AccountView, ProgramError> {
        if self.cursor >= self.accounts.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let account = &self.accounts[self.cursor];
        self.cursor += 1;
        Ok(account)
    }

    /// Take the next account that is a signer.
    #[inline]
    pub fn next_signer(&mut self) -> Result<&'a AccountView, ProgramError> {
        let account = self.next()?;
        account.require_signer()?;
        Ok(account)
    }

    /// Take the next account that is writable.
    #[inline]
    pub fn next_writable(&mut self) -> Result<&'a AccountView, ProgramError> {
        let account = self.next()?;
        account.require_writable()?;
        Ok(account)
    }

    /// Take the next account owned by the given program.
    #[inline]
    pub fn next_owned_by(&mut self, program: &Address) -> Result<&'a AccountView, ProgramError> {
        let account = self.next()?;
        account.require_owned_by(program)?;
        Ok(account)
    }
}
