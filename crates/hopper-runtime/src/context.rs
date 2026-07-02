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
use crate::address::Address;
use crate::audit::AccountAudit;
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
    accounts: &'a [AccountView<'a>],
    /// Raw instruction data (past the discriminator byte, if applicable).
    pub instruction_data: &'a [u8],
    /// Segment-level borrow tracking for fine-grained access control.
    ///
    /// Enables safe concurrent mutable access to non-overlapping regions
    /// of the same account while keeping typed access under Hopper's borrow
    /// registry.
    /// Prefer the `borrows()` / `borrows_mut()` accessors in new code.
    pub(crate) segment_borrows: SegmentBorrowRegistry,
}

impl<'a> Context<'a> {
    /// Create a new context from the entrypoint parameters.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView<'a>],
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
    pub fn account(&self, index: usize) -> Result<&'a AccountView<'a>, ProgramError> {
        self.accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get an account by index (mutation-intent variant).
    ///
    /// Functionally identical to `account()` since `AccountView` uses
    /// interior mutability for data access (`overlay_mut`, `load_mut`,
    /// `try_borrow_mut`). The distinct name signals that the caller
    /// intends to write through the returned reference.
    #[inline(always)]
    pub fn account_mut(&self, index: usize) -> Result<&'a AccountView<'a>, ProgramError> {
        self.accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Get the total number of accounts.
    #[inline(always)]
    pub fn num_accounts(&self) -> usize {
        self.accounts.len()
    }

    /// Get all accounts as a slice.
    #[inline(always)]
    pub fn accounts(&self) -> &'a [AccountView<'a>] {
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
    pub fn remaining_accounts(&self, from: usize) -> &'a [AccountView<'a>] {
        if from >= self.accounts.len() {
            &[]
        } else {
            &self.accounts[from..]
        }
    }

    /// Get remaining accounts in strict duplicate-rejecting mode.
    #[inline(always)]
    pub fn remaining_accounts_strict(
        &self,
        from: usize,
    ) -> crate::remaining::RemainingAccounts<'a> {
        let declared_end = from.min(self.accounts.len());
        crate::remaining::RemainingAccounts::strict(
            &self.accounts[..declared_end],
            self.remaining_accounts(from),
        )
    }

    /// Get remaining accounts in duplicate-preserving passthrough mode.
    #[inline(always)]
    pub fn remaining_accounts_passthrough(
        &self,
        from: usize,
    ) -> crate::remaining::RemainingAccounts<'a> {
        let declared_end = from.min(self.accounts.len());
        crate::remaining::RemainingAccounts::passthrough(
            &self.accounts[..declared_end],
            self.remaining_accounts(from),
        )
    }

    /// Get remaining accounts in strict mode and bind a sequential typed parser.
    #[inline(always)]
    pub fn remaining_accounts_typed(&self, from: usize) -> crate::remaining::RemainingTyped<'a> {
        self.remaining_accounts_strict(from).typed()
    }

    /// Get remaining accounts in strict mode and bind a lazy indexed parser.
    #[inline(always)]
    pub fn remaining_accounts_lazy(&self, from: usize) -> crate::remaining::RemainingLazy<'a> {
        self.remaining_accounts_strict(from).lazy()
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
    pub fn load<T: LayoutContract + crate::Pod>(
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
    pub fn load_mut<T: LayoutContract + crate::Pod>(
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
    pub fn load_cross_program<T: LayoutContract + crate::Pod>(
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
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_ref::<T>(
            &mut self.segment_borrows,
            abs_offset,
            core::mem::size_of::<T>() as u32,
        )
    }

    /// Borrow several disjoint typed sub-ranges of one account mutably at
    /// the same time. See
    /// [`AccountView::split_segments_mut`](crate::AccountView::split_segments_mut).
    ///
    /// ```ignore
    /// let mut segs = ctx.split_segments_mut::<WireU64, 2>(
    ///     vault_idx, [(BALANCE_OFF, 8), (NONCE_OFF, 8)])?;
    /// let [bal, nonce] = segs.all_mut();
    /// bal.set(bal.get() + amount);
    /// nonce.set(nonce.get() + 1);
    /// ```
    #[inline(always)]
    pub fn split_segments_mut<'b, T: crate::Pod, const N: usize>(
        &'b mut self,
        index: usize,
        ranges: [(u32, u32); N],
    ) -> Result<crate::SegmentsMut<'b, T, N>, ProgramError> {
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.split_segments_mut::<T, N>(&mut self.segment_borrows, ranges)
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
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_mut::<T>(
            &mut self.segment_borrows,
            abs_offset,
            core::mem::size_of::<T>() as u32,
        )
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
        let view = self
            .accounts
            .get(index)
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
        let view = self
            .accounts
            .get(index)
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
        let view = self
            .accounts
            .get(index)
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
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.segment_mut_typed::<T, OFFSET>(&mut self.segment_borrows, segment)
    }

    /// Explicit unsafe whole-account typed read.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn raw_ref<T: crate::Pod>(
        &self,
        index: usize,
    ) -> Result<crate::Ref<'_, T>, ProgramError> {
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { view.raw_ref::<T>() }
    }

    /// Explicit unsafe whole-account typed write.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn raw_mut<T: crate::Pod>(
        &self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { view.raw_mut::<T>() }
    }

    /// Legacy alias for [`raw_mut`](Self::raw_mut).
    ///
    /// Despite the name, this does **not** bypass borrow tracking: it
    /// delegates to `raw_mut`, which routes through the checked
    /// `segment_mut(0, size_of::<T>())` path (bounds, writable, and
    /// account-level exclusive borrow all enforced). The caller remains
    /// responsible for using a type that matches the account bytes. For a
    /// genuinely untracked pointer, use [`as_mut_ptr`](Self::as_mut_ptr).
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must uphold the invariants documented for this unsafe API before invoking it.
    pub unsafe fn raw_unchecked<T: crate::Pod>(
        &self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        Ok(view.data_ptr_unchecked())
    }

    /// Immutable sibling of [`as_mut_ptr`]. Returns a `*const u8`.
    ///
    /// Shared-borrow checking still runs, so calling this while an
    /// exclusive borrow is live on the same account fails with
    /// `AccountBorrowFailed`. The return value is safe to obtain; the
    /// caller only needs `unsafe` to dereference it.
    ///
    /// [`as_mut_ptr`]: Self::as_mut_ptr
    #[inline(always)]
    pub fn as_ptr(&self, index: usize) -> Result<*const u8, ProgramError> {
        let view = self
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        view.check_borrow()?;
        Ok(view.data_ptr_unchecked() as *const u8)
    }

    /// Read instruction data as a typed value (unaligned, little-endian safe).
    ///
    /// Reads `size_of::<T>()` bytes starting at `offset` via `read_unaligned`.
    /// Caller must ensure `T` is a plain-old-data type where all bit patterns
    /// are valid.
    #[inline(always)]
    pub fn read_data<T: crate::ValuePod>(&self, offset: usize) -> Result<T, ProgramError> {
        let end = offset
            .checked_add(core::mem::size_of::<T>())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if self.instruction_data.len() < end {
            return Err(ProgramError::InvalidInstructionData);
        }
        // SAFETY: bounds checked; `T: ValuePod` guarantees every bit
        // pattern is valid by value and the type has no drop glue, so
        // `read_unaligned` from instruction data is sound.
        Ok(unsafe {
            core::ptr::read_unaligned(self.instruction_data.as_ptr().add(offset) as *const T)
        })
    }

    /// Get a byte slice from instruction data.
    #[inline(always)]
    pub fn data_slice(&self, offset: usize, len: usize) -> Result<&[u8], ProgramError> {
        let end = offset
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
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
        self.instruction_data
            .first()
            .copied()
            .ok_or(ProgramError::InvalidInstructionData)
    }
}

/// Borrow-scoped view of a [`Context`].
///
/// Generated typed contexts expose this wrapper from their safe `raw()` method
/// instead of returning `&mut Context<'a>` directly. That keeps account and
/// remaining-account references tied to the borrow of the generated context,
/// preventing backend account-view lifetimes from being widened through the raw
/// escape hatch.
pub struct ScopedContext<'ctx, 'a> {
    inner: &'ctx mut Context<'a>,
}

impl<'ctx, 'a> ScopedContext<'ctx, 'a> {
    /// Create a borrow-scoped wrapper around a raw Hopper context.
    #[inline(always)]
    pub fn new(inner: &'ctx mut Context<'a>) -> Self {
        Self { inner }
    }

    /// Program ID, narrowed to the wrapper borrow lifetime.
    #[inline(always)]
    pub fn program_id(&self) -> &'ctx Address {
        self.inner.program_id
    }

    /// Raw instruction data, narrowed to the wrapper borrow lifetime.
    #[inline(always)]
    pub fn instruction_data(&self) -> &'ctx [u8] {
        self.inner.instruction_data
    }

    /// Get an account by index, narrowed to the wrapper borrow lifetime.
    #[inline(always)]
    pub fn account(&self, index: usize) -> Result<&'ctx AccountView<'a>, ProgramError> {
        self.inner
            .accounts
            .get(index)
            .ok_or(ProgramError::NotEnoughAccountKeys)
    }

    /// Mutation-intent account access, narrowed to the wrapper borrow lifetime.
    #[inline(always)]
    pub fn account_mut(&self, index: usize) -> Result<&'ctx AccountView<'a>, ProgramError> {
        self.account(index)
    }

    /// Get the total number of accounts.
    #[inline(always)]
    pub fn num_accounts(&self) -> usize {
        self.inner.num_accounts()
    }

    /// Get all accounts as a slice, narrowed to the wrapper borrow lifetime.
    #[inline(always)]
    pub fn accounts(&self) -> &'ctx [AccountView<'a>] {
        self.inner.accounts
    }

    /// Access the instruction-scoped segment borrow registry.
    #[inline(always)]
    pub fn borrows(&self) -> &SegmentBorrowRegistry {
        &self.inner.segment_borrows
    }

    /// Mutably access the instruction-scoped segment borrow registry.
    #[inline(always)]
    pub fn borrows_mut(&mut self) -> &mut SegmentBorrowRegistry {
        &mut self.inner.segment_borrows
    }

    /// Inspect the currently reachable account slice for duplicate aliases.
    #[inline(always)]
    pub fn audit_accounts(&self) -> AccountAudit<'ctx> {
        AccountAudit::new(self.inner.accounts)
    }

    /// Get the remaining accounts starting at `from`, narrowed to the wrapper
    /// borrow lifetime.
    #[inline(always)]
    pub fn remaining_accounts(&self, from: usize) -> &'ctx [AccountView<'a>] {
        if from >= self.inner.accounts.len() {
            &[]
        } else {
            &self.inner.accounts[from..]
        }
    }

    /// Get remaining accounts in strict duplicate-rejecting mode.
    #[inline(always)]
    pub fn remaining_accounts_strict(
        &self,
        from: usize,
    ) -> crate::remaining::RemainingAccounts<'ctx> {
        let declared_end = from.min(self.inner.accounts.len());
        crate::remaining::RemainingAccounts::strict(
            &self.inner.accounts[..declared_end],
            self.remaining_accounts(from),
        )
    }

    /// Get remaining accounts in duplicate-preserving passthrough mode.
    #[inline(always)]
    pub fn remaining_accounts_passthrough(
        &self,
        from: usize,
    ) -> crate::remaining::RemainingAccounts<'ctx> {
        let declared_end = from.min(self.inner.accounts.len());
        crate::remaining::RemainingAccounts::passthrough(
            &self.inner.accounts[..declared_end],
            self.remaining_accounts(from),
        )
    }

    /// Get remaining accounts in strict mode and bind a sequential typed parser.
    #[inline(always)]
    pub fn remaining_accounts_typed(&self, from: usize) -> crate::remaining::RemainingTyped<'ctx> {
        self.remaining_accounts_strict(from).typed()
    }

    /// Get remaining accounts in strict mode and bind a lazy indexed parser.
    #[inline(always)]
    pub fn remaining_accounts_lazy(&self, from: usize) -> crate::remaining::RemainingLazy<'ctx> {
        self.remaining_accounts_strict(from).lazy()
    }

    /// Require at least `n` accounts are present.
    #[inline(always)]
    pub fn require_accounts(&self, n: usize) -> ProgramResult {
        self.inner.require_accounts(n)
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
        self.inner.require_data_len(n)
    }

    /// Read instruction data as a typed value.
    #[inline(always)]
    pub fn read_data<T: crate::ValuePod>(&self, offset: usize) -> Result<T, ProgramError> {
        self.inner.read_data(offset)
    }

    /// Get a byte slice from instruction data.
    #[inline(always)]
    pub fn data_slice(&self, offset: usize, len: usize) -> Result<&'ctx [u8], ProgramError> {
        let end = offset
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        self.inner
            .instruction_data
            .get(offset..end)
            .ok_or(ProgramError::InvalidInstructionData)
    }
}
