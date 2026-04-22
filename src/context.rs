//! Execution context for Hopper programs.
//!
//! `Context` is the canonical execution object that Hopper handlers receive.
//! It provides structured access to the program_id, accounts, and instruction
//! data, with indexed access and validation helpers.
//!
//! Keep it boring: `Context` is the container for accounts, instruction data,
//! and the instruction-scoped segment borrow registry. `AccountView` owns the
//! actual access operations.

use crate::account::AccountView;
use crate::audit::AccountAudit;
use crate::address::Address;
use crate::error::ProgramError;
use crate::layout::LayoutContract;
use crate::segment_borrow::SegmentBorrowRegistry;
use crate::ProgramResult;

/// Execution context for a Hopper instruction handler.
///
/// Wraps the program_id, account slice, and instruction data into a single
/// object with structured access patterns.
///
/// # Authored flow
///
/// ```ignore
/// pub fn deposit(ctx: &Context, amount: u64) -> ProgramResult {
///     let authority = ctx.account(0)?;
///     let vault = ctx.account(1)?;
///
///     authority.require_signer()?;
///     vault.require_writable()?;
///     vault.check_disc(1)?;
///
///     let mut state = vault.load_mut::<VaultState>()?;
///     state.balance = state.balance.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;
///     Ok(())
/// }
/// ```
pub struct Context<'a> {
    /// The program's own address.
    pub program_id: &'a Address,
    /// All accounts passed to this instruction.
    accounts: &'a [AccountView],
    /// Raw instruction data (past the discriminator byte, if applicable).
    pub instruction_data: &'a [u8],
    /// Segment-level borrow tracking for fine-grained access control.
    ///
    /// Enables safe concurrent mutable access to non-overlapping regions
    /// of the same account. This is what makes Hopper strictly safer than
    /// raw Pinocchio without adding meaningful CU overhead.
    /// Prefer the `borrows()` / `borrows_mut()` accessors in new code.
    pub(crate) segment_borrows: SegmentBorrowRegistry,
}

impl<'a> Context<'a> {
    /// Create a new context from the entrypoint parameters.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView],
        instruction_data: &'a [u8],
    ) -> Self {
        Self {
            program_id,
            accounts,
            instruction_data,
            segment_borrows: SegmentBorrowRegistry::new(),
        }
    }

    /// Program ID.
    #[inline(always)]
    pub fn program_id(&self) -> &Address {
        self.program_id
    }

    /// Raw instruction data.
    #[inline(always)]
    pub fn instruction_data(&self) -> &'a [u8] {
        self.instruction_data
    }

    /// Get an account by index.
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<&AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get an account by index (mutation-intent variant).
    ///
    /// Functionally identical to `account()` since `AccountView` uses
    /// interior mutability for data access (`overlay_mut`, `load_mut`,
    /// `try_borrow_mut`). The distinct name signals that the caller
    /// intends to write through the returned reference.
    #[inline(always)]
    pub fn account_mut(&self, index: usize) -> Result<&AccountView, ProgramError> {
        self.accounts.get(index).ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get the total number of accounts.
    #[inline(always)]
    pub fn num_accounts(&self) -> usize {
        self.accounts.len()
    }

    /// Get all accounts as a slice.
    #[inline(always)]
    pub fn accounts(&self) -> &[AccountView] {
        self.accounts
    }

    /// Access the instruction-scoped segment borrow registry.
    #[inline(always)]
    pub fn borrows(&self) -> &SegmentBorrowRegistry {
        &self.segment_borrows
    }

    /// Mutably access the instruction-scoped segment borrow registry.
    #[inline(always)]
    pub fn borrows_mut(&mut self) -> &mut SegmentBorrowRegistry {
        &mut self.segment_borrows
    }

    /// Inspect the instruction account slice for duplicate aliases.
    #[inline(always)]
    pub fn audit_accounts(&self) -> AccountAudit<'a> {
        AccountAudit::new(self.accounts)
    }

    /// Get the remaining accounts starting at `from`.
    #[inline(always)]
    pub fn remaining_accounts(&self, from: usize) -> &[AccountView] {
        if from >= self.accounts.len() {
            &[]
        } else {
            &self.accounts[from..]
        }
    }

    /// Require at least `n` accounts are present.
    #[inline(always)]
    pub fn require_accounts(&self, n: usize) -> ProgramResult {
        if self.accounts.len() >= n {
            Ok(())
        } else {
            Err(ProgramError::NotEnoughAccountKeys)
        }
    }

    /// Require all account addresses to be unique.
    #[inline(always)]
    pub fn require_unique_accounts(&self) -> ProgramResult {
        self.audit_accounts().require_all_unique()
    }

    /// Require that no duplicated account is writable in this instruction.
    #[inline(always)]
    pub fn require_unique_writable_accounts(&self) -> ProgramResult {
        self.audit_accounts().require_unique_writable()
    }

    /// Require that no duplicated account is used as a signer role.
    #[inline(always)]
    pub fn require_unique_signer_accounts(&self) -> ProgramResult {
        self.audit_accounts().require_unique_signers()
    }

    /// Require at least `n` bytes of instruction data.
    #[inline(always)]
    pub fn require_data_len(&self, n: usize) -> ProgramResult {
        if self.instruction_data.len() >= n {
            Ok(())
        } else {
            Err(ProgramError::InvalidInstructionData)
        }
    }

    // --- Whole-Layout Typed Access ----------------------------------

    /// Validate-and-load the full typed layout for an account.
    ///
    /// This is the indexed shortcut for `ctx.account(idx)?.load::<T>()`.
    /// It's the canonical "Tier A" access path: the runtime checks the
    /// Hopper header, validates the data length, and projects the typed
    /// view in one inlined call. no extra cost over the spelled-out form.
    #[inline(always)]
    pub fn load<T: LayoutContract>(
        &self,
        index: usize,
    ) -> Result<crate::Ref<'_, T>, ProgramError> {
        self.account(index)?.load::<T>()
    }

    /// Validate-and-load a mutable typed layout for an account.
    ///
    /// Indexed shortcut for `ctx.account(idx)?.load_mut::<T>()`. The
    /// returned guard holds the account-level exclusive borrow until
    /// it drops.
    #[inline(always)]
    pub fn load_mut<T: LayoutContract>(
        &self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        self.account(index)?.load_mut::<T>()
    }

    /// Cross-program load: validate ABI fingerprint without ownership check.
    ///
    /// Use this when reading an account whose owner is another program but
    /// whose layout is published as a Hopper layout contract.
    #[inline(always)]
    pub fn load_cross_program<T: LayoutContract>(
        &self,
        index: usize,
    ) -> Result<crate::Ref<'_, T>, ProgramError> {
        self.account(index)?.load_cross_program::<T>()
    }

    // --- Segment-Level Access (fine-grained borrow tracking) --------

    /// Register a read borrow for a segment of an account and return a
    /// [`SegRef<T>`](crate::SegRef) that releases both the account-level
    /// byte guard **and** the segment registry lease on drop.
    ///
    /// `index` is the account index. `abs_offset` is the absolute byte
    /// offset within the account data (including header bytes).
    ///
    /// # Type Safety
    ///
    /// `T` must implement `Pod` (substrate-level "safe to overlay on
    /// raw bytes" contract: every bit pattern valid, align-1, no
    /// padding, no interior pointers). Segment borrow tracking
    /// prevents conflicting write access to the same byte range for
    /// the guard's lifetime.
    ///
    /// # Canonical path (audit ST1 / winning-architecture spec)
    ///
    /// Three variants exist for different offset sources:
    ///
    /// | Variant | Use when |
    /// |---|---|
    /// | [`segment_ref_typed`](Self::segment_ref_typed) (canonical) | Offset is a compile-time constant (the common case). The `const OFFSET: u32` generic becomes an immediate in the pointer arithmetic. |
    /// | [`segment_ref_const`](Self::segment_ref_const) | Offset comes from a runtime [`Segment`] value (dispatching dynamically between named fields). |
    /// | `segment_ref` (this method) | Offset is fully dynamic (iterating segments in a loop, for example). |
    ///
    /// `#[hopper::context]`-generated accessors default to the canonical
    /// typed path; reach for the others only when the use case
    /// genuinely needs a runtime offset.
    #[inline(always)]
    pub fn segment_ref<'b, T: crate::Pod>(
        &'b mut self,
        index: usize,
        abs_offset: u32,
    ) -> Result<crate::SegRef<'b, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_ref::<T>(&mut self.segment_borrows, abs_offset, core::mem::size_of::<T>() as u32)
    }

    /// Register a write borrow for a segment of an account.
    ///
    /// Validates bounds, checks writable, and registers a leased
    /// exclusive borrow, then returns a [`SegRefMut<T>`](crate::SegRefMut)
    /// that releases on drop.
    ///
    /// This is the primitive that enables safe concurrent mutation of
    /// non-overlapping account regions. Hopper's core innovation . 
    /// and the lease model (added post-audit) makes sequential
    /// same-region borrows inside one instruction work correctly.
    #[inline(always)]
    pub fn segment_mut<'b, T: crate::Pod>(
        &'b mut self,
        index: usize,
        abs_offset: u32,
    ) -> Result<crate::SegRefMut<'b, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_mut::<T>(&mut self.segment_borrows, abs_offset, core::mem::size_of::<T>() as u32)
    }

    /// Const-driven segment read: pass a compile-time [`Segment`] and the
    /// account index. Lowers to the same pointer-plus-const-offset shape
    /// as `segment_ref` but without the caller hand-rolling the offset +
    /// size arguments.
    #[inline(always)]
    pub fn segment_ref_const<'b, T: crate::Pod>(
        &'b mut self,
        index: usize,
        segment: crate::Segment,
    ) -> Result<crate::SegRef<'b, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_ref_const::<T>(&mut self.segment_borrows, segment)
    }

    /// Const-driven exclusive segment access. Pair with
    /// `#[hopper::state]` constants for zero-overhead field writes.
    #[inline(always)]
    pub fn segment_mut_const<'b, T: crate::Pod>(
        &'b mut self,
        index: usize,
        segment: crate::Segment,
    ) -> Result<crate::SegRefMut<'b, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_mut_const::<T>(&mut self.segment_borrows, segment)
    }

    /// Typed-segment read: the type and offset are both compile-time
    /// constants, baked into a [`TypedSegment`] zero-sized marker.
    #[inline(always)]
    pub fn segment_ref_typed<'b, T: crate::Pod, const OFFSET: u32>(
        &'b mut self,
        index: usize,
        segment: crate::TypedSegment<T, OFFSET>,
    ) -> Result<crate::SegRef<'b, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_ref_typed::<T, OFFSET>(&mut self.segment_borrows, segment)
    }

    /// Typed-segment write. Mirrors [`segment_ref_typed`] for the
    /// exclusive path.
    #[inline(always)]
    pub fn segment_mut_typed<'b, T: crate::Pod, const OFFSET: u32>(
        &'b mut self,
        index: usize,
        segment: crate::TypedSegment<T, OFFSET>,
    ) -> Result<crate::SegRefMut<'b, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_mut_typed::<T, OFFSET>(&mut self.segment_borrows, segment)
    }

    /// Explicit unsafe whole-account typed read.
    #[inline(always)]
    pub unsafe fn raw_ref<T: crate::Pod>(
        &self,
        index: usize,
    ) -> Result<crate::Ref<'_, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        unsafe { view.raw_ref::<T>() }
    }

    /// Explicit unsafe whole-account typed write.
    #[inline(always)]
    pub unsafe fn raw_mut<T: crate::Pod>(
        &self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        let view = self.accounts.get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        unsafe { view.raw_mut::<T>() }
    }

    /// Explicit unsafe escape hatch for whole-account typed projection.
    ///
    /// This bypasses segment borrow tracking. The caller is responsible for
    /// alias safety and for using a type that matches the account bytes.
    #[inline(always)]
    pub unsafe fn raw_unchecked<T: crate::Pod>(
        &self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        unsafe { self.raw_mut::<T>(index) }
    }

    /// Canonical raw-pointer escape hatch to an account's data buffer.
    ///
    /// Returns a pointer to the first byte of `accounts[index]`'s data
    /// region (after the runtime account header, before any Hopper
    /// 16-byte layout header). The pointer is valid for reads and
    /// writes for the lifetime of the account view and carries no
    /// borrow-tracking obligations. Dereferencing it is `unsafe`
    /// because the caller takes over alias-safety responsibility
    /// that the segment registry normally upholds.
    ///
    /// This is the explicit power-user primitive the audit asks for:
    /// safe code reaches for `segment_ref_typed` / `segment_mut_typed`
    /// / the generated `ctx.<field>_segment_mut(...)` accessors; raw
    /// code drops to `unsafe { ctx.as_mut_ptr(0)?.add(offset) as *mut T }`.
    ///
    /// # Safety
    ///
    /// The caller must guarantee no aliasing mutable borrow is held
    /// on the same account for the duration of any write through the
    /// returned pointer. The returned pointer must be dereferenced
    /// within the `'info` lifetime of the account view; reading past
    /// `AccountView::data_len()` is undefined behaviour.
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub unsafe fn as_mut_ptr(&self, index: usize) -> Result<*mut u8, ProgramError> {
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.require_writable()?;
        // SAFETY: the account view is live for `'info` and
        // `data_ptr` yields a pointer inside the loader-provided
        // per-account buffer. Returning the untyped pointer transfers
        // alias-safety to the caller as documented above.
        Ok(view.data_ptr())
    }

    /// Immutable sibling of [`as_mut_ptr`]. Returns a `*const u8`.
    ///
    /// Shared-borrow checking still runs, so calling this while an
    /// exclusive borrow is live on the same account fails with
    /// `AccountBorrowFailed`. The return value is safe to obtain; the
    /// caller only needs `unsafe` to dereference it.
    ///
    /// [`as_mut_ptr`]: Self::as_mut_ptr
    #[cfg(feature = "hopper-native-backend")]
    #[inline(always)]
    pub fn as_ptr(&self, index: usize) -> Result<*const u8, ProgramError> {
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.check_borrow()?;
        Ok(view.data_ptr() as *const u8)
    }

    /// Read instruction data as a typed value (unaligned, little-endian safe).
    ///
    /// Reads `size_of::<T>()` bytes starting at `offset` via `read_unaligned`.
    /// Caller must ensure `T` is a plain-old-data type where all bit patterns
    /// are valid.
    #[inline(always)]
    pub fn read_data<T: crate::Pod>(&self, offset: usize) -> Result<T, ProgramError> {
        let end = offset.checked_add(core::mem::size_of::<T>())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if self.instruction_data.len() < end {
            return Err(ProgramError::InvalidInstructionData);
        }
        // SAFETY: bounds checked; `T: Pod` guarantees every bit
        // pattern is valid and the type has no drop glue, so
        // `read_unaligned` into instruction data is sound.
        Ok(unsafe {
            core::ptr::read_unaligned(self.instruction_data.as_ptr().add(offset) as *const T)
        })
    }

    /// Get a byte slice from instruction data.
    #[inline(always)]
    pub fn data_slice(&self, offset: usize, len: usize) -> Result<&[u8], ProgramError> {
        let end = offset.checked_add(len).ok_or(ProgramError::ArithmeticOverflow)?;
        if self.instruction_data.len() < end {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(&self.instruction_data[offset..end])
    }

    /// Read the first byte of instruction data as an instruction tag.
    ///
    /// Common pattern for byte-tag dispatch.
    #[inline(always)]
    pub fn instruction_tag(&self) -> Result<u8, ProgramError> {
        self.instruction_data.first().copied().ok_or(ProgramError::InvalidInstructionData)
    }
}
