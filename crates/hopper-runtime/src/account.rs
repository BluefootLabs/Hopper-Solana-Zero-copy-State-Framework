//! Hopper-owned account view for Solana programs.
//!
//! `AccountView` is the canonical typed state gateway for Hopper programs.
//! It wraps the active backend's account representation behind a
//! `#[repr(transparent)]` boundary, delegating all methods with zero-cost
//! type conversion.
//!
//! Key capabilities:
//! - Chainable validation (`check_signer()?.check_writable()?`)
//! - Whole-layout typed access (`load::<T>()`, `load_mut::<T>()`)
//! - Segment-aware typed access (`segment_ref`, `segment_mut`)
//! - Explicit raw escape hatches (`raw_ref`, `raw_mut`)
//! - Hopper header reading (disc, version, layout_id)
//! - Packed flags for batch validation
//! - Remaining accounts iterator

use crate::address::{Address, address_eq};
use crate::error::ProgramError;
use crate::borrow::{Ref, RefMut};
use crate::borrow_registry::{self, BorrowToken};
use crate::compat::{self, BackendAccountView};
use crate::field_map::FieldInfo;
use crate::layout::LayoutContract;
use crate::segment_borrow::SegmentBorrowRegistry;
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
    #[cfg(test)]
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendAccountView) -> Self {
        Self { inner }
    }

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
    #[inline(always)]
    pub fn try_borrow(&self) -> Result<Ref<'_, [u8]>, ProgramError> {
        let token = BorrowToken::shared(self.address())?;
        match self.inner.try_borrow() {
            Ok(data) => Ok(Ref::from_backend(data, token)),
            Err(error) => {
                drop(token);
                Err(ProgramError::from(error))
            }
        }
    }

    /// Try to obtain an exclusive (mutable) borrow of the account data.
    #[inline(always)]
    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, [u8]>, ProgramError> {
        let token = BorrowToken::mutable(self.address())?;
        match self.inner.try_borrow_mut() {
            Ok(data) => Ok(RefMut::from_backend(data, token)),
            Err(error) => {
                drop(token);
                Err(ProgramError::from(error))
            }
        }
    }

    // ── Segment-aware access ───────────────────────────────────────

    /// Project a typed segment from this account with segment-level borrow tracking.
    ///
    /// This is Hopper's fine-grained typed access primitive: the runtime
    /// validates the requested byte range, registers the read borrow in the
    /// provided instruction-scoped registry, and returns a typed reference
    /// directly into account bytes.
    ///
    /// On the native backend (Solana), this uses direct segment access
    /// bypassing the intermediate whole-buffer borrow for lower CU cost.
    #[inline(always)]
    pub fn segment_ref<T: Copy>(
        &self,
        borrows: &mut SegmentBorrowRegistry,
        abs_offset: u32,
        size: u32,
    ) -> Result<Ref<'_, T>, ProgramError> {
        let expected_size = core::mem::size_of::<T>() as u32;
        if size != expected_size {
            return Err(ProgramError::InvalidArgument);
        }

        let end = abs_offset
            .checked_add(size)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end as usize > self.data_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        borrows.register_read(self.address(), abs_offset, size)?;

        // On native backend: use direct segment access to skip the
        // intermediate Ref<[u8]> + project() overhead.
        #[cfg(target_os = "solana")]
        {
            // SAFETY: size, overflow, and bounds already validated above.
            let native_ref = unsafe {
                self.inner.segment_ref_unchecked::<T>(abs_offset)
            }.map_err(ProgramError::from)?;
            let (typed_ptr, state_ptr) = native_ref.into_raw_parts();
            // Create a dummy guard that manages the same borrow state.
            // The runtime Ref's Deref uses self.ptr, never the guard's data.
            let guard = unsafe {
                compat::BackendRef::from_raw_parts(b"" as &[u8], state_ptr)
            };
            Ok(Ref::from_segment(typed_ptr as *const T, guard))
        }
        #[cfg(not(target_os = "solana"))]
        {
            let data = self.try_borrow()?;
            let ptr = unsafe { data.as_bytes_ptr().add(abs_offset as usize) as *const T };
            Ok(unsafe { data.project(ptr) })
        }
    }

    /// Project a mutable typed segment from this account with segment-level borrow tracking.
    ///
    /// The caller supplies the instruction-scoped borrow registry. Hopper checks
    /// that the account is writable, validates the requested byte range, records
    /// the write borrow, and returns a direct mutable reference into the buffer.
    ///
    /// On the native backend (Solana), this uses direct segment access
    /// bypassing the intermediate whole-buffer borrow for lower CU cost.
    #[inline(always)]
    pub fn segment_mut<T: Copy>(
        &self,
        borrows: &mut SegmentBorrowRegistry,
        abs_offset: u32,
        size: u32,
    ) -> Result<RefMut<'_, T>, ProgramError> {
        self.check_writable()?;

        let expected_size = core::mem::size_of::<T>() as u32;
        if size != expected_size {
            return Err(ProgramError::InvalidArgument);
        }

        let end = abs_offset
            .checked_add(size)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end as usize > self.data_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        borrows.register_write(self.address(), abs_offset, size)?;

        // On native backend: use direct segment access to skip the
        // intermediate RefMut<[u8]> + project() overhead.
        #[cfg(target_os = "solana")]
        {
            // SAFETY: writable, size, overflow, and bounds already validated above.
            let native_ref = unsafe {
                self.inner.segment_mut_unchecked::<T>(abs_offset)
            }.map_err(ProgramError::from)?;
            let (typed_ptr, state_ptr) = native_ref.into_raw_parts();
            // SAFETY: dangling pointer for a zero-length slice. The RefMut
            // guard only manages the borrow state byte — it never reads
            // through this pointer.
            let dummy = unsafe {
                core::slice::from_raw_parts_mut(core::ptr::NonNull::dangling().as_ptr(), 0)
            };
            let guard = unsafe {
                compat::BackendRefMut::from_raw_parts(dummy, state_ptr)
            };
            Ok(RefMut::from_segment(typed_ptr as *const T as *mut T, guard))
        }
        #[cfg(not(target_os = "solana"))]
        {
            let mut data = self.try_borrow_mut()?;
            let ptr = unsafe { data.as_bytes_mut_ptr().add(abs_offset as usize) as *mut T };
            Ok(unsafe { data.project(ptr) })
        }
    }

    // ── Zero-copy overlay access ─────────────────────────────────────

    /// Low-level whole-buffer overlay helper.
    ///
    /// Prefer `load()` / `load_mut()` for Hopper-owned layouts and
    /// `segment_ref()` / `segment_mut()` when you want precise byte-range
    /// borrowing. `overlay()` is the direct slice-to-type escape hatch for
    /// fixed-layout expert paths.
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
    /// let state = account.overlay::<MyState>()?;
    /// ```
    ///
    /// **Deprecated:** Use `load()` for Hopper layouts or `raw_ref()` for
    /// explicit unvalidated access.
    #[deprecated(since = "0.2.0", note = "use load() for Hopper layouts or raw_ref() for explicit bypass")]
    #[inline(always)]
    pub fn overlay<T: Copy>(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        if data.len() < core::mem::size_of::<T>() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = data.as_ptr() as *const T;
        // SAFETY: Bounds checked above. `ptr` points into the bytes protected by `data`.
        Ok(unsafe { data.project(ptr) })
    }

    /// Low-level mutable whole-buffer overlay helper.
    ///
    /// Prefer `load_mut()` for Hopper-authored whole-layout access and
    /// `segment_mut()` for precise segment writes. `overlay_mut()` exists for
    /// explicit fixed-layout slice projection when you intentionally want the
    /// raw buffer shape.
    ///
    /// **Deprecated:** Use `load_mut()` or `raw_mut()` instead.
    #[deprecated(since = "0.2.0", note = "use load_mut() or raw_mut() instead")]
    #[inline(always)]
    pub fn overlay_mut<T: Copy>(&self) -> Result<RefMut<'_, T>, ProgramError> {
        let mut data = self.try_borrow_mut()?;
        if data.len() < core::mem::size_of::<T>() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = data.as_mut_ptr() as *mut T;
        // SAFETY: Bounds checked above. `ptr` points into the bytes protected by `data`.
        Ok(unsafe { data.project(ptr) })
    }

    /// Interpret account data at a specific offset as a typed overlay.
    ///
    /// Useful for reading past a header or into a specific region.
    ///
    /// **Deprecated:** Use `segment_ref()` with a typed segment offset for
    /// tracked reads, or `load()` for full-layout projection.
    #[deprecated(since = "0.2.0", note = "use segment_ref() or load() instead")]
    #[inline(always)]
    pub fn overlay_at<T: Copy>(&self, offset: usize) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        let end = offset.checked_add(core::mem::size_of::<T>())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if data.len() < end {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = unsafe { data.as_bytes_ptr().add(offset) as *const T };
        // SAFETY: Bounds checked above. `ptr` points into the bytes protected by `data`.
        Ok(unsafe { data.project(ptr) })
    }

    // ── Typed load (LayoutContract-aware) ────────────────────────────

    /// Load a typed layout after validating the account header.
    ///
    /// This is the canonical "validate then project" path:
    /// 1. Check disc, version, and layout_id match `T`
    /// 2. Verify data length >= `T::SIZE`
    /// 3. Return zero-copy reference into account data
    ///
    /// The returned reference begins at `T::TYPE_OFFSET`. Body-only layouts
    /// project past the Hopper header; header-inclusive layouts project the
    /// full account struct from byte 0.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let vault = account.load::<Vault>()?;
    /// ```
    #[inline(always)]
    pub fn load<T: LayoutContract>(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        T::validate_header(&data)?;
        if data.len() < T::required_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = unsafe { data.as_bytes_ptr().add(T::TYPE_OFFSET) as *const T };
        // SAFETY: Header and length validated above. `ptr` points into the borrowed bytes.
        Ok(unsafe { data.project(ptr) })
    }

    /// Load a mutable typed layout after validating the account header.
    ///
    /// Same as `load()` but provides a mutable reference for in-place
    /// state updates. Changes write directly to account data.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut vault = account.load_mut::<Vault>()?;
    /// vault.balance = vault.balance.checked_add(amount)?;
    /// ```
    #[inline(always)]
    pub fn load_mut<T: LayoutContract>(&self) -> Result<RefMut<'_, T>, ProgramError> {
        let mut data = self.try_borrow_mut()?;
        T::validate_header(&data)?;
        if data.len() < T::required_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = unsafe { data.as_bytes_mut_ptr().add(T::TYPE_OFFSET) as *mut T };
        // SAFETY: Header and length validated above. `ptr` points into the borrowed bytes.
        Ok(unsafe { data.project(ptr) })
    }

    /// Explicit raw typed read of the account buffer.
    ///
    /// This bypasses Hopper layout validation and segment tracking, but it still
    /// respects the account-level borrow rules enforced by `try_borrow()`.
    #[inline(always)]
    pub unsafe fn raw_ref<T: Copy>(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        if core::mem::size_of::<T>() > data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = data.as_ptr() as *const T;
        Ok(unsafe { data.project(ptr) })
    }

    /// Explicit raw typed write of the account buffer.
    ///
    /// This bypasses Hopper layout validation and segment tracking, but it still
    /// enforces writability and the account-level exclusive borrow rules.
    #[inline(always)]
    pub unsafe fn raw_mut<T: Copy>(&self) -> Result<RefMut<'_, T>, ProgramError> {
        self.check_writable()?;
        let mut data = self.try_borrow_mut()?;
        if core::mem::size_of::<T>() > data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = data.as_bytes_mut_ptr() as *mut T;
        Ok(unsafe { data.project(ptr) })
    }

    /// Load a typed layout checking only the discriminator (fast path).
    ///
    /// Skips version and layout_id checks. Use when you trust the account
    /// source and only need type dispatch.
    ///
    /// **Deprecated:** Use `load()` (validates disc + version + layout_id) or
    /// `raw_ref()` (skips all checks) instead. Partial validation is a
    /// foot-gun: either validate fully or take explicit responsibility.
    #[deprecated(since = "0.2.0", note = "use load() for safe access or raw_ref() for explicit bypass")]
    #[inline(always)]
    pub fn load_unchecked<T: LayoutContract>(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        T::check_disc(&data)?;
        if data.len() < T::required_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = unsafe { data.as_bytes_ptr().add(T::TYPE_OFFSET) as *const T };
        // SAFETY: Discriminator and size validated above.
        Ok(unsafe { data.project(ptr) })
    }

    /// Load a layout with version compatibility checking.
    ///
    /// Like `load()` but uses `T::compatible(version)` instead of an exact
    /// version match. This allows loading older account versions that the
    /// current layout version still understands (e.g. forward-compatible
    /// append-only migrations).
    ///
    /// **Deprecated:** Use `load()` with the upcoming migration framework,
    /// or implement version-aware loading in your instruction handler.
    #[deprecated(since = "0.2.0", note = "use load() or implement version-aware loading in your handler")]
    #[inline(always)]
    pub fn load_versioned<T: LayoutContract>(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        T::check_disc(&data)?;
        let version = crate::layout::read_version(&data)
            .ok_or(ProgramError::AccountDataTooSmall)?;
        if !T::compatible(version) {
            return Err(ProgramError::InvalidAccountData);
        }
        if data.len() < T::required_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = unsafe { data.as_bytes_ptr().add(T::TYPE_OFFSET) as *const T };
        // SAFETY: Discriminator, compatibility, and size validated above.
        Ok(unsafe { data.project(ptr) })
    }

    /// Load a foreign layout without ownership or authorization checks.
    ///
    /// Only validates the wire format (disc + layout_id + size). Use
    /// this for cross-program reads where the account is owned by
    /// another program and you just need a typed view of its data.
    ///
    /// **Deprecated:** Use `load_cross_program()` which replaces this with
    /// the same semantics but a clearer name.
    #[deprecated(since = "0.2.0", note = "renamed to load_cross_program()")]
    #[inline(always)]
    pub fn load_foreign<T: LayoutContract>(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        if data.len() < T::required_len() {
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
        let ptr = unsafe { data.as_bytes_ptr().add(T::TYPE_OFFSET) as *const T };
        // SAFETY: Wire identity and size validated above.
        Ok(unsafe { data.project(ptr) })
    }

    /// Load a cross-program layout without ownership checks.
    ///
    /// Validates wire format (disc + layout_id + size) but does not check
    /// that the account is owned by this program. Use for cross-program
    /// reads where the account is owned by another program and you need
    /// a typed, zero-copy view of its data.
    ///
    /// The layout_id check ensures ABI compatibility: if the other program
    /// changes its layout, this will fail rather than silently misinterpret.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let other_vault = foreign_account.load_cross_program::<OtherVault>()?;
    /// ```
    #[inline(always)]
    pub fn load_cross_program<T: LayoutContract>(&self) -> Result<Ref<'_, T>, ProgramError> {
        let data = self.try_borrow()?;
        if data.len() < T::required_len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        T::check_disc(&data)?;
        if let Some(id) = crate::layout::read_layout_id(&data) {
            if *id != T::LAYOUT_ID {
                return Err(ProgramError::InvalidAccountData);
            }
        } else {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let ptr = unsafe { data.as_bytes_ptr().add(T::TYPE_OFFSET) as *const T };
        // SAFETY: Wire identity and size validated above.
        Ok(unsafe { data.project(ptr) })
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

    /// Alias for runtime layout inspection.
    #[deprecated(since = "0.2.0", note = "use layout_info() directly")]
    #[inline(always)]
    pub fn inspect(&self) -> Option<crate::layout::LayoutInfo> {
        self.layout_info()
    }

    /// Compile-time field metadata for a layout contract.
    #[inline(always)]
    pub fn fields<T: LayoutContract>() -> &'static [FieldInfo] {
        T::fields()
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

    /// Borrow the extension/tail region declared by a layout contract.
    #[inline(always)]
    pub fn extension_bytes<T: LayoutContract>(&self) -> Result<Ref<'_, [u8]>, ProgramError> {
        let offset = T::EXTENSION_OFFSET.ok_or(ProgramError::InvalidArgument)?;
        let data = self.try_borrow()?;
        if data.len() < offset {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(data.slice_from(offset))
    }

    /// Mutably borrow the extension/tail region declared by a layout contract.
    #[inline(always)]
    pub fn extension_bytes_mut<T: LayoutContract>(&self) -> Result<RefMut<'_, [u8]>, ProgramError> {
        let offset = T::EXTENSION_OFFSET.ok_or(ProgramError::InvalidArgument)?;
        let data = self.try_borrow_mut()?;
        if data.len() < offset {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(data.slice_from(offset))
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
    pub(crate) fn data_ptr(&self) -> *mut u8 {
        self.inner.data_ptr()
    }

    /// Raw pointer to the RuntimeAccount header.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub(crate) fn account_ptr(&self) -> *const hopper_native::RuntimeAccount {
        self.inner.account_ptr()
    }

    /// Check that the account can be shared-borrowed.
    #[inline(always)]
    pub fn check_borrow(&self) -> Result<(), ProgramError> {
        borrow_registry::check_shared(self.address())?;
        self.inner.check_borrow().map_err(ProgramError::from)
    }

    /// Check that the account can be exclusively borrowed.
    #[inline(always)]
    pub fn check_borrow_mut(&self) -> Result<(), ProgramError> {
        borrow_registry::check_mutable(self.address())?;
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
    #[cfg(target_os = "solana")]
    #[inline(always)]
    pub(crate) fn as_backend(&self) -> &BackendAccountView {
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

#[cfg(all(test, feature = "hopper-native-backend"))]
mod tests {
    use super::*;
    use crate::layout::HopperHeader;

    use hopper_native::{AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED};

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    struct TestLayout {
        a: u64,
        b: u64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct HeaderLayout {
        header: HopperHeader,
        amount: u64,
    }

    impl crate::field_map::FieldMap for TestLayout {
        const FIELDS: &'static [crate::field_map::FieldInfo] = &[
            crate::field_map::FieldInfo::new("a", HopperHeader::SIZE, 8),
            crate::field_map::FieldInfo::new("b", HopperHeader::SIZE + 8, 8),
        ];
    }

    impl LayoutContract for TestLayout {
        const DISC: u8 = 7;
        const VERSION: u8 = 1;
        const LAYOUT_ID: [u8; 8] = [0xAB; 8];
        const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
        const EXTENSION_OFFSET: Option<usize> = Some(Self::SIZE);
    }

    impl crate::field_map::FieldMap for HeaderLayout {
        const FIELDS: &'static [crate::field_map::FieldInfo] = &[
            crate::field_map::FieldInfo::new("amount", HopperHeader::SIZE, 8),
        ];
    }

    impl LayoutContract for HeaderLayout {
        const DISC: u8 = 11;
        const VERSION: u8 = 2;
        const LAYOUT_ID: [u8; 8] = [0xCD; 8];
        const SIZE: usize = core::mem::size_of::<Self>();
        const TYPE_OFFSET: usize = 0;
    }

    fn make_account(total_data_len: usize, address_byte: u8) -> (std::vec::Vec<u8>, AccountView) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + total_data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([address_byte; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports: 42,
                data_len: total_data_len as u64,
            });
        }
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let account = AccountView::from_backend(backend);
        (backing, account)
    }

    #[test]
    fn load_mut_is_zero_copy_and_pointer_stable() {
        let (_backing, account) = make_account(TestLayout::SIZE + 8, 1);

        {
            let mut data = account.try_borrow_mut().unwrap();
            crate::layout::init_header::<TestLayout>(&mut data).unwrap();
            data[HopperHeader::SIZE..HopperHeader::SIZE + 8].copy_from_slice(&10u64.to_le_bytes());
            data[HopperHeader::SIZE + 8..HopperHeader::SIZE + 16].copy_from_slice(&20u64.to_le_bytes());
            data[TestLayout::SIZE..TestLayout::SIZE + 8].copy_from_slice(b"tailpass");
        }

        let first_ptr = {
            let first = account.load::<TestLayout>().unwrap();
            assert_eq!(first.a, 10);
            assert_eq!(first.b, 20);
            first.as_ptr() as usize
        };

        {
            let tail = account.extension_bytes::<TestLayout>().unwrap();
            assert_eq!(&tail[..8], b"tailpass");
        }

        let mut second = account.load_mut::<TestLayout>().unwrap();
        let second_ptr = second.as_mut_ptr() as usize;
        second.b = 99;
        assert_eq!(first_ptr, second_ptr);
        drop(second);

        let reread = account.load::<TestLayout>().unwrap();
        assert_eq!(reread.a, 10);
        assert_eq!(reread.b, 99);
    }

    #[test]
    fn typed_load_holds_borrow_until_drop() {
        let (_backing, account) = make_account(TestLayout::SIZE, 3);

        {
            let mut data = account.try_borrow_mut().unwrap();
            crate::layout::init_header::<TestLayout>(&mut data).unwrap();
        }

        let shared = account.load::<TestLayout>().unwrap();
        assert_eq!(account.load_mut::<TestLayout>().unwrap_err(), ProgramError::AccountBorrowFailed);
        drop(shared);
        assert!(account.load_mut::<TestLayout>().is_ok());
    }

    #[test]
    fn duplicate_address_aliases_are_rejected_across_views() {
        let (_first_backing, first) = make_account(TestLayout::SIZE, 9);
        let (_second_backing, second) = make_account(TestLayout::SIZE, 9);

        let first_shared = first.try_borrow().unwrap();
        let second_shared = second.try_borrow().unwrap();
        assert_eq!(second.try_borrow_mut().unwrap_err(), ProgramError::AccountBorrowFailed);
        drop(first_shared);
        drop(second_shared);
        assert!(second.try_borrow_mut().is_ok());
    }

    #[test]
    fn load_rejects_wrong_disc_and_wrong_version() {
        let (_backing, account) = make_account(TestLayout::SIZE, 4);

        {
            let mut data = account.try_borrow_mut().unwrap();
            crate::layout::init_header::<TestLayout>(&mut data).unwrap();
        }

        {
            let mut data = account.try_borrow_mut().unwrap();
            data[0] = TestLayout::DISC.wrapping_add(1);
        }
        assert_eq!(account.load::<TestLayout>().unwrap_err(), ProgramError::InvalidAccountData);

        {
            let mut data = account.try_borrow_mut().unwrap();
            crate::layout::init_header::<TestLayout>(&mut data).unwrap();
            data[1] = TestLayout::VERSION.wrapping_add(1);
        }
        assert_eq!(account.load::<TestLayout>().unwrap_err(), ProgramError::InvalidAccountData);
    }

    #[test]
    fn load_rejects_undersized_layout_body() {
        let (_backing, account) = make_account(TestLayout::SIZE - 1, 5);

        {
            let mut data = account.try_borrow_mut().unwrap();
            data[0] = TestLayout::DISC;
            data[1] = TestLayout::VERSION;
            data[4..12].copy_from_slice(&TestLayout::LAYOUT_ID);
        }

        assert_eq!(account.load::<TestLayout>().unwrap_err(), ProgramError::AccountDataTooSmall);
    }

    #[test]
    fn load_supports_header_inclusive_layouts() {
        let (_backing, account) = make_account(HeaderLayout::SIZE, 6);

        {
            let mut data = account.try_borrow_mut().unwrap();
            crate::layout::init_header::<HeaderLayout>(&mut data).unwrap();
        }

        {
            #[allow(deprecated)]
            let mut layout = account.overlay_mut::<HeaderLayout>().unwrap();
            layout.amount = 55;
        }

        let layout = account.load::<HeaderLayout>().unwrap();
        assert_eq!(layout.header.disc, HeaderLayout::DISC);
        assert_eq!(layout.header.version, HeaderLayout::VERSION);
        assert_eq!(layout.amount, 55);
    }
}
