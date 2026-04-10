//! Hopper-owned account view for Solana programs.
//!
//! `AccountView` is the canonical typed state gateway for Hopper programs.
//! It wraps the active backend's account representation behind a
//! `#[repr(transparent)]` boundary, delegating all methods with zero-cost
//! type conversion.
//!
//! Key capabilities:
//! - Chainable validation (`check_signer()?.check_writable()?`)
//! - Zero-copy overlay access (`overlay::<T>()`, `overlay_mut::<T>()`)
//! - Hopper header reading (disc, version, layout_id)
//! - Packed flags for batch validation
//! - Remaining accounts iterator

use crate::address::{Address, address_eq};
use crate::compat::{self, BackendAccountView};
use crate::error::ProgramError;
use crate::borrow::{Ref, RefMut};
use crate::layout::{HopperHeader, LayoutContract};
use crate::ProgramResult;

// ══════════════════════════════════════════════════════════════════════
//  AccountView -- Hopper's canonical typed state gateway
// ══════════════════════════════════════════════════════════════════════

/// Zero-copy view over a Solana account.
///
/// `AccountView` is the single canonical type for account access in
/// Hopper programs. It wraps whatever backend is active and exposes a
/// Hopper-owned API surface.
///
/// The `#[repr(transparent)]` layout guarantees that `&[backend::AccountView]`
/// can be safely reinterpreted as `&[AccountView]` at the entrypoint
/// boundary with zero conversion cost.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
pub struct AccountView {
    inner: BackendAccountView,
}

// SAFETY: AccountView is safe to send between threads (BPF is single-threaded;
// tests may need Send/Sync).
unsafe impl Send for AccountView {}
unsafe impl Sync for AccountView {}

impl AccountView {
    // ── Getters ──────────────────────────────────────────────────────

    /// The account's public key.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        compat::account_address(&self.inner)
    }

    /// The owning program's address.
    ///
    /// # Safety
    ///
    /// The returned reference is invalidated if the account is assigned
    /// to a new owner. The caller must ensure no concurrent mutation.
    #[inline(always)]
    pub unsafe fn owner(&self) -> &Address {
        unsafe { compat::account_owner(&self.inner) }
    }

    /// Read the owner address as a copy (safe, no aliasing hazard).
    #[inline(always)]
    pub fn read_owner(&self) -> Address {
        compat::read_owner(&self.inner)
    }

    /// Whether this account is owned by the given program.
    #[inline(always)]
    pub fn owned_by(&self, program: &Address) -> bool {
        compat::owned_by(&self.inner, program)
    }

    /// Whether this account signed the transaction.
    #[inline(always)]
    pub fn is_signer(&self) -> bool {
        self.inner.is_signer()
    }

    /// Whether this account is writable in the transaction.
    #[inline(always)]
    pub fn is_writable(&self) -> bool {
        self.inner.is_writable()
    }

    /// Whether this account contains an executable program.
    #[inline(always)]
    pub fn executable(&self) -> bool {
        self.inner.executable()
    }

    /// Current data length in bytes.
    #[inline(always)]
    pub fn data_len(&self) -> usize {
        self.inner.data_len()
    }

    /// Current lamport balance.
    #[inline(always)]
    pub fn lamports(&self) -> u64 {
        self.inner.lamports()
    }

    /// Whether the account data is empty.
    #[inline(always)]
    pub fn is_data_empty(&self) -> bool {
        self.data_len() == 0
    }

    /// Set the lamport balance.
    #[inline(always)]
    pub fn set_lamports(&self, lamports: u64) {
        self.inner.set_lamports(lamports);
    }

    // ── Borrow tracking ─────────────────────────────────────────────

    /// Try to obtain a shared borrow of the account data.
    #[inline]
    pub fn try_borrow(&self) -> Result<Ref<'_, [u8]>, ProgramError> {
        self.inner.try_borrow()
            .map(Ref::from_backend)
            .map_err(ProgramError::from)
    }

    /// Try to obtain an exclusive (mutable) borrow of the account data.
    #[inline]
    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, [u8]>, ProgramError> {
        self.inner.try_borrow_mut()
            .map(RefMut::from_backend)
            .map_err(ProgramError::from)
    }

    // ── Zero-copy overlay access ─────────────────────────────────────

    /// Interpret account data as a typed overlay (immutable).
    ///
    /// Performs a length check and returns a zero-cost pointer cast
    /// into the account's data buffer. No copies, no deserialization.
    ///
    /// # Safety model
    ///
    /// The caller must ensure `T` is a plain-old-data type where all
    /// bit patterns are valid (i.e. a `Pod` type). Alignment must be 1
    /// or the type must be `#[repr(C)]` over alignment-1 fields.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let state: &MyState = account.overlay::<MyState>()?;
    /// ```
    #[inline(always)]
    pub fn overlay<T: Copy>(&self) -> Result<&T, ProgramError> {
        let data = self.try_borrow()?;
        if data.len() < core::mem::size_of::<T>() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: T: Copy implies no drop glue. Caller guarantees Pod semantics.
        // Borrow guard keeps the data alive for the returned reference lifetime.
        Ok(unsafe { &*(data.as_ptr() as *const T) })
    }

    /// Interpret account data as a typed overlay (mutable).
    ///
    /// Same as `overlay()` but provides a mutable reference.
    /// Changes are written directly to the account's data buffer.
    #[inline(always)]
    pub fn overlay_mut<T: Copy>(&self) -> Result<&mut T, ProgramError> {
        let mut data = self.try_borrow_mut()?;
        if data.len() < core::mem::size_of::<T>() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: Exclusive borrow guard guarantees no aliasing.
        Ok(unsafe { &mut *(data.as_mut_ptr() as *mut T) })
    }

    /// Interpret account data at a specific offset as a typed overlay.
    ///
    /// Useful for reading past a header or into a specific region.
    #[inline(always)]
    pub fn overlay_at<T: Copy>(&self, offset: usize) -> Result<&T, ProgramError> {
        let data = self.try_borrow()?;
        let end = offset.checked_add(core::mem::size_of::<T>())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if data.len() < end {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(unsafe { &*(data.as_ptr().add(offset) as *const T) })
    }

    // ── Typed load (LayoutContract-aware) ────────────────────────────

    /// Load a typed layout after validating the account header.
    ///
    /// This is the canonical "validate then project" path:
    /// 1. Check disc, version, and layout_id match `T`
    /// 2. Verify data length >= `T::SIZE`
    /// 3. Return zero-copy reference into account data
    ///
    /// The returned reference points past the 16-byte header directly
    /// into the struct's fields.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let vault: &Vault = account.load::<Vault>()?;
    /// ```
    #[inline(always)]
    pub fn load<T: LayoutContract>(&self) -> Result<&T, ProgramError> {
        let data = self.try_borrow()?;
        T::validate_header(&data)?;
        let body = &data[HopperHeader::SIZE..];
        if body.len() < core::mem::size_of::<T>() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(unsafe { &*(body.as_ptr() as *const T) })
    }

    /// Load a mutable typed layout after validating the account header.
    ///
    /// Same as `load()` but provides a mutable reference for in-place
    /// state updates. Changes write directly to account data.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let vault: &mut Vault = account.load_mut::<Vault>()?;
    /// vault.balance = vault.balance.checked_add(amount)?;
    /// ```
    #[inline(always)]
    pub fn load_mut<T: LayoutContract>(&self) -> Result<&mut T, ProgramError> {
        let mut data = self.try_borrow_mut()?;
        T::validate_header(&data)?;
        let body = &mut data[HopperHeader::SIZE..];
        if body.len() < core::mem::size_of::<T>() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(unsafe { &mut *(body.as_mut_ptr() as *mut T) })
    }

    /// Load a typed layout checking only the discriminator (fast path).
    ///
    /// Skips version and layout_id checks. Use when you trust the account
    /// source and only need type dispatch.
    #[inline(always)]
    pub fn load_unchecked<T: LayoutContract>(&self) -> Result<&T, ProgramError> {
        let data = self.try_borrow()?;
        T::check_disc(&data)?;
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let body = &data[HopperHeader::SIZE..];
        Ok(unsafe { &*(body.as_ptr() as *const T) })
    }

    /// Load a layout with version compatibility checking.
    ///
    /// Like `load()` but uses `T::compatible(version)` instead of an exact
    /// version match. This allows loading older account versions that the
    /// current layout version still understands (e.g. forward-compatible
    /// append-only migrations).
    #[inline(always)]
    pub fn load_versioned<T: LayoutContract>(&self) -> Result<&T, ProgramError> {
        let data = self.try_borrow()?;
        T::check_disc(&data)?;
        let version = crate::layout::read_version(&data)
            .ok_or(ProgramError::AccountDataTooSmall)?;
        if !T::compatible(version) {
            return Err(ProgramError::InvalidAccountData);
        }
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let body = &data[HopperHeader::SIZE..];
        Ok(unsafe { &*(body.as_ptr() as *const T) })
    }

    /// Load a foreign layout without ownership or authorization checks.
    ///
    /// Only validates the wire format (disc + layout_id + size). Use
    /// this for cross-program reads where the account is owned by
    /// another program and you just need a typed view of its data.
    #[inline(always)]
    pub fn load_foreign<T: LayoutContract>(&self) -> Result<&T, ProgramError> {
        let data = self.try_borrow()?;
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        T::check_disc(&data)?;
        // Verify layout_id to confirm the wire format matches.
        if let Some(id) = crate::layout::read_layout_id(&data) {
            if *id != T::LAYOUT_ID {
                return Err(ProgramError::InvalidAccountData);
            }
        } else {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let body = &data[HopperHeader::SIZE..];
        Ok(unsafe { &*(body.as_ptr() as *const T) })
    }

    /// Read runtime layout metadata from this account's header.
    ///
    /// Returns `None` if the account data is too short for a Hopper header.
    /// This is useful for runtime inspection, manager tooling, and schema
    /// checking when the concrete layout type is not known at compile time.
    #[inline(always)]
    pub fn layout_info(&self) -> Option<crate::layout::LayoutInfo> {
        let data = self.try_borrow().ok()?;
        crate::layout::LayoutInfo::from_data(&data)
    }

    /// Return the extension-region byte range for a layout that declares one.
    ///
    /// Callers can apply the returned range to a borrowed data slice when they
    /// want to inspect or mutate extension bytes explicitly.
    #[inline(always)]
    pub fn extension_range<T: LayoutContract>(&self) -> Result<core::ops::Range<usize>, ProgramError> {
        let offset = T::EXTENSION_OFFSET.ok_or(ProgramError::InvalidArgument)?;
        let data_len = self.data_len();
        if data_len < offset {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(offset..data_len)
    }

    /// Initialize an account with the given layout contract header.
    ///
    /// Writes the disc, version, layout_id, and zeroes flags/reserved.
    /// Call this when creating a new account before writing field data.
    #[inline(always)]
    pub fn init_layout<T: LayoutContract>(&self) -> ProgramResult {
        let mut data = self.try_borrow_mut()?;
        crate::layout::init_header::<T>(&mut data)
    }

    // ── Validation helpers ───────────────────────────────────────────

    /// Validate that this account is a signer.
    #[inline(always)]
    pub fn require_signer(&self) -> ProgramResult {
        if self.is_signer() { Ok(()) } else { Err(ProgramError::MissingRequiredSignature) }
    }

    /// Validate that this account is writable.
    #[inline(always)]
    pub fn require_writable(&self) -> ProgramResult {
        if self.is_writable() { Ok(()) } else { Err(ProgramError::Immutable) }
    }

    /// Validate that this account is owned by the given program.
    #[inline(always)]
    pub fn require_owned_by(&self, program: &Address) -> ProgramResult {
        if self.owned_by(program) { Ok(()) } else { Err(ProgramError::IncorrectProgramId) }
    }

    /// Validate signer + writable (common "payer" pattern).
    #[inline(always)]
    pub fn require_payer(&self) -> ProgramResult {
        self.require_signer()?;
        self.require_writable()
    }

    // ── Chainable validation ─────────────────────────────────────────

    /// Chainable signer check.
    #[inline(always)]
    pub fn check_signer(&self) -> Result<&Self, ProgramError> {
        if self.is_signer() { Ok(self) } else { Err(ProgramError::MissingRequiredSignature) }
    }

    /// Chainable writable check.
    #[inline(always)]
    pub fn check_writable(&self) -> Result<&Self, ProgramError> {
        if self.is_writable() { Ok(self) } else { Err(ProgramError::Immutable) }
    }

    /// Chainable ownership check.
    #[inline(always)]
    pub fn check_owned_by(&self, program: &Address) -> Result<&Self, ProgramError> {
        if self.owned_by(program) { Ok(self) } else { Err(ProgramError::IncorrectProgramId) }
    }

    /// Chainable discriminator check.
    #[inline(always)]
    pub fn check_disc(&self, expected: u8) -> Result<&Self, ProgramError> {
        if self.disc() == expected { Ok(self) } else { Err(ProgramError::InvalidAccountData) }
    }

    /// Chainable non-empty data check.
    #[inline(always)]
    pub fn check_has_data(&self) -> Result<&Self, ProgramError> {
        if !self.is_data_empty() { Ok(self) } else { Err(ProgramError::AccountDataTooSmall) }
    }

    /// Chainable executable check.
    #[inline(always)]
    pub fn check_executable(&self) -> Result<&Self, ProgramError> {
        if self.executable() { Ok(self) } else { Err(ProgramError::InvalidArgument) }
    }

    /// Chainable address check.
    #[inline(always)]
    pub fn check_address(&self, expected: &Address) -> Result<&Self, ProgramError> {
        if address_eq(self.address(), expected) { Ok(self) } else { Err(ProgramError::InvalidArgument) }
    }

    /// Chainable minimum data length check.
    #[inline(always)]
    pub fn check_data_len(&self, min_len: usize) -> Result<&Self, ProgramError> {
        if self.data_len() >= min_len { Ok(self) } else { Err(ProgramError::AccountDataTooSmall) }
    }

    /// Chainable version check.
    #[inline(always)]
    pub fn check_version(&self, expected: u8) -> Result<&Self, ProgramError> {
        if self.version() == expected { Ok(self) } else { Err(ProgramError::InvalidAccountData) }
    }

    /// Chainable full layout contract check (disc + version + layout_id + size).
    #[inline(always)]
    pub fn check_layout<T: LayoutContract>(&self) -> Result<&Self, ProgramError> {
        let data = self.try_borrow()?;
        T::validate_header(&data)?;
        Ok(self)
    }

    // ── Hopper header readers ────────────────────────────────────────

    /// Read the Hopper account discriminator (first byte of data).
    #[inline(always)]
    pub fn disc(&self) -> u8 {
        compat::disc(&self.inner)
    }

    /// Read the Hopper account version (second byte of data).
    #[inline(always)]
    pub fn version(&self) -> u8 {
        compat::version(&self.inner)
    }

    /// Read the 8-byte layout_id from the Hopper account header (bytes 4..12).
    #[inline(always)]
    pub fn layout_id(&self) -> Option<&[u8; 8]> {
        compat::layout_id(&self.inner)
    }

    /// Verify that this account has the given discriminator.
    #[inline(always)]
    pub fn require_disc(&self, expected: u8) -> ProgramResult {
        if self.disc() == expected { Ok(()) } else { Err(ProgramError::InvalidAccountData) }
    }

    // ── Packed flags ─────────────────────────────────────────────────

    /// Pack the account's boolean flags into a single byte.
    ///
    /// Bit layout: bit 0 = signer, bit 1 = writable, bit 2 = executable,
    /// bit 3 = has data.
    #[inline(always)]
    pub fn flags(&self) -> u8 {
        let mut f: u8 = 0;
        if self.is_signer() { f |= 0b0001; }
        if self.is_writable() { f |= 0b0010; }
        if self.executable() { f |= 0b0100; }
        if !self.is_data_empty() { f |= 0b1000; }
        f
    }

    /// Check that the account's flags contain all required bits.
    #[inline(always)]
    pub fn expect_flags(&self, required: u8) -> ProgramResult {
        if self.flags() & required == required { Ok(()) } else { Err(ProgramError::InvalidArgument) }
    }

    // ── Resize / Close ───────────────────────────────────────────────

    /// Resize the account data.
    #[inline]
    pub fn resize(&self, new_len: usize) -> ProgramResult {
        self.inner.resize(new_len).map_err(ProgramError::from)
    }

    /// Assign a new owner.
    ///
    /// # Safety
    ///
    /// The caller must ensure the account is writable and that ownership
    /// transfer is authorized.
    #[inline(always)]
    pub unsafe fn assign(&self, new_owner: &Address) {
        unsafe { compat::assign(&self.inner, new_owner); }
    }

    /// Close the account: zero lamports and data.
    #[inline]
    pub fn close(&self) -> ProgramResult {
        compat::close(&self.inner)
    }

    /// Close the account, transferring remaining lamports to `destination`.
    ///
    /// This is the idiomatic Solana close pattern: move all lamports to the
    /// destination account, then zero this account's data so the runtime
    /// garbage-collects it at the end of the transaction.
    #[inline]
    pub fn close_to(&self, destination: &AccountView) -> ProgramResult {
        let lamports = self.lamports();
        let dest_lamports = destination.lamports();
        destination.set_lamports(dest_lamports.checked_add(lamports).ok_or(ProgramError::ArithmeticOverflow)?);
        self.set_lamports(0);
        compat::zero_data(&self.inner)?;
        Ok(())
    }

    // ── Raw access (hopper-native-backend only) ──────────────────────

    /// Raw pointer to the first byte of account data.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub fn data_ptr(&self) -> *mut u8 {
        self.inner.data_ptr()
    }

    /// Raw pointer to the RuntimeAccount header.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub fn account_ptr(&self) -> *const hopper_native::RuntimeAccount {
        self.inner.account_ptr()
    }

    /// Resize delta (hopper-native only).
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub fn resize_delta(&self) -> i32 {
        self.inner.resize_delta()
    }

    /// Whether the account data is currently borrowed.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub fn is_borrowed(&self) -> bool {
        self.inner.is_borrowed()
    }

    /// Whether the account data is exclusively borrowed.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub fn is_borrowed_mut(&self) -> bool {
        self.inner.is_borrowed_mut()
    }

    /// Check that the account can be shared-borrowed.
    #[inline(always)]
    pub fn check_borrow(&self) -> Result<(), ProgramError> {
        self.inner.check_borrow().map_err(ProgramError::from)
    }

    /// Check that the account can be exclusively borrowed.
    #[inline(always)]
    pub fn check_borrow_mut(&self) -> Result<(), ProgramError> {
        self.inner.check_borrow_mut().map_err(ProgramError::from)
    }

    /// Borrow account data without tracking.
    ///
    /// # Safety
    ///
    /// The caller must ensure no mutable borrow is active.
    #[inline(always)]
    pub unsafe fn borrow_unchecked(&self) -> &[u8] {
        unsafe { self.inner.borrow_unchecked() }
    }

    /// Mutably borrow account data without tracking.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other borrows are active.
    #[inline(always)]
    pub unsafe fn borrow_unchecked_mut(&self) -> &mut [u8] {
        unsafe { self.inner.borrow_unchecked_mut() }
    }

    /// Resize without bounds checking.
    ///
    /// # Safety
    ///
    /// The caller must guarantee the new length is within the permitted increase.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub unsafe fn resize_unchecked(&self, new_len: usize) {
        unsafe { self.inner.resize_unchecked(new_len); }
    }

    /// Close without borrow checks.
    ///
    /// # Safety
    ///
    /// The caller must ensure no active borrows exist.
    #[inline(always)]
    pub unsafe fn close_unchecked(&self) {
        unsafe { self.inner.close_unchecked(); }
    }

    // ── Backend access ───────────────────────────────────────────────

    /// Access the active backend account view inside the runtime crate.
    #[cfg(feature = "solana-program-backend")]
    #[inline(always)]
    pub(crate) fn as_backend(&self) -> &BackendAccountView {
        &self.inner
    }

    /// Access the underlying backend AccountView.
    ///
    /// Escape hatch for code that needs backend-specific functionality.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub fn as_native(&self) -> &hopper_native::AccountView {
        &self.inner
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

/// Iterator over remaining (unstructured) accounts.
pub struct RemainingAccounts<'a> {
    accounts: &'a [AccountView],
    cursor: usize,
}

impl<'a> RemainingAccounts<'a> {
    /// Create from a slice of accounts.
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
