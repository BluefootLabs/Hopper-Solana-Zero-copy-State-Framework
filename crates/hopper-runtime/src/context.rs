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
    /// Declared write-set enforced on every Context-mediated write
    /// acquire (innovation I12). `None` (the default) means no policy:
    /// writes are governed by the Sealevel `writable` flag and the
    /// borrow system alone, with zero added cost beyond one pointer
    /// compare per write acquire.
    write_policy: Option<&'static crate::write_policy::WritePolicy>,
}

impl<'a> Context<'a> {
    /// Create a new context from the entrypoint parameters.
    #[inline(always)]
    pub fn new(
        program_id: &'a Address,
        accounts: &'a [AccountView<'a>],
        instruction_data: &'a [u8],
    ) -> Self {
        // Start-of-instruction reset for the instruction-AMBIENT touch
        // log (it lives outside this struct so `AccountView`-level
        // borrows record with no Context in reach). On SBF this is
        // redundant with per-invocation heap zeroing; on hosts it is
        // what scopes the log to this instruction.
        #[cfg(feature = "touch-map")]
        crate::segment_borrow::touch_log::reset();
        Self {
            program_id,
            accounts,
            instruction_data,
            segment_borrows: SegmentBorrowRegistry::new(),
            write_policy: None,
        }
    }

    /// Install a declared write policy (innovation I12).
    ///
    /// From this point on, **every** Context-mediated write acquire —
    /// segment writes, whole-account `load_mut`, and the raw escape
    /// hatches `raw_mut` / `as_mut_ptr` — must be fully contained in one
    /// of the policy's declared ranges or it fails with
    /// `Custom(0xD000 | account_index)` before any byte is written.
    /// Whole-account paths claim `[0, data_len)`, so a policy that
    /// declares only field ranges forces handlers onto the declared
    /// segment accessors.
    ///
    /// `#[hopper::context(strict_writes)]` compiles the context's
    /// `mut` / `mut(seg, ...)` declarations into a `static` policy
    /// and installs it during `bind()`; calling this by hand is the raw
    /// equivalent. Direct substrate access on the raw
    /// [`AccountView`](crate::account::AccountView) (via
    /// [`account`](Self::account)) is outside the governed surface, like
    /// every other documented escape hatch.
    #[inline(always)]
    pub fn set_write_policy(&mut self, policy: &'static crate::write_policy::WritePolicy) {
        self.write_policy = Some(policy);
    }

    /// The installed write policy, if any.
    #[inline(always)]
    pub fn write_policy(&self) -> Option<&'static crate::write_policy::WritePolicy> {
        self.write_policy
    }

    /// Gate a proposed write acquire behind the installed policy.
    /// No policy installed = allowed (one branch on a `None`).
    #[inline(always)]
    fn check_write_policy(&self, index: usize, offset: u32, size: u32) -> ProgramResult {
        if let Some(policy) = self.write_policy {
            // Account indices are u8 on the wire; an index beyond 255
            // can never have been declared, so refuse it outright rather
            // than truncating into a potential false allow.
            if index > u8::MAX as usize {
                return Err(crate::write_policy::write_policy_violation(u8::MAX));
            }
            policy.check_write(index as u8, offset, size)?;
        }
        Ok(())
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

    /// Visit every distinct `(account, offset, size, R/W)` range this
    /// instruction has touched so far (`touch-map` feature, innovation
    /// I7). The log is cumulative — RAII lease releases do not remove
    /// records — so calling this at the end of a handler yields the
    /// instruction's segment-level footprint in first-touch order.
    /// Pair with [`touch_map_overflowed`](Self::touch_map_overflowed).
    #[cfg(feature = "touch-map")]
    #[inline]
    pub fn for_each_touch<F: FnMut(&crate::segment_borrow::SegmentBorrow)>(&self, f: F) {
        self.segment_borrows.for_each_touch(f)
    }

    /// Number of distinct touch records captured (`touch-map` feature).
    #[cfg(feature = "touch-map")]
    #[inline(always)]
    pub fn touch_map_len(&self) -> usize {
        self.segment_borrows.touch_map_len()
    }

    /// Whether the touch log overflowed and is partial (`touch-map`
    /// feature).
    #[cfg(feature = "touch-map")]
    #[inline(always)]
    pub fn touch_map_overflowed(&self) -> bool {
        self.segment_borrows.touch_map_overflowed()
    }

    /// Encode this instruction's touch map into the versioned v1 wire
    /// format (`touch-map` feature). Pure and allocation-free: returns
    /// the fixed-capacity buffer plus the number of valid bytes. The
    /// format is documented in [`crate::segment_borrow`] (magic `0x7A`,
    /// version `0x01`, flags, count, then 9-byte records).
    ///
    /// Each touched `(account, offset, size, R/W)` range is resolved to
    /// the account's slot index in this context's account list. A touch
    /// whose address is not among the instruction accounts (should be
    /// impossible — every touch originates from an account in this
    /// context) or whose slot exceeds `u8::MAX` is skipped and reported
    /// via flag bit1 rather than mis-attributed. Flag bit0 carries the
    /// touch log's overflow state so partial maps are honestly marked.
    #[cfg(feature = "touch-map")]
    pub fn encode_touch_map(
        &self,
    ) -> (
        [u8; crate::segment_borrow::TOUCH_MAP_MAX_ENCODED_LEN],
        usize,
    ) {
        use crate::segment_borrow::{AccessKind, TouchMapRecord, MAX_TOUCH_RECORDS};
        let empty = TouchMapRecord {
            slot: 0,
            offset: 0,
            size: 0,
            write: false,
        };
        let mut records = [empty; MAX_TOUCH_RECORDS];
        let mut n = 0usize;
        let mut skipped = false;
        self.segment_borrows.for_each_touch(|t| {
            let slot = self
                .accounts
                .iter()
                .position(|view| view.address().as_array() == t.key.as_array());
            match slot {
                // `n < MAX_TOUCH_RECORDS` always holds: the touch log and
                // the record array share the same capacity.
                Some(i) if i <= u8::MAX as usize && n < MAX_TOUCH_RECORDS => {
                    records[n] = TouchMapRecord {
                        slot: i as u8,
                        offset: t.offset,
                        size: t.size,
                        write: t.kind == AccessKind::Write,
                    };
                    n += 1;
                }
                _ => skipped = true,
            }
        });
        crate::segment_borrow::encode_touch_map(
            &records[..n],
            self.segment_borrows.touch_map_overflowed(),
            skipped,
        )
    }

    /// Emit this instruction's touch map as a single `sol_log_data`
    /// record (`touch-map` feature), making the transaction
    /// self-describing: `hopper tx explain` and the generated TypeScript
    /// `decodeHopperTouchMap` helper can reconstruct the instruction's
    /// field-level state effects from the signature alone.
    ///
    /// Call at the end of a handler, after the last state access — the
    /// touch log is cumulative, so this snapshots everything touched so
    /// far. Off-chain (`cfg(not(target_os = "solana"))`) the syscall is a
    /// no-op; use [`encode_touch_map`](Self::encode_touch_map) to test
    /// the encoded bytes.
    #[cfg(feature = "touch-map")]
    pub fn emit_touch_map(&self) {
        let (buf, len) = self.encode_touch_map();
        hopper_native::log::log_data(&[&buf[..len]]);
    }

    /// Opt-in post-handler epilogue: finalize a successful instruction by
    /// emitting its touch map, making the transaction self-describing
    /// (innovation I7).
    ///
    /// This is the single hook the `#[hopper::context(emit_touch_map)]`
    /// opt-in drives, so a developer gets the self-describing touch-map
    /// record on the golden path without hand-writing the `sol_log_data`
    /// syscall. The generated **dispatcher** — which alone sees the
    /// handler's `Result` — calls this on the handler's **Ok** path only,
    /// guarded by the context's `EMIT_TOUCH_MAP` const:
    ///
    /// ```ignore
    /// handler(Ctx::bind(&mut ctx)?, ..)?;      // Err short-circuits here
    /// if Ctx::EMIT_TOUCH_MAP { ctx.finish_with_touch_map(); }
    /// Ok(())
    /// ```
    ///
    /// It is deliberately NOT called from a `Drop` for the bound context:
    /// Rust runs drop glue on every scope exit — including `?`/`Err`
    /// returns — and a `Drop` cannot observe the handler's `Result`, so it
    /// would emit a misleading record advertising Write ranges for a
    /// failed, rolled-back instruction (adversarial review, CONFIRMED P2).
    /// Routing on the Ok path makes the record fire exclusively on
    /// success. It is also deliberately routed through a runtime helper
    /// (rather than a macro-emitted `#[cfg]`) so the **feature gate lives
    /// here**: the macro always emits the same call, and this method's two
    /// `cfg` bodies decide whether it does anything.
    ///
    /// With the `touch-map` feature **on** it forwards to
    /// [`emit_touch_map`](Self::emit_touch_map) — one `sol_log_data`
    /// record on-chain, a no-op off-chain. With the feature **off** the
    /// [zero-cost sibling](#method.finish_with_touch_map) is compiled
    /// instead, so the generated call emits nothing and costs nothing.
    #[cfg(feature = "touch-map")]
    #[inline]
    pub fn finish_with_touch_map(&self) {
        self.emit_touch_map();
    }

    /// Zero-cost sibling of
    /// [`finish_with_touch_map`](Self::finish_with_touch_map), compiled
    /// when the `touch-map` feature is off.
    ///
    /// Keeps the macro-generated opt-in epilogue call compiling on builds
    /// that never enabled the touch-map machinery, and emits nothing.
    /// This is what makes "opt-in present but feature off" produce no
    /// `sol_log_data` record and pay no compute for it.
    #[cfg(not(feature = "touch-map"))]
    #[inline(always)]
    pub fn finish_with_touch_map(&self) {}

    /// Get the remaining accounts starting at `from`.
    ///
    /// NOTE (binary size): the slicing below goes through `get(..)`, never
    /// `self.accounts[from..]`. A range index LLVM cannot statically bound
    /// emits `slice_end_index_len_fail`, which *formats* its arguments and
    /// links `Formatter::pad_integral`, `do_count_chars` and the integer
    /// `Display` impls — ~3.7 KiB of `core::fmt` — into every Hopper
    /// program's `.text`. These are `#[inline(always)]` hot-path helpers,
    /// so one panicking index here taxes every program. Keep them `get`-based.
    #[inline(always)]
    pub fn remaining_accounts(&self, from: usize) -> &'a [AccountView<'a>] {
        let accounts: &'a [AccountView<'a>] = self.accounts;
        accounts.get(from..).unwrap_or(&[])
    }

    /// Get remaining accounts in strict duplicate-rejecting mode.
    #[inline(always)]
    pub fn remaining_accounts_strict(
        &self,
        from: usize,
    ) -> crate::remaining::RemainingAccounts<'a> {
        let accounts: &'a [AccountView<'a>] = self.accounts;
        let declared_end = from.min(accounts.len());
        crate::remaining::RemainingAccounts::strict(
            accounts.get(..declared_end).unwrap_or(&[]),
            self.remaining_accounts(from),
        )
    }

    /// Get remaining accounts in duplicate-preserving passthrough mode.
    #[inline(always)]
    pub fn remaining_accounts_passthrough(
        &self,
        from: usize,
    ) -> crate::remaining::RemainingAccounts<'a> {
        let accounts: &'a [AccountView<'a>] = self.accounts;
        let declared_end = from.min(accounts.len());
        crate::remaining::RemainingAccounts::passthrough(
            accounts.get(..declared_end).unwrap_or(&[]),
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
    ///
    /// As a whole-account write borrow, this claims `[0, data_len)`:
    /// under an installed [write policy](Self::set_write_policy) it
    /// requires a whole-account allowance (a plain `mut` declaration),
    /// and with the `touch-map` feature it lands in the instruction
    /// touch map as a full-account write record.
    #[inline(always)]
    pub fn load_mut<T: LayoutContract + crate::Pod>(
        &mut self,
        index: usize,
    ) -> Result<crate::RefMut<'_, T>, ProgramError> {
        let view = self.account(index)?;
        let data_len = view.data_len() as u32;
        self.check_write_policy(index, 0, data_len)?;
        // The touch-map footprint records inside `try_borrow_mut` (the
        // choke point every mutable data borrow crosses), so this path
        // no longer stamps it explicitly — one source of truth.
        view.load_mut::<T>()
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
        let mut i = 0;
        while i < N {
            self.check_write_policy(index, ranges[i].0, ranges[i].1)?;
            i += 1;
        }
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
        self.check_write_policy(index, abs_offset, core::mem::size_of::<T>() as u32)?;
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
        self.check_write_policy(index, segment.offset, segment.size)?;
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
        self.check_write_policy(index, OFFSET, core::mem::size_of::<T>() as u32)?;
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
        // Whole-account write claim: an installed write policy gates the
        // raw path exactly like `load_mut` (coarse, never under-claims).
        self.check_write_policy(index, 0, view.data_len() as u32)?;
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
        // The untracked pointer is whole-account write capability, so an
        // installed write policy must have granted the whole account.
        self.check_write_policy(index, 0, view.data_len() as u32)?;
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

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod write_policy_tests {
    use super::*;
    use crate::write_policy::{WritePolicy, WriteRange};
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    const DATA_LEN: usize = 64;
    const BALANCE_OFF: u32 = 16;
    const NONCE_OFF: u32 = 24;

    fn make_account(address_byte: u8) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + DATA_LEN];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: `backing` is sized for the header plus DATA_LEN bytes and
        // outlives the returned view (the caller holds the Vec).
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
                data_len: DATA_LEN as u64,
            });
        }
        // SAFETY: `raw` points at a fully initialized RuntimeAccount with
        // its data region in the same allocation.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    // Field-granular policy on account 0: balance + nonce only.
    static FIELD_POLICY: WritePolicy = WritePolicy::new(&[
        WriteRange::new(0, BALANCE_OFF, 8),
        WriteRange::new(0, NONCE_OFF, 8),
    ]);
    // Whole-account allowance on account 0 (a plain `mut` declaration).
    static WHOLE_POLICY: WritePolicy = WritePolicy::new(&[WriteRange::whole_account(0)]);

    #[test]
    fn no_policy_leaves_every_write_path_open() {
        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);

        assert!(ctx.segment_mut::<[u8; 8]>(0, BALANCE_OFF).is_ok());
        assert!(ctx.segment_mut::<[u8; 4]>(0, 0).is_ok());
    }

    #[test]
    fn field_policy_allows_declared_segments_and_refuses_the_rest() {
        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);
        ctx.set_write_policy(&FIELD_POLICY);

        // Declared ranges work, including disjoint simultaneous writes.
        {
            let mut segs = ctx
                .split_segments_mut::<[u8; 8], 2>(0, [(BALANCE_OFF, 8), (NONCE_OFF, 8)])
                .unwrap();
            let [bal, nonce] = segs.all_mut();
            bal[0] = 1;
            nonce[0] = 2;
        }
        assert!(ctx.segment_mut::<[u8; 8]>(0, BALANCE_OFF).is_ok());

        // An undeclared range is refused with the indexed policy error —
        // before any borrow state changes, so reads still work after.
        assert_eq!(
            ctx.segment_mut::<[u8; 8]>(0, 0).unwrap_err(),
            crate::write_policy::write_policy_violation(0)
        );
        assert!(ctx.segment_ref::<[u8; 8]>(0, 0).is_ok());

        // A split where ONE range is undeclared is refused whole.
        assert!(ctx
            .split_segments_mut::<[u8; 8], 2>(0, [(BALANCE_OFF, 8), (0, 8)])
            .is_err());

        // Reads are never policy-gated.
        assert!(ctx.segment_ref::<[u8; 8]>(0, BALANCE_OFF).is_ok());
    }

    #[test]
    fn field_policy_refuses_whole_account_write_paths() {
        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);
        ctx.set_write_policy(&FIELD_POLICY);

        // The untracked whole-account pointer is whole-account write
        // capability: a field-granular policy must refuse it.
        // SAFETY: never dereferenced; testing the acquire-time gate only.
        assert_eq!(
            unsafe { ctx.as_mut_ptr(0) }.unwrap_err(),
            crate::write_policy::write_policy_violation(0)
        );
    }

    #[test]
    fn whole_account_allowance_admits_all_write_paths() {
        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);
        ctx.set_write_policy(&WHOLE_POLICY);

        assert!(ctx.segment_mut::<[u8; 8]>(0, BALANCE_OFF).is_ok());
        // SAFETY: never dereferenced; testing the acquire-time gate only.
        assert!(unsafe { ctx.as_mut_ptr(0) }.is_ok());
    }

    #[test]
    fn empty_policy_is_a_machine_checked_read_only_instruction() {
        static READ_ONLY: WritePolicy = WritePolicy::new(&[]);
        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);
        ctx.set_write_policy(&READ_ONLY);

        assert!(ctx.segment_mut::<[u8; 8]>(0, BALANCE_OFF).is_err());
        // SAFETY: never dereferenced; testing the acquire-time gate only.
        assert!(unsafe { ctx.as_mut_ptr(0) }.is_err());
        // Reads remain untouched.
        assert!(ctx.segment_ref::<[u8; 8]>(0, BALANCE_OFF).is_ok());
        assert!(ctx.as_ptr(0).is_ok());
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PolicyLayout {
        a: [u8; 8],
    }
    // SAFETY: repr(C), all-byte-array fields — every bit pattern valid,
    // no padding, align 1.
    unsafe impl crate::Zeroable for PolicyLayout {}
    // SAFETY: as above.
    unsafe impl crate::Pod for PolicyLayout {}
    impl crate::field_map::FieldMap for PolicyLayout {
        const FIELDS: &'static [crate::field_map::FieldInfo] = &[crate::field_map::FieldInfo::new(
            "a",
            crate::layout::HopperHeader::SIZE,
            8,
        )];
    }
    impl LayoutContract for PolicyLayout {
        const DISC: u8 = 77;
        const VERSION: u8 = 1;
        const LAYOUT_ID: [u8; 8] = [0x77; 8];
        const SIZE: usize = crate::layout::HopperHeader::SIZE + core::mem::size_of::<Self>();
    }

    #[test]
    fn load_mut_is_gated_before_header_validation_or_borrow() {
        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);
        ctx.set_write_policy(&FIELD_POLICY);

        // The whole-account claim `[0, data_len)` is refused by a
        // field-granular policy — with the policy error, not a layout
        // error, proving the gate runs before any borrow or header read.
        assert_eq!(
            ctx.load_mut::<PolicyLayout>(0).unwrap_err(),
            crate::write_policy::write_policy_violation(0)
        );

        // Under a whole-account allowance the gate passes; the account
        // has no valid Hopper header, so whatever happens next it is not
        // a policy refusal.
        let mut ctx2 = Context::new(&pid, &accounts, &[]);
        ctx2.set_write_policy(&WHOLE_POLICY);
        assert_ne!(
            ctx2.load_mut::<PolicyLayout>(0).unwrap_err(),
            crate::write_policy::write_policy_violation(0)
        );
    }

    #[cfg(feature = "touch-map")]
    #[test]
    fn whole_account_load_mut_lands_in_the_touch_map() {
        use crate::segment_borrow::AccessKind;

        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);

        // A segment write followed by a whole-account borrow recorded
        // through the same ledger: the I7 map now sees both shapes.
        drop(ctx.segment_mut::<[u8; 8]>(0, BALANCE_OFF).unwrap());
        {
            let view = ctx.account(0).unwrap();
            let data_len = view.data_len() as u32;
            let addr = *view.address();
            ctx.borrows_mut()
                .record_account_touch(&addr, data_len, AccessKind::Write);
        }

        let mut seen = std::vec::Vec::new();
        ctx.for_each_touch(|t| seen.push((t.offset, t.size, t.kind)));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], (BALANCE_OFF, 8, AccessKind::Write));
        assert_eq!(seen[1], (0, DATA_LEN as u32, AccessKind::Write));
    }

    #[cfg(feature = "touch-map")]
    #[test]
    fn touch_map_emission_round_trips_through_the_wire_format() {
        use crate::segment_borrow::{decode_touch_map_for_tests, AccessKind, TouchMapRecord};

        let (_b0, account0) = make_account(1);
        let (_b1, account1) = make_account(2);
        let accounts = [account0, account1];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);

        // A write and a read on account 1, a read on account 0, and a
        // whole-account write on account 0 — every touch shape.
        drop(ctx.segment_mut::<[u8; 8]>(1, BALANCE_OFF).unwrap());
        drop(ctx.segment_ref::<[u8; 8]>(1, NONCE_OFF).unwrap());
        drop(ctx.segment_ref::<[u8; 4]>(0, 0).unwrap());
        {
            let view = ctx.account(0).unwrap();
            let (data_len, addr) = (view.data_len() as u32, *view.address());
            ctx.borrows_mut()
                .record_account_touch(&addr, data_len, AccessKind::Write);
        }

        let (buf, len) = ctx.encode_touch_map();
        let (flags, records) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags, 0, "complete map must carry no overflow/skip flags");
        assert_eq!(
            records,
            std::vec![
                TouchMapRecord {
                    slot: 1,
                    offset: BALANCE_OFF,
                    size: 8,
                    write: true,
                },
                TouchMapRecord {
                    slot: 1,
                    offset: NONCE_OFF,
                    size: 8,
                    write: false,
                },
                TouchMapRecord {
                    slot: 0,
                    offset: 0,
                    size: 4,
                    write: false,
                },
                TouchMapRecord {
                    slot: 0,
                    offset: 0,
                    size: DATA_LEN as u32,
                    write: true,
                },
            ]
        );

        // Off-chain the syscall is a no-op; the call must still be safe.
        ctx.emit_touch_map();
    }

    #[cfg(feature = "touch-map")]
    #[test]
    fn touch_map_emission_marks_overflow_honestly() {
        use crate::segment_borrow::{decode_touch_map_for_tests, TOUCH_MAP_FLAG_OVERFLOWED};

        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);

        // Touch more distinct ranges than the log capacity.
        let addr = *ctx.account(0).unwrap().address();
        let mut i: u32 = 0;
        while (i as usize) < crate::segment_borrow::MAX_TOUCH_RECORDS + 3 {
            let b = ctx.borrows_mut().register_leased_read(&addr, i, 1).unwrap();
            ctx.borrows_mut().release(&b);
            i += 1;
        }
        assert!(ctx.touch_map_overflowed());

        let (buf, len) = ctx.encode_touch_map();
        let (flags, records) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags & TOUCH_MAP_FLAG_OVERFLOWED, TOUCH_MAP_FLAG_OVERFLOWED);
        assert_eq!(records.len(), crate::segment_borrow::MAX_TOUCH_RECORDS);
    }

    /// The opt-in epilogue helper the generated dispatcher calls on the
    /// handler's Ok path when the context declared
    /// `#[hopper::context(emit_touch_map)]`. With the `touch-map` feature
    /// on it must finalize the instruction exactly like a hand-written
    /// `emit_touch_map`: snapshot the cumulative touch log and hand the
    /// same wire bytes the encoder produces to `sol_log_data`. Off-chain
    /// the syscall is a no-op, so we prove the record is decodable via the
    /// shared encoder and that calling the helper is safe.
    #[cfg(feature = "touch-map")]
    #[test]
    fn finish_with_touch_map_finalizes_a_decodable_record() {
        use crate::segment_borrow::{decode_touch_map_for_tests, TouchMapRecord};

        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);

        // A bound handler with the opt-in touches state, then the
        // dispatcher calls `finish_with_touch_map()` on the Ok path.
        drop(ctx.segment_mut::<[u8; 8]>(0, BALANCE_OFF).unwrap());
        drop(ctx.segment_ref::<[u8; 8]>(0, NONCE_OFF).unwrap());

        // The record the epilogue emits is byte-identical to the encoder's
        // output (what `emit_touch_map` / `finish_with_touch_map` send).
        let (buf, len) = ctx.encode_touch_map();
        let (flags, records) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags, 0, "complete map carries no overflow/skip flags");
        assert_eq!(
            records,
            std::vec![
                TouchMapRecord {
                    slot: 0,
                    offset: BALANCE_OFF,
                    size: 8,
                    write: true,
                },
                TouchMapRecord {
                    slot: 0,
                    offset: NONCE_OFF,
                    size: 8,
                    write: false,
                },
            ]
        );

        // The helper the dispatcher calls on the Ok path: off-chain
        // no-op, must be safe and must not disturb the recorded footprint.
        ctx.finish_with_touch_map();
    }

    /// The mirror of the opt-in: a context that touched nothing (a handler
    /// that did no state access, or one whose context did NOT opt in and
    /// so whose dispatcher never calls the helper) has an empty footprint
    /// — the epilogue would emit a header-only record with zero touch
    /// entries. This pins "without the opt-in / without touches, produces
    /// none".
    #[cfg(feature = "touch-map")]
    #[test]
    fn untouched_context_finish_emits_no_touch_records() {
        use crate::segment_borrow::decode_touch_map_for_tests;

        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let ctx = Context::new(&pid, &accounts, &[]);

        assert_eq!(ctx.touch_map_len(), 0);
        let (buf, len) = ctx.encode_touch_map();
        let (flags, records) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags, 0, "empty map carries no flags");
        assert!(
            records.is_empty(),
            "no touches means no records: {records:?}"
        );

        // Safe to finalize even with nothing to report.
        ctx.finish_with_touch_map();
    }

    /// CONFIRMED P2 regression: the touch-map record must be emitted on
    /// the handler's **Ok** path ONLY, never on `Err`. This reconstructs
    /// the exact shape the dispatcher generates for an opted-in typed
    /// context —
    ///
    /// ```ignore
    /// handler(Ctx::bind(&mut ctx)?, ..)?;              // Err short-circuits
    /// if Ctx::EMIT_TOUCH_MAP { ctx.finish_with_touch_map(); }
    /// Ok(())
    /// ```
    ///
    /// — and drives it with a handler that returns `Err` and the same
    /// handler that returns `Ok`. The old `Drop`-based emit fired on both
    /// paths (drop glue runs on every scope exit) and so leaked a
    /// misleading record for the rolled-back instruction; the Ok-only
    /// dispatch emits nothing on `Err` and exactly one decodable record on
    /// `Ok`. Off-chain the real syscall is a no-op, so the finish point is
    /// made observable by snapshotting the same wire bytes
    /// `finish_with_touch_map` would send.
    #[cfg(feature = "touch-map")]
    #[test]
    fn dispatch_emits_touch_map_on_ok_path_only() {
        use crate::segment_borrow::decode_touch_map_for_tests;

        // The context opted in, so its `EMIT_TOUCH_MAP` const is `true`.
        const EMIT_TOUCH_MAP: bool = true;

        // Faithful reconstruction of the generated dispatch helper body.
        // `emitted` captures each record the Ok-path finish point would
        // send — its length is the number of touch-map records emitted.
        fn generated_dispatch(
            ctx: &mut Context<'_>,
            handler: impl FnOnce(&mut Context<'_>) -> ProgramResult,
            emitted: &mut std::vec::Vec<std::vec::Vec<u8>>,
        ) -> ProgramResult {
            // `handler(Ctx::bind(&mut ctx)?, ..)?` — an `Err` here
            // short-circuits before the emit below, exactly as `?` does
            // after a real bound handler returns.
            handler(ctx)?;
            // `if Ctx::EMIT_TOUCH_MAP { ctx.finish_with_touch_map(); }` —
            // reached only on the Ok path. Off-chain the syscall is a
            // no-op, so snapshot the identical bytes to observe the emit.
            if EMIT_TOUCH_MAP {
                let (buf, len) = ctx.encode_touch_map();
                emitted.push(buf[..len].to_vec());
                ctx.finish_with_touch_map();
            }
            Ok(())
        }

        // A handler that touches state, then fails (as `require!`/`?`
        // would). The touch log is populated, but the instruction rolls
        // back — so NO self-describing record may be emitted.
        let (_b, account) = make_account(1);
        let accounts = [account];
        let pid = Address::new([9u8; 32]);
        let mut ctx = Context::new(&pid, &accounts, &[]);
        let mut emitted = std::vec::Vec::new();
        let err = generated_dispatch(
            &mut ctx,
            |c| {
                drop(c.segment_mut::<[u8; 8]>(0, BALANCE_OFF).unwrap());
                Err(ProgramError::Custom(7))
            },
            &mut emitted,
        );
        assert_eq!(
            err,
            Err(ProgramError::Custom(7)),
            "handler error must propagate"
        );
        assert!(
            emitted.is_empty(),
            "a failed instruction must emit NO touch-map record: {emitted:?}",
        );

        // The same handler shape, now returning Ok, must emit exactly one
        // decodable record describing what it touched.
        let (_b2, account2) = make_account(1);
        let accounts2 = [account2];
        let mut ctx_ok = Context::new(&pid, &accounts2, &[]);
        let mut emitted_ok = std::vec::Vec::new();
        let ok = generated_dispatch(
            &mut ctx_ok,
            |c| {
                drop(c.segment_mut::<[u8; 8]>(0, BALANCE_OFF).unwrap());
                Ok(())
            },
            &mut emitted_ok,
        );
        assert_eq!(ok, Ok(()), "successful handler returns Ok");
        assert_eq!(
            emitted_ok.len(),
            1,
            "a successful instruction must emit exactly one touch-map record",
        );
        let (_flags, records) = decode_touch_map_for_tests(&emitted_ok[0]).unwrap();
        assert_eq!(
            records.len(),
            1,
            "the record must describe the one touched range"
        );
        assert_eq!(records[0].offset, BALANCE_OFF);
        assert!(records[0].write, "the touched range was a write");
    }
}
