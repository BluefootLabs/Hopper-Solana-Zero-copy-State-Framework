//! Declared write-sets enforced at borrow acquisition (innovation I12).
//!
//! Sealevel's account model stops at one bit of write granularity: the
//! transaction-level `writable` flag covers the *entire* account. This
//! module extends that to **byte-range granularity**: an instruction
//! declares the exact ranges it is allowed to write, and the
//! [`Context`](crate::context::Context) rejects any write borrow outside
//! the declared set *at acquisition time* — before a single byte moves.
//!
//! ## How it composes
//!
//! - `#[hopper::context(strict_writes)]` compiles the context's declared
//!   `mut` / `mut(seg, ...)` attributes into a `static` [`WritePolicy`]
//!   and installs it during `bind()`. Nothing is computed at runtime; the
//!   policy is a const slice scanned at each write acquire.
//! - Every Context-mediated write path is gated: segment writes
//!   (`segment_mut`, `segment_mut_const`, `segment_mut_typed`,
//!   `split_segments_mut`), whole-account typed loads (`load_mut`), and
//!   the raw escape hatches (`raw_mut`, `as_mut_ptr`).
//! - A whole-account borrow claims `[0, data_len)`, so under a policy that
//!   declares only field ranges, `load_mut` / `as_mut_ptr` are refused and
//!   the handler must use the declared segment accessors. That is the
//!   discipline the policy exists to enforce.
//! - Paired with the `touch-map` feature (I7), the declared set can be
//!   compared against the *actual* footprint in tests: declared-vs-actual
//!   write verification with no instrumentation in the program itself.
//!
//! ## The lamport dimension (BLD-MUT)
//!
//! Byte ranges cover **data**; Sealevel writability also covers
//! **lamport** mutation (close, top-up, transfer). A policy built with
//! [`WritePolicy::with_lamports`] declares that second dimension: the
//! listed account indices may have their lamports mutated, every other
//! account's lamport mutation is refused. Because lamport operations
//! flow through `AccountView` (not `Context`), enforcement uses an
//! instruction-scoped ambient gate ([`try_install_lamport_gate`])
//! consulted by the runtime's lamport choke points: the
//! `native_boundary` `try_set_lamports`/`close` funnel (which every
//! runtime and `hopper-core` lifecycle lamport path crosses) and the
//! validated CPI tiers' writable-meta construction (a writable CPI
//! hand-off is unbounded delegation of both dimensions).
//!
//! The gate stores address **values** copied at install time — never a
//! pointer into the account slice — and checks compare the address of
//! the live view being mutated against those values. A leaked guard
//! (`mem::forget`) therefore leaves an observable *stale value policy*
//! installed (fail-closed for unknown addresses) rather than any form
//! of memory unsafety; see the gate section below for the full
//! contract, including the loud fail-closed install errors
//! (`0xD1__` page).
//!
//! ## Enforcement boundary
//!
//! The policy governs access **through `Context`** (data ranges) and
//! through the runtime's `AccountView`/CPI surface (lamports) — which
//! is every path `#[hopper::context]`-generated code and the supported
//! safe APIs use. Direct substrate access (`hopper_native` calls such
//! as `batch::transfer_lamports`, or `try_borrow_mut` on the raw
//! backend view) and the `unsafe` unchecked CPI tier are outside the
//! governed surface, exactly like the documented raw-pointer escape
//! hatches; they are visible in review and lintable. The Sealevel
//! `writable` flag is still enforced underneath in all cases.
//!
//! For the common no-CPI lamport move the escape hatch has a
//! first-class **gated** alternative:
//! [`crate::lamports::transfer_lamports`] (re-exported at the crate
//! root and through `hopper::prelude`; `lamports(...)` contexts also
//! expose it as a generated `ctx.transfer_lamports(..)` method) runs
//! the substrate helper's exact arithmetic through the
//! `native_boundary` funnel, checking **both** sides against the gate
//! before any balance changes — so gated programs keep the
//! mutation-complete guarantee without paying for a System CPI.
//!
//! [`AccountView`]: crate::account::AccountView

use crate::address::Address;
use crate::error::ProgramError;
use crate::ProgramResult;

mod exact_cell_selector_sealed {
    pub trait Sealed {}

    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for u32 {}
}

/// A selector whose runtime value and wire representation are guaranteed to
/// agree with Hopper's exact-cell Effect ABI.
///
/// The trait is sealed: aliases of the real unsigned primitives inherit the
/// implementation, while signed integers and user-defined lookalikes cannot
/// opt themselves in. Macro-generated code also checks [`WIRE_SIZE`](Self::WIRE_SIZE)
/// against the authored type spelling, catching primitive-shadowing aliases
/// such as a local `type u16 = u8` before a manifest can publish the wrong
/// decoder width.
pub trait ExactCellSelector: exact_cell_selector_sealed::Sealed + Copy {
    /// Canonical fixed width of the selector on the instruction wire.
    const WIRE_SIZE: u16;

    /// Convert to the compact value consumed by the write-policy gate.
    fn to_u32(self) -> u32;
}

impl ExactCellSelector for u8 {
    const WIRE_SIZE: u16 = 1;

    #[inline(always)]
    fn to_u32(self) -> u32 {
        self as u32
    }
}

impl ExactCellSelector for u16 {
    const WIRE_SIZE: u16 = 2;

    #[inline(always)]
    fn to_u32(self) -> u32 {
        self as u32
    }
}

impl ExactCellSelector for u32 {
    const WIRE_SIZE: u16 = 4;

    #[inline(always)]
    fn to_u32(self) -> u32 {
        self
    }
}

/// Error page for write-policy violations.
///
/// A rejected write on account index `i` surfaces as
/// `ProgramError::Custom(0xD000 | i)`, mirroring the `0xC000 | i`
/// convention used for declarative constraint failures. The account
/// index in the low byte makes the offending account recoverable from
/// the bare error code in logs and explorers.
pub const WRITE_POLICY_VIOLATION_PAGE: u32 = 0xD0_00;

/// Build the write-policy-violation error for an account index.
#[inline(always)]
pub const fn write_policy_violation(account_index: u8) -> ProgramError {
    ProgramError::Custom(WRITE_POLICY_VIOLATION_PAGE | account_index as u32)
}

/// One allowed write range on one instruction account.
///
/// `account_index` is the position in the instruction's account list
/// (the same index handed to [`Context::account`](crate::context::Context::account)).
/// Offsets are absolute within the account data, including any layout
/// header bytes — the same convention the segment access primitives use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteRange {
    /// Instruction account-list index this range applies to.
    pub account_index: u8,
    /// Absolute byte offset of the allowed range.
    pub offset: u32,
    /// Byte size of the allowed range. [`WriteRange::whole_account`]
    /// uses `u32::MAX`, which contains any request on the account
    /// (account data is capped far below 4 GiB).
    pub size: u32,
}

/// A runtime-selected fixed-size cell inside a statically declared column.
///
/// The static [`WriteRange`] remains the conservative envelope used by
/// tooling. This descriptor narrows that envelope for one invocation to
/// `base_offset + args[argument_index] * stride .. + cell_size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParametricWriteRange {
    /// Instruction account-list index this column belongs to.
    pub account_index: u8,
    /// Absolute offset of element zero.
    pub base_offset: u32,
    /// Byte distance between consecutive elements.
    pub stride: u32,
    /// Writable bytes in the selected element.
    pub cell_size: u32,
    /// Number of cells in the column.
    pub count: u32,
    /// Index into the invocation's bound `u32` policy arguments.
    pub argument_index: u8,
    /// Stable instruction-argument name for manifests and diagnostics.
    pub argument_name: &'static str,
    /// Stable state segment/field name for manifests and diagnostics.
    pub segment_name: &'static str,
}

impl ParametricWriteRange {
    /// Build one parametric column rule.
    #[inline(always)]
    pub const fn new(
        account_index: u8,
        base_offset: u32,
        stride: u32,
        cell_size: u32,
        count: u32,
        argument_index: u8,
        argument_name: &'static str,
        segment_name: &'static str,
    ) -> Self {
        Self {
            account_index,
            base_offset,
            stride,
            cell_size,
            count,
            argument_index,
            argument_name,
            segment_name,
        }
    }

    #[inline(always)]
    fn envelope_overlaps(&self, offset: u32, size: u32) -> bool {
        let envelope_start = self.base_offset as u64;
        let envelope_end = envelope_start
            + self.stride as u64 * self.count.saturating_sub(1) as u64
            + self.cell_size as u64;
        let request_start = offset as u64;
        let request_end = request_start + size as u64;
        request_start < envelope_end && request_end > envelope_start
    }

    #[inline(always)]
    fn selected_contains(&self, selected: u32, offset: u32, size: u32) -> bool {
        if selected >= self.count {
            return false;
        }
        let start = self.base_offset as u64 + self.stride as u64 * selected as u64;
        let end = start + self.cell_size as u64;
        let request_start = offset as u64;
        let request_end = request_start + size as u64;
        request_start >= start && request_end <= end
    }
}

impl WriteRange {
    /// Allow writes to `[offset, offset + size)` on `account_index`.
    #[inline(always)]
    pub const fn new(account_index: u8, offset: u32, size: u32) -> Self {
        Self {
            account_index,
            offset,
            size,
        }
    }

    /// Allow whole-account writes on `account_index` (what a plain
    /// `mut` declaration grants).
    #[inline(always)]
    pub const fn whole_account(account_index: u8) -> Self {
        Self {
            account_index,
            offset: 0,
            size: u32::MAX,
        }
    }

    /// Allow writes to the **open-ended tail** `[offset, +inf)` on
    /// `account_index` — the grant a growable `Seq<T>` tail needs.
    ///
    /// `size` is `u32::MAX`, so [`contains`](Self::contains) admits any
    /// sub-range starting at or after `offset` regardless of how large the
    /// account grows (the account data cap is far below 4 GiB, and
    /// `contains` widens to `u64` so `offset + u32::MAX` cannot wrap).
    ///
    /// Crucially this is **not** a whole-account grant when `offset != 0`:
    /// [`allows_whole_account_write`](WritePolicy::allows_whole_account_write)
    /// requires a range containing `[0, u32::MAX)`, and a tail range that
    /// starts past the fixed head fails that test. So the fixed head stays
    /// byte-protected and CPI writable-meta delegation stays refused, while
    /// the tail region past `offset` remains freely writable and growable.
    #[inline(always)]
    pub const fn tail_from(account_index: u8, offset: u32) -> Self {
        Self {
            account_index,
            offset,
            size: u32::MAX,
        }
    }

    /// Whether `[offset, offset + size)` is fully contained in this
    /// range. Widened to `u64` so `offset + size` cannot wrap.
    #[inline(always)]
    pub const fn contains(&self, offset: u32, size: u32) -> bool {
        let req_start = offset as u64;
        let req_end = offset as u64 + size as u64;
        let start = self.offset as u64;
        let end = self.offset as u64 + self.size as u64;
        req_start >= start && req_end <= end
    }
}

/// The lamport-write dimension of a [`WritePolicy`] (BLD-MUT).
///
/// A [`WriteRange`] covers **data** bytes; Sealevel writability also
/// covers **lamport** credits/debits (close, top-up, transfer). This
/// enum records whether the instruction declared that second dimension:
///
/// - [`Undeclared`](Self::Undeclared): the policy says nothing about
///   lamports. Lamport mutation stays ungoverned (today's behavior for
///   every pre-existing `strict_writes` context), and the write set is
///   **not** mutation-complete.
/// - [`Declared`](Self::Declared): only the listed account indices may
///   have their lamports mutated through the runtime's lamport choke
///   points; every other account's lamport mutation is refused with the
///   same `Custom(0xD000 | index)` error the data-range gate uses. An
///   **empty** list is valid and means "no lamport mutation anywhere".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LamportPolicy {
    /// Lamport writes undeclared: passthrough, not mutation-complete.
    Undeclared,
    /// Lamport writes permitted only on the listed account indices.
    Declared(&'static [u8]),
}

/// Declared write-set for one instruction.
///
/// Intended to be a `static` built at macro-expansion time from the
/// context's `mut` / `mut(seg, ...)` declarations and installed via
/// [`Context::set_write_policy`](crate::context::Context::set_write_policy).
/// An **empty** set is a valid policy: it denies every Context-mediated
/// write, turning the instruction into a machine-checked read-only
/// contract.
///
/// The optional [`lamports`](Self::lamports) dimension extends the set
/// from data bytes to lamport balances; see [`LamportPolicy`].
#[derive(Debug)]
pub struct WritePolicy {
    /// Allowed write ranges. Scanned linearly; contexts declare a
    /// handful of ranges, so a bounded scan beats any lookup structure
    /// at Solana scale.
    pub allows: &'static [WriteRange],
    /// Invocation-parametric rules that narrow selected static envelopes.
    pub parametric: &'static [ParametricWriteRange],
    /// Declared lamport-write permission set (BLD-MUT). See
    /// [`LamportPolicy`] for the exact semantics of each variant.
    pub lamports: LamportPolicy,
}

impl WritePolicy {
    /// Wrap a const slice of allowed ranges as a policy.
    ///
    /// The lamport dimension is left [`LamportPolicy::Undeclared`], which
    /// preserves the pre-BLD-MUT contract exactly: lamport mutation is
    /// ungoverned and the policy is not mutation-complete.
    #[inline(always)]
    pub const fn new(allows: &'static [WriteRange]) -> Self {
        Self {
            allows,
            parametric: &[],
            lamports: LamportPolicy::Undeclared,
        }
    }

    /// Build a policy that declares **both** dimensions: data ranges and
    /// the account indices allowed to have their lamports mutated.
    #[inline(always)]
    pub const fn with_lamports(
        allows: &'static [WriteRange],
        lamport_accounts: &'static [u8],
    ) -> Self {
        Self {
            allows,
            parametric: &[],
            lamports: LamportPolicy::Declared(lamport_accounts),
        }
    }

    /// Build a data policy with invocation-parametric cell narrowing.
    #[inline(always)]
    pub const fn with_parametric(
        allows: &'static [WriteRange],
        parametric: &'static [ParametricWriteRange],
    ) -> Self {
        Self {
            allows,
            parametric,
            lamports: LamportPolicy::Undeclared,
        }
    }

    /// Build a mutation-complete policy with parametric cell narrowing.
    #[inline(always)]
    pub const fn with_parametric_and_lamports(
        allows: &'static [WriteRange],
        parametric: &'static [ParametricWriteRange],
        lamport_accounts: &'static [u8],
    ) -> Self {
        Self {
            allows,
            parametric,
            lamports: LamportPolicy::Declared(lamport_accounts),
        }
    }

    /// Whether the lamport dimension was declared (making the policy a
    /// mutation-complete write-set when installed by a `strict_writes`
    /// context).
    #[inline(always)]
    pub const fn lamports_declared(&self) -> bool {
        matches!(self.lamports, LamportPolicy::Declared(_))
    }

    /// Whether the policy permits mutating `account_index`'s lamports.
    ///
    /// [`LamportPolicy::Undeclared`] permits everything (the dimension
    /// carries no authority); a declared set permits only its members.
    #[inline(always)]
    pub fn allows_lamport_mutation(&self, account_index: u8) -> bool {
        match self.lamports {
            LamportPolicy::Undeclared => true,
            LamportPolicy::Declared(indices) => {
                let mut i = 0;
                while i < indices.len() {
                    if indices[i] == account_index {
                        return true;
                    }
                    i += 1;
                }
                false
            }
        }
    }

    /// Whether a declared range grants **whole-account** data writes on
    /// `account_index` (what a plain `mut` / lifecycle declaration
    /// compiles to). Used by the CPI writable-meta gate: handing an
    /// account writable to a callee is unbounded data delegation, so it
    /// requires a whole-account grant, not just field ranges.
    #[inline(always)]
    pub fn allows_whole_account_write(&self, account_index: u8) -> bool {
        let ranges = self.allows;
        let mut i = 0;
        while i < ranges.len() {
            let r = &ranges[i];
            if r.account_index == account_index && r.contains(0, u32::MAX) {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Whether this policy grants any data-write authority on
    /// `account_index`.
    ///
    /// Length/presence transitions cannot be represented as an ordinary
    /// byte range because the affected bytes may not exist yet.  The ambient
    /// gate therefore treats a declared data range as the account-level
    /// authority required to resize that account, while still requiring a
    /// whole-account grant for raw full-buffer writes and writable CPI
    /// delegation.
    #[inline(always)]
    pub fn allows_any_account_write(&self, account_index: u8) -> bool {
        let mut i = 0;
        while i < self.allows.len() {
            if self.allows[i].account_index == account_index {
                return true;
            }
            i += 1;
        }
        false
    }

    /// `Ok(())` iff `[offset, offset + size)` on `account_index` is
    /// fully contained in a **single** declared range. Adjacent declared
    /// ranges are not coalesced at check time; the macro emits ranges
    /// exactly as declared, so a request straddling two declarations is
    /// refused (declare a covering range if that access is intended).
    #[inline(always)]
    pub fn check_write(
        &self,
        account_index: u8,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        let ranges = self.allows;
        let mut i = 0;
        while i < ranges.len() {
            let r = &ranges[i];
            if r.account_index == account_index && r.contains(offset, size) {
                return Ok(());
            }
            i += 1;
        }
        Err(write_policy_violation(account_index))
    }

    /// Parametric form of [`check_write`](Self::check_write). A request that
    /// touches a governed column must fit the invocation-selected cell; other
    /// requests fall back to the ordinary static policy.
    #[inline(always)]
    pub fn check_write_with_args(
        &self,
        account_index: u8,
        offset: u32,
        size: u32,
        args: &[u32],
    ) -> Result<(), ProgramError> {
        let mut i = 0;
        while i < self.parametric.len() {
            let rule = &self.parametric[i];
            if rule.account_index == account_index && rule.envelope_overlaps(offset, size) {
                let argument = args.get(rule.argument_index as usize).copied();
                if argument
                    .map(|selected| rule.selected_contains(selected, offset, size))
                    .unwrap_or(false)
                {
                    return Ok(());
                }
                return Err(write_policy_violation(account_index));
            }
            i += 1;
        }
        self.check_write(account_index, offset, size)
    }

    /// Return the first byte in a recorded write touch that is not authorized
    /// by this invocation's effective policy.
    ///
    /// Unlike [`check_write_with_args`](Self::check_write_with_args), this
    /// method checks the **union** of authorized ranges. The distinction is
    /// intentional: a write acquire must fit one declaration, but the touch
    /// ledger may coalesce adjacent, independently authorized acquires into a
    /// single record. Static ranges authorize bytes outside parametric
    /// envelopes; inside an envelope, only the invocation-selected cell is
    /// authorized. Missing or out-of-range selector values therefore fail
    /// closed at the first governed byte.
    #[inline]
    pub fn first_unauthorized_byte_with_args(
        &self,
        account_index: u8,
        offset: u32,
        size: u32,
        args: &[u32],
    ) -> Option<u64> {
        let mut cursor = offset as u64;
        let request_end = cursor + size as u64;

        while cursor < request_end {
            // A parametric envelope replaces the static authority over its
            // bytes. Rules generated by the context macro are disjoint; for
            // hand-built overlapping rules, preserve the runtime gate's
            // first-matching-rule behavior.
            let mut governing_rule = None;
            let mut i = 0;
            while i < self.parametric.len() {
                let rule = &self.parametric[i];
                if rule.account_index == account_index {
                    let envelope_start = rule.base_offset as u64;
                    let envelope_end = envelope_start
                        + rule.stride as u64 * rule.count.saturating_sub(1) as u64
                        + rule.cell_size as u64;
                    if envelope_start <= cursor && cursor < envelope_end {
                        governing_rule = Some(rule);
                        break;
                    }
                }
                i += 1;
            }

            if let Some(rule) = governing_rule {
                let Some(selected) = args.get(rule.argument_index as usize).copied() else {
                    return Some(cursor);
                };
                if selected >= rule.count {
                    return Some(cursor);
                }
                let selected_start = rule.base_offset as u64 + rule.stride as u64 * selected as u64;
                let selected_end = selected_start + rule.cell_size as u64;
                if selected_start <= cursor && cursor < selected_end {
                    cursor = core::cmp::min(selected_end, request_end);
                    continue;
                }
                return Some(cursor);
            }

            // Outside a governed envelope, advance through the union of all
            // static ranges covering the cursor. Use the furthest end so the
            // result is independent of declaration order.
            let mut covered_until = cursor;
            let mut j = 0;
            while j < self.allows.len() {
                let range = &self.allows[j];
                if range.account_index == account_index {
                    let range_start = range.offset as u64;
                    let range_end = range_start + range.size as u64;
                    if range_start <= cursor && cursor < range_end {
                        covered_until = core::cmp::max(covered_until, range_end);
                    }
                }
                j += 1;
            }
            if covered_until == cursor {
                return Some(cursor);
            }

            // Do not jump across an envelope hidden inside a broad static
            // range: its start is where parametric authority takes over.
            let mut k = 0;
            while k < self.parametric.len() {
                let rule = &self.parametric[k];
                let envelope_start = rule.base_offset as u64;
                if rule.account_index == account_index
                    && cursor < envelope_start
                    && envelope_start < covered_until
                {
                    covered_until = envelope_start;
                }
                k += 1;
            }
            cursor = core::cmp::min(covered_until, request_end);
        }

        None
    }

    /// Non-erroring form of [`check_write`](Self::check_write).
    #[inline(always)]
    pub fn allows_write(&self, account_index: u8, offset: u32, size: u32) -> bool {
        self.check_write(account_index, offset, size).is_ok()
    }
}

// ── Lamport gate (BLD-MUT) ───────────────────────────────────────────
//
// The data dimension is enforced *inside* `Context`, which sees the
// account index on every write acquire. Lamport mutation, by contrast,
// happens through `AccountView`-level operations (`try_set_lamports`,
// `close`, the CPI writable hand-off) that never see the `Context`.
// The gate bridges that: `bind()` on a `strict_writes` context whose
// policy declares the lamport dimension installs an instruction-scoped
// ambient record derived from `(account slice, policy)`, and the
// runtime's lamport choke points — `native_boundary::try_set_lamports`,
// `native_boundary::close`, and the validated CPI tiers' writable-meta
// construction — consult it before mutating.
//
// ## Value-based storage (nothing ambient is ever dereferenced)
//
// Storage mirrors `borrow_registry`: the ambient record holds address
// VALUES copied out of the account slice at install time (plus the
// per-address permission bits precomputed from the policy), never a
// pointer into the slice. `check_*` compares the address the caller
// reads from the live view it is about to mutate against those stored
// values. Because nothing in the store is pointer-shaped, a leaked
// guard (`mem::forget`, a forgotten bound context) degrades to a
// **stale value policy**: the stale gate keeps governing later checks
// on its tier — observable over-/stale enforcement that fails closed
// for unknown addresses — until the tier's slots run out, at which
// point further installs fail loudly. It can never become a dangling
// read. (An earlier design stored a raw pointer to the account slice
// and claimed the guard's lifetime bounded every ambient read; safe
// `mem::forget` skips `Drop` and voided that claim, which is exactly
// why values are stored instead.)
//
// Matching by address value means duplicate-meta accounts (same
// address ⇒ same underlying account, as the loader hands duplicate
// metas one `RuntimeAccount`) share one merged permission entry; the
// permission is a property of the account, not of the meta position,
// so an OR-merge across duplicate indices preserves the old
// pointer-identity semantics. Each merged entry remembers the first
// account index carrying the address so refusals keep the indexed
// `Custom(0xD000 | index)` error shape.
//
// ## Slot store (no prev-chain)
//
// Each tier keeps a small fixed array of slots plus a monotonically
// increasing token counter. Install claims a free slot under a unique
// token; `Drop` clears exactly the slot holding its own token (scan +
// match) and nothing else; checks consult the still-active slot with
// the HIGHEST token — the most recently installed gate — so nested
// binds shadow outer gates while alive and dropping guards in any
// order can only ever free the dropper's own slot, never corrupt or
// resurrect another gate. When no slot is free, install FAILS CLOSED
// with a loud error instead of truncating, evicting, or silently
// sharing.

/// Error page for lamport-gate **installation** failures (`0xD1__`).
///
/// Deliberately distinct from the `0xD0__` per-account violation page so
/// an install failure can never be mistaken for a policy refusal on a
/// specific account index.
pub const LAMPORT_GATE_INSTALL_ERROR_PAGE: u32 = 0xD1_00;

/// Install refused: the instruction's account slice has more accounts
/// than [`LAMPORT_GATE_CAPACITY`]. The gate refuses loudly rather than
/// silently truncating the governed set (a truncated gate would treat
/// the overflow accounts as foreign — surprising, and wrong the moment
/// one of them was declared).
pub const LAMPORT_GATE_TOO_MANY_ACCOUNTS: ProgramError =
    ProgramError::Custom(LAMPORT_GATE_INSTALL_ERROR_PAGE | 0x01);

/// Install refused: all [`LAMPORT_GATE_DEPTH`] slots on this tier are
/// occupied by still-active (or leaked) gates. Nesting deeper than the
/// Solana invoke-depth budget, or leaking guards via `mem::forget`,
/// exhausts the store; the install fails closed instead of evicting a
/// live gate.
pub const LAMPORT_GATE_DEPTH_EXCEEDED: ProgramError =
    ProgramError::Custom(LAMPORT_GATE_INSTALL_ERROR_PAGE | 0x02);

/// Install refused (host fallback tier only): another still-active gate
/// occupies the process-global single-slot store. On `no_std`
/// multi-threaded hosts without the `thread-local-registry` feature the
/// gate cannot attribute nesting to a thread, so a concurrent second
/// install is refused loudly — never silently shared with, or allowed
/// to corrupt, the gate another thread installed. Enable
/// `thread-local-registry` (or run gated instructions one at a time)
/// to lift this.
pub const LAMPORT_GATE_CONTENDED: ProgramError =
    ProgramError::Custom(LAMPORT_GATE_INSTALL_ERROR_PAGE | 0x03);

/// Install refused because invocation-resolved exact-cell selectors exceed
/// the bounded ambient ABI.
pub const AMBIENT_GATE_TOO_MANY_ARGUMENTS: ProgramError =
    ProgramError::Custom(LAMPORT_GATE_INSTALL_ERROR_PAGE | 0x04);

/// Install refused: this build carries the `unguarded-raw-surfaces` size
/// opt-out, but the policy declares data write ranges (fixed or
/// parametric) — governance the opt-out build cannot enforce on the raw
/// `AccountView` surfaces. Refusing at install keeps the bypass loud on
/// EVERY tier: macro-bound strict contexts are already a compile error in
/// such builds, and this is the runtime fence for hand-rolled installs.
/// Lamports-only policies still install (their dimensions stay enforced).
pub const AMBIENT_GATE_UNGUARDED_BUILD: ProgramError =
    ProgramError::Custom(LAMPORT_GATE_INSTALL_ERROR_PAGE | 0x05);

/// Per-gate account capacity: the runtime's transaction account bound.
/// An instruction can never carry more accounts than the transaction
/// that contains it, so a gate over one instruction's slice always
/// fits; anything larger is refused loudly at install
/// ([`LAMPORT_GATE_TOO_MANY_ACCOUNTS`]).
pub const LAMPORT_GATE_CAPACITY: usize = crate::MAX_TX_ACCOUNTS;

/// Maximum number of invocation-resolved selector values retained by one
/// ambient gate. This is the same bounded ABI used by [`Context`](crate::Context).
pub const AMBIENT_GATE_ARG_CAPACITY: usize = 8;

/// Concurrent gates per tier: Solana's nested-CPI depth budget (the
/// runtime caps the instruction stack at 5 = one top-level + 4 nested
/// CPI levels). On-chain every CPI level runs in a fresh VM whose
/// writable data is re-initialized, so a single store only ever sees
/// the nested binds of one handler frame; on host, hopper's CPI
/// syscall is a no-op (no nested handler dispatch), so concurrent
/// gates arise only from manually nested binds. Four slots cover both
/// with room to spare, and exhaustion fails closed loudly
/// ([`LAMPORT_GATE_DEPTH_EXCEEDED`]) rather than corrupting a live
/// gate.
pub const LAMPORT_GATE_DEPTH: usize = 4;

/// Why a gate installation was refused (tier-independent). Mapped to a
/// loud [`ProgramError`] by the install entry points; keeping the two
/// cases distinct lets each tier name its own no-free-slot failure
/// (nesting depth on the per-thread tiers vs. single-slot contention on
/// the fallback tier).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateInstallError {
    /// More accounts than [`LAMPORT_GATE_CAPACITY`].
    TooManyAccounts,
    /// More invocation selector values than
    /// [`AMBIENT_GATE_ARG_CAPACITY`].
    TooManyArguments,
    /// Every slot on the tier is occupied.
    NoFreeSlot,
}

/// One gated account: the address VALUE copied at install time plus the
/// permission bits precomputed from the installed policy. Contains no
/// pointers and is never dereferenced — only compared.
#[derive(Clone, Copy)]
struct GateEntry {
    /// Address value copied from the account slice at install time.
    address: Address,
    /// First instruction account index carrying this address (for the
    /// indexed `Custom(0xD000 | index)` refusal error).
    index: u8,
    /// OR over all indices with this address of
    /// `policy.allows_lamport_mutation(index)`.
    allow_mutation: bool,
    /// OR over all indices with this address of `allows_lamport_mutation
    /// && allows_whole_account_write` (the CPI writable hand-off needs
    /// BOTH dimensions on one declared index).
    allow_delegation: bool,
    /// Whether the account has any declared data authority. Account-length
    /// transitions are account-level effects whose newly exposed bytes do
    /// not yet have an ordinary range, so this is the bounded transition
    /// capability.
    #[cfg_attr(feature = "unguarded-raw-surfaces", allow(dead_code))]
    allow_transition: bool,
}

impl GateEntry {
    const EMPTY: Self = Self {
        address: Address::new([0; 32]),
        index: 0,
        allow_mutation: false,
        allow_delegation: false,
        allow_transition: false,
    };
}

/// One installed gate. `token == 0` means the slot is free.
#[derive(Clone, Copy)]
struct GateSlot {
    /// Unique install token (0 = free slot).
    token: u64,
    /// Number of initialized entries (distinct addresses).
    len: usize,
    /// Static policy installed by generated code. The reference is safe even
    /// if a guard is leaked: policies have program lifetime and never borrow
    /// the instruction's account slice.
    policy: Option<&'static WritePolicy>,
    /// Invocation-decoded selectors used by parametric exact-cell rules.
    args: [u32; AMBIENT_GATE_ARG_CAPACITY],
    args_len: usize,
    /// Copied `(address, permissions)` values; only `entries[..len]`
    /// are meaningful.
    entries: [GateEntry; LAMPORT_GATE_CAPACITY],
}

impl GateSlot {
    const FREE: Self = Self {
        token: 0,
        len: 0,
        policy: None,
        args: [0; AMBIENT_GATE_ARG_CAPACITY],
        args_len: 0,
        entries: [GateEntry::EMPTY; LAMPORT_GATE_CAPACITY],
    };
}

#[derive(Clone, Copy)]
enum GateCheck {
    Lamports,
    // Constructed only by the raw-surface guard wrappers; compiled out
    // under the `unguarded-raw-surfaces` size opt-out so the variants do
    // not read as dead code there.
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    Data {
        offset: u32,
        size: u32,
    },
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    Transition,
    Delegation,
}

/// Fixed-slot, value-only gate store. `DEPTH` is the number of
/// concurrently installed gates the tier supports (see
/// [`LAMPORT_GATE_DEPTH`]; the host fallback tier uses 1).
struct GateStore<const DEPTH: usize> {
    /// Count of tokens issued so far. The Nth install receives token `N`
    /// (see [`GateStore::install`]), so `0` means "none issued yet" and
    /// `0` remains the free-slot sentinel no live slot can hold.
    ///
    /// **This field must stay zero in [`GateStore::new`].** The SBF tier
    /// holds this store in a `static mut`; an all-zero initializer lets
    /// the linker place it in `.bss` (`NOBITS`, no file bytes), while a
    /// *single* non-zero byte anywhere in the struct forces it into
    /// `.data` (`PROGBITS`) and writes the whole
    /// `DEPTH x capacity x size_of::<GateEntry>()` array — tens of KiB of
    /// zeros — into every program's `.so`. That is exactly what a
    /// `next_token: 1` initializer used to do. `initial_gate_store_is_all_zero_bytes`
    /// pins this.
    issued: u64,
    /// Number of CURRENTLY installed (unremoved) gates, `0..=DEPTH`.
    ///
    /// This is the hot-path fast-out: `check_lamport_mutation` /
    /// `check_lamport_delegation` run on EVERY runtime lamport write and
    /// EVERY writable CPI meta — including in the vast majority of
    /// programs that never declare `lamports(...)`. Without this counter
    /// the no-gate path still walked the `DEPTH` slots per call, and the
    /// router bench measured that dead scanning at ~+44 CU **per hop**
    /// (2026-07-09: swap rows regressed 1564/3044/4525 → 1610/3136/4663,
    /// flipping every row behind Quasar). `installed == 0` must make
    /// `check` a single load + branch. Zero when zeroed (the all-zero
    /// invariant below).
    installed: u64,
    /// Token of the CURRENT governing gate (highest live token), `0` when
    /// none. Maintained on install (the fresh token is always the max)
    /// and on remove (rescan only when the governing gate itself is
    /// removed), so [`active_slot`](Self::active_slot) is O(1) instead of
    /// a DEPTH-slot scan on every gated check. Zero when zeroed (the
    /// all-zero invariant below).
    top_token: u64,
    /// Slot index holding [`top_token`](Self::top_token); meaningless
    /// while `top_token == 0`. Zero when zeroed.
    top_idx: u8,
    slots: [GateSlot; DEPTH],
}

impl<const DEPTH: usize> GateStore<DEPTH> {
    /// All-zero by construction — see [`GateStore::issued`]. Do not add a
    /// non-zero field initializer here without re-reading that doc.
    const fn new() -> Self {
        Self {
            issued: 0,
            installed: 0,
            top_token: 0,
            top_idx: 0,
            slots: [GateSlot::FREE; DEPTH],
        }
    }

    /// Copy `(address, permission)` values for every account into a free
    /// slot and return that slot's unique token. Fails closed (no state
    /// change) when the accounts outnumber the capacity or no slot is
    /// free.
    fn install_with_args(
        &mut self,
        accounts: &[crate::account::AccountView<'_>],
        policy: &'static WritePolicy,
        args: &[u32],
    ) -> Result<u64, GateInstallError> {
        if accounts.len() > LAMPORT_GATE_CAPACITY {
            return Err(GateInstallError::TooManyAccounts);
        }
        if args.len() > AMBIENT_GATE_ARG_CAPACITY {
            return Err(GateInstallError::TooManyArguments);
        }
        // NOTE (binary size): every index in this function is derived from a
        // value LLVM cannot statically bound (a `usize::MAX` sentinel, or the
        // stored `len`). A `slice[i]` it cannot prove in-bounds emits
        // `panic_bounds_check`, which *formats* its arguments — dragging
        // `core::fmt` (Formatter::pad_integral, do_count_chars, the integer
        // Display impls) into `.text`. One such site costs ~5 KiB in every
        // Hopper program. So this whole path uses iterators / `get`/`get_mut`,
        // which are provably panic-free. Keep it that way.
        let (slot_idx, slot) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, s)| s.token == 0)
            .ok_or(GateInstallError::NoFreeSlot)?;

        let mut len = 0usize;
        for (i, view) in accounts.iter().enumerate() {
            // Reading `address()` here is an always-safe read of a live
            // reference the caller just gave us; only the VALUE is kept.
            let address = *view.address();
            // Indices above u8::MAX can never have been declared (the
            // policy wire format is u8): permissionless. Unreachable
            // while LAMPORT_GATE_CAPACITY <= 255, kept for honesty.
            let (index, allow_mutation, allow_delegation, allow_transition) =
                if i <= u8::MAX as usize {
                    let idx = i as u8;
                    let m = policy.allows_lamport_mutation(idx);
                    (
                        idx,
                        m,
                        policy.lamports_declared() && m && policy.allows_whole_account_write(idx),
                        policy.allows_any_account_write(idx),
                    )
                } else {
                    (u8::MAX, false, false, false)
                };
            // Keep one entry per account position. Duplicate addresses are
            // still OR-authorized by the check scan, while retaining every
            // index lets exact byte ranges be evaluated correctly.
            let Some(entry) = slot.entries.get_mut(len) else {
                return Err(GateInstallError::TooManyAccounts);
            };
            *entry = GateEntry {
                address,
                index,
                allow_mutation,
                allow_delegation,
                allow_transition,
            };
            len += 1;
        }
        slot.len = len;
        slot.policy = Some(policy);
        slot.args_len = args.len();
        for (dst, src) in slot.args.iter_mut().zip(args.iter().copied()) {
            *dst = src;
        }
        // Tokens are 1, 2, 3, ... — `issued` counts up from a zeroed store
        // (which is what keeps this static in `.bss`; see the field doc).
        // 0 is the free-slot sentinel, so skip it on the (2^64 installs,
        // practically unreachable) wrap.
        self.issued = self.issued.wrapping_add(1);
        if self.issued == 0 {
            self.issued = 1;
        }
        let token = self.issued;
        slot.token = token;
        // The fresh token is strictly the highest live token, so it is
        // always the new governing gate.
        self.top_token = token;
        self.top_idx = slot_idx as u8;
        self.installed += 1;
        Ok(token)
    }

    /// Free exactly the slot installed under `token` (no-op when the
    /// token is not present — e.g. an inert guard). A guard can only
    /// ever clear its own slot, never another gate's. Returns whether a
    /// live slot was actually freed, so tier wrappers can maintain their
    /// fast-path liveness flag.
    fn remove(&mut self, token: u64) -> bool {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.token == token) {
            slot.token = 0;
            slot.len = 0;
            slot.policy = None;
            slot.args_len = 0;
            // Saturating: a stale/forged token cannot reach here (the
            // find above gates on a live token), but keep the counter
            // incapable of wrapping regardless.
            self.installed = self.installed.saturating_sub(1);
            // Removing the governing gate resumes the next-highest live
            // one: rescan (rare path — only when the INNER of a nested
            // pair drops, never on ordinary single-gate teardown after
            // the counter already hit zero).
            if token == self.top_token {
                let mut best_token = 0u64;
                let mut best_idx = 0u8;
                for (i, s) in self.slots.iter().enumerate() {
                    if s.token > best_token {
                        best_token = s.token;
                        best_idx = i as u8;
                    }
                }
                self.top_token = best_token;
                self.top_idx = best_idx;
            }
            true
        } else {
            false
        }
    }

    /// The governing gate: the still-active slot with the highest token,
    /// i.e. the most recently installed gate. Nested binds therefore
    /// shadow outer gates while alive; when the inner guard drops, the
    /// outer gate resumes governing.
    fn active_slot(&self) -> Option<&GateSlot> {
        // Tokens are unique and monotonic, so "highest token" is exactly
        // "most recently installed" — maintained as `top_token`/`top_idx`
        // by install/remove, making this a two-load lookup instead of a
        // DEPTH-slot scan on every gated check. `get` (not indexing)
        // keeps the path provably panic-free (see the binary-size note in
        // `install_with_args`).
        if self.top_token == 0 {
            return None;
        }
        self.slots.get(self.top_idx as usize)
    }

    /// Match `address` against the governing gate's stored values.
    /// Passthrough when no gate is installed; fail closed (indexed
    /// error, `u8::MAX` for addresses foreign to the gated instruction)
    /// otherwise.
    #[cfg_attr(feature = "unguarded-raw-surfaces", allow(unused_variables))]
    fn check(&self, address: &Address, check: GateCheck) -> ProgramResult {
        // Hot-path fast-out: programs that never declare `lamports(...)`
        // pay ONE load + branch here, not a slot walk. This check runs on
        // every lamport write and every writable CPI meta, so the walk
        // was measured at ~+44 CU per router hop before this guard.
        if self.installed == 0 {
            return Ok(());
        }
        let Some(slot) = self.active_slot() else {
            return Ok(());
        };
        let Some(policy) = slot.policy else {
            // A live token without its policy can only result from corrupted
            // ambient state. Refuse it as a foreign-account mutation.
            return Err(write_policy_violation(u8::MAX));
        };
        // A data-only strict policy intentionally leaves direct lamport
        // arithmetic ungoverned for backward compatibility. Other ambient
        // effects remain governed, including writable CPI delegation.
        if matches!(check, GateCheck::Lamports) && !policy.lamports_declared() {
            return Ok(());
        }
        // `get(..len)` rather than `entries[i]`: `len` is a stored field, so
        // LLVM cannot prove an index derived from it is in bounds and would
        // emit a formatting `panic_bounds_check` (~5 KiB of `core::fmt`).
        // A corrupt `len` degrades to an empty slice, i.e. fail closed.
        let seen = slot.entries.get(..slot.len).unwrap_or(&[]);
        let args = slot.args.get(..slot.args_len).unwrap_or(&[]);
        let mut first_matching_index = None;
        for entry in seen {
            if entry.address == *address {
                if first_matching_index.is_none() {
                    first_matching_index = Some(entry.index);
                }
                let allowed = match check {
                    GateCheck::Lamports => entry.allow_mutation,
                    #[cfg(not(feature = "unguarded-raw-surfaces"))]
                    GateCheck::Data { offset, size } => policy
                        .check_write_with_args(entry.index, offset, size, args)
                        .is_ok(),
                    #[cfg(not(feature = "unguarded-raw-surfaces"))]
                    GateCheck::Transition => entry.allow_transition,
                    GateCheck::Delegation => entry.allow_delegation,
                };
                if allowed {
                    return Ok(());
                }
            }
        }
        Err(write_policy_violation(
            first_matching_index.unwrap_or(u8::MAX),
        ))
    }

    /// Whether any gate (governing or shadowed) is installed.
    fn any_active(&self) -> bool {
        self.installed != 0
    }
}

/// Spinlocked single-slot gate store — the host fallback tier's cell
/// (`no_std` multi-threaded hosts without the `thread-local-registry`
/// feature). Also compiled under `test` so the tier's occupancy
/// semantics stay unit-testable while the thread-local tier is the
/// active one.
///
/// Single-slot occupancy is deliberate: one global store is shared by
/// every thread, so "nesting" cannot be attributed to a thread and a
/// second install while any gate is active is refused loudly
/// ([`LAMPORT_GATE_CONTENDED`]) instead of silently sharing or
/// corrupting another thread's gate. All address copies and
/// comparisons happen entirely inside [`Self::with_lock`]; nothing
/// pointer- or reference-shaped survives past the lock release.
#[cfg(all(
    not(target_os = "solana"),
    any(test, not(feature = "thread-local-registry"))
))]
struct SpinlockGateStore {
    lock: core::sync::atomic::AtomicBool,
    cell: core::cell::UnsafeCell<GateStore<1>>,
}

// SAFETY: all access to `cell` goes through `with_lock`, which
// serializes via the acquire/release spinlock, so no two threads can
// observe the store concurrently; and the store contains only plain
// values (addresses, flags, tokens — no pointers or references), so no
// other thread-affine state is smuggled across threads.
#[cfg(all(
    not(target_os = "solana"),
    any(test, not(feature = "thread-local-registry"))
))]
unsafe impl Sync for SpinlockGateStore {}

#[cfg(all(
    not(target_os = "solana"),
    any(test, not(feature = "thread-local-registry"))
))]
impl SpinlockGateStore {
    const fn new() -> Self {
        Self {
            lock: core::sync::atomic::AtomicBool::new(false),
            cell: core::cell::UnsafeCell::new(GateStore::new()),
        }
    }

    fn with_lock<R>(&self, f: impl FnOnce(&mut GateStore<1>) -> R) -> R {
        use core::sync::atomic::Ordering;
        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // SAFETY: the spinlock above grants exclusive access to the
        // store until the release store below, and the store operations
        // passed as `f` (`install`/`remove`/`check`/`any_active`) never
        // call back into this module, so the `&mut` cannot be aliased
        // or re-entered while it is live.
        let result = f(unsafe { &mut *self.cell.get() });
        self.lock.store(false, Ordering::Release);
        result
    }
}

/// Gate nesting depth on SBF. Each CPI level runs in its own VM with
/// its own heap, so cross-program nesting never shares this store;
/// depth is only consumed by one handler binding multiple
/// lamport-declared contexts simultaneously in the SAME instruction.
/// Two concurrent binds is already exotic; a third fails loudly with
/// [`LAMPORT_GATE_DEPTH_EXCEEDED`].
#[cfg(target_os = "solana")]
pub(crate) const SBF_GATE_DEPTH: usize = 2;

/// End offset (exclusive) of the lamport gate store inside the reserved
/// VM-heap scratch (`hopper_native::HEAP_RUNTIME_RESERVED`) — i.e. the
/// first byte a LATER ambient consumer may claim. The touch log
/// (`segment_borrow::touch_log`) starts at this offset rounded up to 8.
/// Anyone adding a third consumer extends from THAT module's end the
/// same way, keeping the reserved-scratch layout a single linear chain.
#[cfg(target_os = "solana")]
pub(crate) const SBF_GATE_HEAP_END: usize =
    core::mem::size_of::<usize>() + core::mem::size_of::<GateStore<SBF_GATE_DEPTH>>();

#[cfg(target_os = "solana")]
mod gate_store {
    use super::{GateCheck, GateInstallError, GateStore, WritePolicy, SBF_GATE_DEPTH};
    use crate::address::Address;
    use crate::error::ProgramError;
    use crate::ProgramResult;

    /// Byte offset of the gate store inside the VM heap region: right
    /// after the [`hopper_native::BumpAllocator`] cursor word.
    const GATE_HEAP_OFFSET: usize = core::mem::size_of::<usize>();

    // The store must fit the reserved runtime scratch at the heap bottom,
    // and start 8-aligned (GateStore leads with a u64).
    const _: () = assert!(
        core::mem::size_of::<GateStore<SBF_GATE_DEPTH>>()
            <= hopper_native::HEAP_RUNTIME_RESERVED - GATE_HEAP_OFFSET,
        "GateStore exceeds HEAP_RUNTIME_RESERVED; grow the reservation in \
         hopper-native/src/entrypoint.rs or shrink the store"
    );
    const _: () = assert!((hopper_native::HEAP_START_ADDRESS + GATE_HEAP_OFFSET) % 8 == 0);

    /// This tier supports two simultaneous same-instruction gates;
    /// running out means a handler bound three lamport-declared contexts
    /// at once (or leaked guards).
    pub(super) const NO_FREE_SLOT: ProgramError = super::LAMPORT_GATE_DEPTH_EXCEEDED;

    /// Per-invocation gate store, living in the RESERVED BOTTOM of the VM
    /// heap (`[HEAP_START + 8, HEAP_START + 8 + size_of::<GateStore>...)`,
    /// see [`hopper_native::HEAP_RUNTIME_RESERVED`]).
    ///
    /// Why the heap and not a `static`: deployed SBF programs cannot carry
    /// writable sections AT ALL — the loader's ELF parser rejects
    /// `.bss`/`.data` (`WritableSectionNotSupported`), so ANY `static mut`
    /// here makes every program that links the gate fail to load. (Found
    /// empirically: Mollusk refused the parity vault; the loader is the
    /// same parser mainnet uses.) The VM heap is the one writable region a
    /// program owns, and it is **zero-initialized by the VM on every
    /// invocation** — which composes exactly with two invariants this
    /// module already pins:
    ///
    /// - `GateStore::new()` is ALL-ZERO (`initial_gate_store_is_all_zero_bytes`),
    ///   so zeroed heap IS the valid empty store; no init code runs at all.
    /// - Heap freshness per invocation gives instruction scoping for free:
    ///   state can never leak across transactions or CPI levels (each CPI
    ///   level is its own VM with its own heap).
    ///
    /// The `BumpAllocator`'s floor excludes this range, so allocations can
    /// never overwrite an installed gate.
    fn with_store<R>(f: impl FnOnce(&mut GateStore<SBF_GATE_DEPTH>) -> R) -> R {
        let ptr = (hopper_native::HEAP_START_ADDRESS + GATE_HEAP_OFFSET)
            as *mut GateStore<SBF_GATE_DEPTH>;
        // SAFETY: SBF execution is single-threaded and none of the store
        // operations passed as `f` (`install`/`remove`/`check`/
        // `any_active`) re-enter this module, so this exclusive reference
        // is unique for the duration of `f`. The pointee is valid: the VM
        // maps and ZEROES the heap region for every invocation, all-zero
        // bytes are a valid `GateStore` (pinned by
        // `initial_gate_store_is_all_zero_bytes`), the address is 8-aligned
        // (const-asserted above) and the whole object lies inside
        // `HEAP_RUNTIME_RESERVED` (const-asserted above), a range the
        // `BumpAllocator` floor excludes and no other Hopper code touches.
        f(unsafe { &mut *ptr })
    }

    pub(super) fn install_with_args(
        accounts: &[crate::account::AccountView<'_>],
        policy: &'static WritePolicy,
        args: &[u32],
    ) -> Result<u64, GateInstallError> {
        with_store(|store| store.install_with_args(accounts, policy, args))
    }

    pub(super) fn remove(token: u64) {
        with_store(|store| store.remove(token));
    }

    pub(super) fn check(address: &Address, check: GateCheck) -> ProgramResult {
        with_store(|store| store.check(address, check))
    }

    pub(super) fn any_active() -> bool {
        with_store(|store| store.any_active())
    }
}

#[cfg(all(
    not(target_os = "solana"),
    any(test, feature = "thread-local-registry")
))]
mod gate_store {
    use super::{GateCheck, GateInstallError, GateStore, WritePolicy, LAMPORT_GATE_DEPTH};
    use crate::address::Address;
    use crate::error::ProgramError;
    use crate::ProgramResult;
    use std::cell::RefCell;

    // Per-thread gate store: hopper-runtime's own unit tests get it via
    // `test`; downstream test binaries opt in through the same
    // `thread-local-registry` feature the borrow registry uses, so
    // parallel test threads never observe each other's gates.
    std::thread_local! {
        static STORE: RefCell<GateStore<LAMPORT_GATE_DEPTH>> =
            const { RefCell::new(GateStore::new()) };
    }

    /// This tier supports full nesting; running out of slots means
    /// nesting deeper than the Solana invoke-depth budget (or leaking
    /// guards).
    pub(super) const NO_FREE_SLOT: ProgramError = super::LAMPORT_GATE_DEPTH_EXCEEDED;

    fn with_store<R>(f: impl FnOnce(&mut GateStore<LAMPORT_GATE_DEPTH>) -> R) -> R {
        STORE.with(|cell| f(&mut cell.borrow_mut()))
    }

    pub(super) fn install_with_args(
        accounts: &[crate::account::AccountView<'_>],
        policy: &'static WritePolicy,
        args: &[u32],
    ) -> Result<u64, GateInstallError> {
        with_store(|store| store.install_with_args(accounts, policy, args))
    }

    pub(super) fn remove(token: u64) {
        with_store(|store| store.remove(token));
    }

    pub(super) fn check(address: &Address, check: GateCheck) -> ProgramResult {
        with_store(|store| store.check(address, check))
    }

    pub(super) fn any_active() -> bool {
        with_store(|store| store.any_active())
    }
}

#[cfg(all(
    not(target_os = "solana"),
    not(any(test, feature = "thread-local-registry"))
))]
mod gate_store {
    use super::{GateCheck, GateInstallError, SpinlockGateStore, WritePolicy};
    use crate::address::Address;
    use crate::error::ProgramError;
    use crate::ProgramResult;

    /// Host fallback tier (`no_std` hosts without the thread-local
    /// feature): one process-global spinlocked slot. Cross-thread
    /// sharing means the one active gate governs every thread's lamport
    /// writes in this configuration — the same (documented) imprecision
    /// the fallback borrow registry accepts, and it fails closed rather
    /// than open. A second install while one gate is active is refused
    /// with [`super::LAMPORT_GATE_CONTENDED`] (see
    /// [`SpinlockGateStore`]); the per-thread and SBF tiers keep full
    /// nesting.
    static STORE: SpinlockGateStore = SpinlockGateStore::new();

    /// Single-slot occupancy: a second concurrent install is contention,
    /// refused loudly instead of shared or corrupted.
    pub(super) const NO_FREE_SLOT: ProgramError = super::LAMPORT_GATE_CONTENDED;

    pub(super) fn install_with_args(
        accounts: &[crate::account::AccountView<'_>],
        policy: &'static WritePolicy,
        args: &[u32],
    ) -> Result<u64, GateInstallError> {
        STORE.with_lock(|store| store.install_with_args(accounts, policy, args))
    }

    pub(super) fn remove(token: u64) {
        STORE.with_lock(|store| store.remove(token));
    }

    pub(super) fn check(address: &Address, check: GateCheck) -> ProgramResult {
        STORE.with_lock(|store| store.check(address, check))
    }

    pub(super) fn any_active() -> bool {
        STORE.with_lock(|store| store.any_active())
    }
}

/// RAII installation of the lamport gate for one bound instruction.
///
/// Returned by [`try_install_lamport_gate`] / [`install_lamport_gate`].
/// While this is the most recently installed still-active gate on its
/// tier, the runtime's lamport choke points refuse mutation on any
/// account the installed policy does not permit; an inner (newer) gate
/// shadows it until that inner guard drops. Dropping frees exactly this
/// guard's slot (matched by unique token), so guards may be dropped in
/// any order without disturbing — or resurrecting — other gates.
///
/// ## Leak behavior (`mem::forget`)
///
/// The gate store holds copied address values, never pointers into the
/// account slice, so leaking the guard leaves a **stale value policy**
/// installed: later checks on this tier keep being governed by it
/// (addresses it does not know fail closed) until enough leaks exhaust
/// the tier's [`LAMPORT_GATE_DEPTH`] slots and further installs fail
/// loudly. That is observable over-/stale enforcement — never memory
/// unsafety.
///
/// The `'accounts` lifetime parameter is retained for API stability
/// (macro codegen names `LamportGateGuard<'a>`); it is not load-bearing
/// for soundness, because nothing borrowed from the slice outlives the
/// install call.
#[derive(Debug)]
pub struct LamportGateGuard<'accounts> {
    /// Unique slot token; 0 = inert guard (nothing installed).
    token: u64,
    _accounts: core::marker::PhantomData<&'accounts ()>,
}

impl Drop for LamportGateGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        if self.token != 0 {
            gate_store::remove(self.token);
        }
    }
}

/// Install the instruction-scoped lamport gate for `accounts` under
/// `policy` — the fallible entry point.
///
/// Returns an inert guard (nothing installed) unless `policy` declares
/// its lamport dimension ([`WritePolicy::with_lamports`]) — a data-only
/// policy keeps the passthrough behavior. `#[hopper::context(
/// strict_writes, lamports(...))]`-generated `bind()` calls this (and
/// fails the bind on error) and stores the guard in the bound context,
/// so the gate lives exactly as long as the bound instruction scope.
///
/// # Errors
///
/// Fails closed, changing nothing, with
/// [`LAMPORT_GATE_TOO_MANY_ACCOUNTS`] (slice larger than
/// [`LAMPORT_GATE_CAPACITY`]), [`LAMPORT_GATE_DEPTH_EXCEEDED`] (all
/// [`LAMPORT_GATE_DEPTH`] slots on a nesting tier in use), or
/// [`LAMPORT_GATE_CONTENDED`] (host fallback tier: another still-active
/// gate occupies the process-global slot).
#[inline]
pub fn try_install_lamport_gate<'accounts>(
    accounts: &'accounts [crate::account::AccountView<'accounts>],
    policy: &'static WritePolicy,
) -> Result<LamportGateGuard<'accounts>, ProgramError> {
    try_install_ambient_gate_with_args(accounts, policy, &[])
}

/// Install the complete instruction-scoped ambient mutation gate, including
/// invocation-resolved selector values for parametric exact-cell policies.
///
/// Generated `strict_writes` bindings use this entry point. A data-only
/// policy still preserves undeclared-lamport passthrough, but raw data
/// mutation, account transitions, writable CPI delegation, and accounts
/// outside the declared write-set are governed for the guard's lifetime.
///
/// Under the `unguarded-raw-surfaces` size opt-out, installing a policy
/// that declares data write ranges is refused with
/// [`AMBIENT_GATE_UNGUARDED_BUILD`] — that build cannot enforce the raw
/// surfaces, and a silently half-enforced gate is worse than a loud
/// refusal. Lamports-only policies still install and stay enforced.
#[inline]
pub fn try_install_ambient_gate_with_args<'accounts>(
    accounts: &'accounts [crate::account::AccountView<'accounts>],
    policy: &'static WritePolicy,
    args: &[u32],
) -> Result<LamportGateGuard<'accounts>, ProgramError> {
    // Runtime fence for the `unguarded-raw-surfaces` size opt-out: a
    // policy that declares data write ranges is asking for governance
    // this build cannot enforce on the raw `AccountView` surfaces, so
    // the install refuses loudly instead of degrading silently. The
    // macro tier can never reach this (strict contexts are a compile
    // error under the opt-out); this catches hand-rolled installs, and
    // costs nothing on default builds.
    #[cfg(feature = "unguarded-raw-surfaces")]
    if !policy.allows.is_empty() || !policy.parametric.is_empty() {
        return Err(AMBIENT_GATE_UNGUARDED_BUILD);
    }
    match gate_store::install_with_args(accounts, policy, args) {
        Ok(token) => Ok(LamportGateGuard {
            token,
            _accounts: core::marker::PhantomData,
        }),
        Err(GateInstallError::TooManyAccounts) => Err(LAMPORT_GATE_TOO_MANY_ACCOUNTS),
        Err(GateInstallError::TooManyArguments) => Err(AMBIENT_GATE_TOO_MANY_ARGUMENTS),
        Err(GateInstallError::NoFreeSlot) => Err(gate_store::NO_FREE_SLOT),
    }
}

/// Infallible wrapper around [`try_install_lamport_gate`], kept for
/// call sites that predate the fallible API.
///
/// # Panics
///
/// Panics (failing the whole invocation — still fail-closed, still
/// loud) if the install is refused: more accounts than
/// [`LAMPORT_GATE_CAPACITY`], no free gate slot, or a contended host
/// fallback store. Callers that want the refusal as a
/// [`ProgramError`] — every generated `bind()` does — must use
/// [`try_install_lamport_gate`].
#[inline]
pub fn install_lamport_gate<'accounts>(
    accounts: &'accounts [crate::account::AccountView<'accounts>],
    policy: &'static WritePolicy,
) -> LamportGateGuard<'accounts> {
    match try_install_lamport_gate(accounts, policy) {
        Ok(guard) => guard,
        Err(_) => panic!(
            "lamport gate install refused (capacity or slot occupancy); \
             use try_install_lamport_gate to handle this as a ProgramError"
        ),
    }
}

/// Whether a lamport gate is currently installed on this tier
/// (diagnostic/testing). Counts shadowed and leaked (stale) gates.
#[inline]
pub fn lamport_gate_active() -> bool {
    gate_store::any_active()
}

/// Gate a direct lamport mutation (`try_set_lamports` / `close`) on the
/// account with `address`.
///
/// The caller passes the address it read from the **live view** it is
/// about to mutate (an always-safe read); it is compared against the
/// address values copied at install time — nothing stored is
/// dereferenced. `Ok(())` when no gate is installed (the dimension is
/// opt-in) or when the governing gate permits lamport mutation on that
/// address.
#[inline]
pub(crate) fn check_lamport_mutation(address: &Address) -> ProgramResult {
    gate_store::check(address, GateCheck::Lamports)
}

/// Gate a mutable data borrow against the invocation-resolved byte policy.
/// AccountView-level raw borrows pass the full buffer; segment paths pass the
/// exact range, so parametric cells remain enforceable outside `Context`.
#[inline]
pub(crate) fn check_data_mutation(address: &Address, offset: u32, size: u32) -> ProgramResult {
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    {
        gate_store::check(address, GateCheck::Data { offset, size })
    }
    #[cfg(feature = "unguarded-raw-surfaces")]
    {
        // Raw-tier opt-out: the raw-surface guard is compiled out, which
        // also unlinks the gate-check machinery from programs that never
        // install a gate. `strict_writes` codegen const-asserts
        // [`RAW_SURFACES_GUARDED`], so this branch cannot coexist with a
        // bound strict context.
        let _ = (address, offset, size);
        Ok(())
    }
}

/// Gate a data-length/presence transition on an account. Any declared data
/// authority grants the transition capability; foreign and remaining-only
/// accounts fail closed.
#[inline]
pub(crate) fn check_account_transition(address: &Address) -> ProgramResult {
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    {
        gate_store::check(address, GateCheck::Transition)
    }
    #[cfg(feature = "unguarded-raw-surfaces")]
    {
        let _ = address;
        Ok(())
    }
}

/// Whether the public raw `AccountView` surfaces are governed by the
/// ambient write gate in this build (the default). `false` only under the
/// `unguarded-raw-surfaces` opt-out; `strict_writes` codegen const-asserts
/// this is `true`, making the opt-out impossible to combine with a bound
/// strict context.
pub const RAW_SURFACES_GUARDED: bool = !cfg!(feature = "unguarded-raw-surfaces");

/// Gate a CPI **writable meta** on the account with `address`. A
/// writable hand-off delegates unbounded data AND lamport mutation to
/// the callee, so it requires the account to carry both a
/// whole-account data grant and lamport permission. Same value-compare
/// contract as [`check_lamport_mutation`].
#[inline]
pub(crate) fn check_lamport_delegation(address: &Address) -> ProgramResult {
    gate_store::check(address, GateCheck::Delegation)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod gate_store_layout_tests {
    use super::*;

    /// The SBF tier holds [`GateStore`] in a `static mut`. The linker puts
    /// it in `.bss` (`NOBITS`, **zero bytes in the `.so`**) only if its
    /// initializer is entirely zero; one non-zero byte moves it to `.data`
    /// (`PROGBITS`) and writes the whole ~35 KiB array of zeros into every
    /// Hopper program's binary. A `next_token: 1` initializer did exactly
    /// that, costing 35,656 file bytes — 61% of the vault's `.so`.
    ///
    /// This pins every field's initializer at zero. If you add a field to
    /// `GateStore`/`GateSlot`/`GateEntry`, its zero value must be valid, or
    /// the binary silently regrows.
    #[test]
    fn initial_gate_store_is_all_zero_bytes() {
        let store = GateStore::<2>::new();
        assert_eq!(store.issued, 0, "issued must start at 0, not 1");
        assert_eq!(store.installed, 0, "installed count must start at 0");
        for slot in &store.slots {
            assert_eq!(slot.token, 0, "free-slot sentinel is 0");
            assert_eq!(slot.len, 0);
            for entry in slot.entries.iter() {
                assert_eq!(*entry.address.as_array(), [0u8; 32]);
                assert_eq!(entry.index, 0);
                assert!(!entry.allow_mutation);
                assert!(!entry.allow_delegation);
            }
        }
    }

    /// Tokens must still be non-zero and monotonic after the zero-init
    /// change (0 stays the free-slot sentinel), so nesting/shadowing and
    /// out-of-order drops keep working.
    #[test]
    fn issued_counter_hands_out_nonzero_monotonic_tokens() {
        let mut store = GateStore::<2>::new();
        store.issued = 0;
        store.issued = store.issued.wrapping_add(1);
        assert_eq!(store.issued, 1, "first token is 1, never the 0 sentinel");
        // Wrap: 2^64 installs must skip the sentinel rather than hand out 0.
        store.issued = u64::MAX;
        store.issued = store.issued.wrapping_add(1);
        if store.issued == 0 {
            store.issued = 1;
        }
        assert_eq!(store.issued, 1, "wrap skips the 0 sentinel");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // vault (account 1): balance [16, 24), nonce [24, 32)
    static POLICY: WritePolicy = WritePolicy::new(&[
        WriteRange::new(1, 16, 8),
        WriteRange::new(1, 24, 8),
        WriteRange::whole_account(2),
    ]);

    #[test]
    fn declared_ranges_allow_exact_and_contained_writes() {
        assert!(POLICY.check_write(1, 16, 8).is_ok());
        assert!(POLICY.check_write(1, 24, 8).is_ok());
        // Strictly inside a declared range is also allowed.
        assert!(POLICY.check_write(1, 18, 4).is_ok());
        // Zero-size request inside a range is trivially contained.
        assert!(POLICY.check_write(1, 20, 0).is_ok());
    }

    #[test]
    fn parametric_column_allows_only_the_selected_cell() {
        static PARAMETRIC: &[ParametricWriteRange] = &[ParametricWriteRange::new(
            1, 100, 8, 8, 20, 0, "slot", "balances",
        )];
        static P: WritePolicy =
            WritePolicy::with_parametric(&[WriteRange::new(1, 100, 20 * 8)], PARAMETRIC);

        assert!(P.check_write_with_args(1, 100 + 7 * 8, 8, &[7]).is_ok());
        assert!(P.check_write_with_args(1, 100 + 7 * 8 + 2, 4, &[7]).is_ok());
        assert_eq!(
            P.check_write_with_args(1, 100 + 8 * 8, 8, &[7]),
            Err(ProgramError::Custom(0xD0_01)),
        );
        assert!(P.check_write_with_args(1, 100 + 19 * 8, 8, &[20]).is_err());
        assert!(P.check_write_with_args(1, 100 + 7 * 8, 8, &[]).is_err());
    }

    #[test]
    fn containment_oracle_resolves_parametric_cells_and_static_unions() {
        static PARAMETRIC: &[ParametricWriteRange] = &[ParametricWriteRange::new(
            1, 100, 8, 8, 20, 0, "slot", "balances",
        )];
        static P: WritePolicy = WritePolicy::with_parametric(
            &[
                WriteRange::new(1, 96, 4),
                WriteRange::new(1, 100, 20 * 8),
                WriteRange::new(1, 260, 4),
            ],
            PARAMETRIC,
        );

        assert_eq!(
            P.first_unauthorized_byte_with_args(1, 100 + 7 * 8, 8, &[7]),
            None
        );
        assert_eq!(
            P.first_unauthorized_byte_with_args(1, 100 + 8 * 8, 8, &[7]),
            Some((100 + 8 * 8) as u64)
        );
        // Coalesced records may span an adjacent static acquire and the
        // selected first cell even though one runtime acquire may not.
        assert_eq!(P.first_unauthorized_byte_with_args(1, 96, 12, &[0]), None);
        assert_eq!(
            P.first_unauthorized_byte_with_args(1, 100, 8, &[]),
            Some(100)
        );
        assert_eq!(
            P.first_unauthorized_byte_with_args(1, 100, 8, &[20]),
            Some(100)
        );

        // The non-parametric oracle walks the union, so a coalesced touch
        // over adjacent declarations remains certifiable.
        assert_eq!(
            POLICY.first_unauthorized_byte_with_args(1, 16, 16, &[]),
            None
        );
        assert_eq!(
            POLICY.first_unauthorized_byte_with_args(1, 15, 17, &[]),
            Some(15)
        );
    }

    #[test]
    fn undeclared_ranges_are_refused_with_indexed_error() {
        // Outside every declared range.
        assert_eq!(
            POLICY.check_write(1, 0, 8),
            Err(ProgramError::Custom(0xD0_01))
        );
        // Overlapping but not contained.
        assert!(POLICY.check_write(1, 12, 8).is_err());
        // Straddling two adjacent declared ranges is refused: containment
        // is per-declaration, not per-union.
        assert!(POLICY.check_write(1, 16, 16).is_err());
        // Right account, range declared on a different account.
        assert_eq!(
            POLICY.check_write(0, 16, 8),
            Err(ProgramError::Custom(0xD0_00))
        );
    }

    #[test]
    fn whole_account_allowance_contains_any_request() {
        assert!(POLICY.check_write(2, 0, 8).is_ok());
        assert!(POLICY.check_write(2, 0, u32::MAX).is_ok());
        assert!(POLICY.check_write(2, 4096, 10 * 1024 * 1024).is_ok());
    }

    #[test]
    fn empty_policy_denies_all_writes() {
        static READ_ONLY: WritePolicy = WritePolicy::new(&[]);
        assert!(READ_ONLY.check_write(0, 0, 1).is_err());
        assert!(READ_ONLY.check_write(255, 0, 0).is_err());
    }

    #[test]
    fn open_ended_tail_range_allows_tail_refuses_head_and_is_not_whole_account() {
        // A `Seq<T>` tail on account 1 whose fixed head occupies `[0, 24)`
        // (16-byte Hopper header + an 8-byte head field): the tail region
        // starts at offset 24 and is open-ended.
        const TAIL_OFF: u32 = 24;
        static P: WritePolicy = WritePolicy::new(&[WriteRange::tail_from(1, TAIL_OFF)]);

        // Any sub-range at or past the tail offset is admitted, no matter
        // how large the account grows — the size is u32::MAX and `contains`
        // widens to u64 so `TAIL_OFF + u32::MAX` cannot wrap.
        assert!(P.check_write(1, TAIL_OFF, 4).is_ok()); // the u32 count prefix
        assert!(P.check_write(1, TAIL_OFF + 4, 32).is_ok()); // first element
        assert!(P.check_write(1, TAIL_OFF, 10 * 1024 * 1024).is_ok()); // grown far
        assert!(P.check_write(1, TAIL_OFF + 1_000_000, 32).is_ok());

        // Every byte of the fixed head is refused with the indexed error —
        // the open-ended tail range does NOT leak backwards onto the head.
        assert_eq!(P.check_write(1, 0, 8), Err(write_policy_violation(1)));
        assert_eq!(P.check_write(1, 16, 8), Err(write_policy_violation(1)));
        // A write straddling the head/tail boundary is refused (it is not
        // fully contained in the tail range).
        assert!(P.check_write(1, TAIL_OFF - 1, 8).is_err());

        // The load-bearing property: a tail range with `offset != 0` is
        // NOT a whole-account grant. CPI writable-meta delegation demands
        // `contains(0, u32::MAX)`, which starts at 0 and this range does
        // not — so delegation stays refused and the head stays protected.
        assert!(!P.allows_whole_account_write(1));
        // A tail range anchored at 0 (a degenerate "whole tail from the
        // start") IS a whole-account grant, by the same rule.
        static P0: WritePolicy = WritePolicy::new(&[WriteRange::tail_from(1, 0)]);
        assert!(P0.allows_whole_account_write(1));
    }

    #[test]
    fn containment_survives_u32_boundary_arithmetic() {
        static EDGE: WritePolicy = WritePolicy::new(&[WriteRange::new(0, u32::MAX - 8, 8)]);
        // `offset + size` at the top of u32 must not wrap into a false allow.
        assert!(EDGE.check_write(0, u32::MAX - 8, 8).is_ok());
        assert!(EDGE.check_write(0, u32::MAX - 4, 8).is_err());
    }

    // ── Lamport dimension (BLD-MUT) ─────────────────────────────────

    #[test]
    fn undeclared_lamport_dimension_permits_everything_and_is_incomplete() {
        assert!(!POLICY.lamports_declared());
        assert!(POLICY.allows_lamport_mutation(0));
        assert!(POLICY.allows_lamport_mutation(255));
    }

    #[test]
    fn declared_lamport_dimension_permits_only_members() {
        static P: WritePolicy =
            WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0, 3]);
        assert!(P.lamports_declared());
        assert!(P.allows_lamport_mutation(0));
        assert!(P.allows_lamport_mutation(3));
        assert!(!P.allows_lamport_mutation(1));
        // An empty declared set refuses every account — a valid,
        // mutation-complete "no lamport writes anywhere" contract.
        static NONE: WritePolicy = WritePolicy::with_lamports(&[], &[]);
        assert!(NONE.lamports_declared());
        assert!(!NONE.allows_lamport_mutation(0));
    }

    #[test]
    fn whole_account_grant_is_required_for_delegation() {
        static P: WritePolicy = WritePolicy::with_lamports(
            &[WriteRange::whole_account(0), WriteRange::new(1, 16, 8)],
            &[0, 1],
        );
        assert!(P.allows_whole_account_write(0));
        // Field-granular ranges are not a whole-account grant.
        assert!(!P.allows_whole_account_write(1));
        assert!(!P.allows_whole_account_write(2));
    }

    // ── Ambient gate behavior ───────────────────────────────────────

    use crate::account::AccountView;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make_account(seed: u8) -> (std::vec::Vec<u64>, AccountView<'static>) {
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + 32).div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: the test owns `backing`, writes one valid RuntimeAccount
        // header, and keeps the buffer alive for the returned view.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([seed; 32]),
                owner: NativeAddress::new_from_array([2; 32]),
                lamports: 100,
                data_len: 32,
            });
        }
        // SAFETY: `raw` points at the RuntimeAccount just initialized above.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    // Guarded-tier semantics: installs a data-declaring policy, which the
    // `unguarded-raw-surfaces` fence refuses at install (covered by its
    // own explicit test in that shape).
    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn gate_refuses_undeclared_and_allows_declared_lamport_mutation() {
        let (_b0, a0) = make_account(10);
        let (_b1, a1) = make_account(11);
        let accounts = [a0, a1];

        static P: WritePolicy = WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0]);

        // No gate installed: passthrough for everything.
        assert!(check_lamport_mutation(accounts[1].address()).is_ok());

        {
            let _gate = install_lamport_gate(&accounts, &P);
            assert!(lamport_gate_active());
            // Declared index 0: allowed. Undeclared index 1: refused
            // with the indexed policy error.
            assert!(check_lamport_mutation(accounts[0].address()).is_ok());
            assert_eq!(
                check_lamport_mutation(accounts[1].address()),
                Err(write_policy_violation(1))
            );
            // An address foreign to the gated slice fails closed.
            let (_bf, foreign) = make_account(99);
            assert_eq!(
                check_lamport_mutation(foreign.address()),
                Err(write_policy_violation(u8::MAX))
            );
        }

        // Guard dropped: gate cleared, passthrough restored.
        assert!(!lamport_gate_active());
        assert!(check_lamport_mutation(accounts[1].address()).is_ok());
    }

    // Guarded-tier semantics: installs a data-declaring policy, which the
    // `unguarded-raw-surfaces` fence refuses at install (covered by its
    // own explicit test in that shape).
    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn gate_delegation_requires_both_dimensions() {
        let (_b0, a0) = make_account(20);
        let (_b1, a1) = make_account(21);
        let (_b2, a2) = make_account(22);
        let accounts = [a0, a1, a2];

        // 0: whole data + lamports (delegable). 1: lamports only.
        // 2: field range only.
        static P: WritePolicy = WritePolicy::with_lamports(
            &[WriteRange::whole_account(0), WriteRange::new(2, 16, 8)],
            &[0, 1],
        );
        let _gate = install_lamport_gate(&accounts, &P);

        assert!(check_lamport_delegation(accounts[0].address()).is_ok());
        assert_eq!(
            check_lamport_delegation(accounts[1].address()),
            Err(write_policy_violation(1))
        );
        assert_eq!(
            check_lamport_delegation(accounts[2].address()),
            Err(write_policy_violation(2))
        );
        // Direct lamport mutation on 1 is still fine (declared).
        assert!(check_lamport_mutation(accounts[1].address()).is_ok());
    }

    /// The runtime fence for hand-rolled installs under the size opt-out:
    /// a data-declaring policy is REFUSED at install (never a silently
    /// half-enforced gate), while a lamports-only policy installs and its
    /// dimensions stay fully enforced.
    #[test]
    #[cfg(feature = "unguarded-raw-surfaces")]
    fn unguarded_build_refuses_data_declaring_installs_loudly() {
        let (_b0, a0) = make_account(95);
        let accounts = [a0];

        // Data-declaring policy: refused with the dedicated install error.
        static DATA: WritePolicy = WritePolicy::new(&[WriteRange::whole_account(0)]);
        assert_eq!(
            try_install_lamport_gate(&accounts, &DATA).map(|_| ()),
            Err(AMBIENT_GATE_UNGUARDED_BUILD),
        );
        assert!(!lamport_gate_active(), "a refused install leaves no gate");

        // Lamports-only policy: installs, and the lamport dimension still
        // enforces (declared account 0 passes, a foreign account fails).
        static LAMPORTS_ONLY: WritePolicy = WritePolicy::with_lamports(&[], &[0]);
        let _gate = install_lamport_gate(&accounts, &LAMPORTS_ONLY);
        let (_bf, foreign) = make_account(96);
        assert!(check_lamport_mutation(accounts[0].address()).is_ok());
        assert_eq!(
            check_lamport_mutation(foreign.address()),
            Err(write_policy_violation(u8::MAX)),
        );
    }

    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn public_raw_surfaces_are_governed_by_the_ambient_gate() {
        // The historical `strict_writes` bypass: raw `AccountView` writes
        // outside a `Context`. With the gate wired into the public
        // surfaces, a bound policy governs them too.
        let (_b0, a0) = make_account(90);
        let (_bf, foreign) = make_account(91);
        let accounts = [a0];
        // Account 0 may write ONLY bytes [8, 16) of its 32-byte data.
        static P: WritePolicy = WritePolicy::new(&[WriteRange::new(0, 8, 8)]);

        // Without a gate every surface passes through untouched.
        {
            let mut reg = crate::segment_borrow::SegmentBorrowRegistry::new();
            assert!(accounts[0].try_borrow_mut().is_ok());
            assert!(accounts[0].segment_mut::<[u8; 8]>(&mut reg, 20, 8).is_ok());
        }

        let _gate = install_lamport_gate(&accounts, &P);

        // Raw whole-account borrow: the narrow declaration does not cover
        // the full data range, so the former bypass is refused with the
        // account's indexed policy error.
        assert_eq!(
            accounts[0].try_borrow_mut().map(|_| ()),
            Err(write_policy_violation(0)),
        );
        // A foreign account fails closed at u8::MAX.
        assert_eq!(
            foreign.try_borrow_mut().map(|_| ()),
            Err(write_policy_violation(u8::MAX)),
        );

        // Direct segment access outside a `Context`: the declared cell is
        // allowed; a range shifted one byte past it is refused.
        let mut reg = crate::segment_borrow::SegmentBorrowRegistry::new();
        assert!(accounts[0].segment_mut::<[u8; 8]>(&mut reg, 8, 8).is_ok());
        let mut reg2 = crate::segment_borrow::SegmentBorrowRegistry::new();
        assert_eq!(
            accounts[0]
                .segment_mut::<[u8; 8]>(&mut reg2, 9, 8)
                .map(|_| ()),
            Err(write_policy_violation(0)),
        );

        // Data-length / presence transitions on a foreign account are
        // refused before any native-boundary work happens.
        assert_eq!(foreign.resize(16), Err(write_policy_violation(u8::MAX)));
        assert_eq!(foreign.close(), Err(write_policy_violation(u8::MAX)));
    }

    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn data_only_policy_governs_ambient_data_but_passes_lamports() {
        // A data-only `strict_writes` policy (no `lamports(...)`) still installs
        // the instruction-ambient gate: raw data mutation, writable CPI
        // delegation, and accounts outside the declared write-set are governed
        // for the guard's lifetime. Only DIRECT lamport arithmetic stays
        // passthrough — the documented backward-compatibility carve-out.
        let (_b0, a0) = make_account(30);
        let accounts = [a0];
        static P: WritePolicy = WritePolicy::new(&[WriteRange::whole_account(0)]);
        let _gate = install_lamport_gate(&accounts, &P);
        assert!(
            lamport_gate_active(),
            "a data-only policy installs the ambient gate"
        );

        let (_bf, foreign) = make_account(31);

        // Lamport dimension: undeclared, so BOTH the declared account and a
        // foreign one pass through. The gate never tightens direct lamport
        // arithmetic for a data-only policy.
        assert!(check_lamport_mutation(accounts[0].address()).is_ok());
        assert!(check_lamport_mutation(foreign.address()).is_ok());

        // Data dimension IS governed: the declared whole-account write is
        // permitted, but a foreign account's data write is refused fail-closed.
        assert!(check_data_mutation(accounts[0].address(), 0, 1).is_ok());
        assert_eq!(
            check_data_mutation(foreign.address(), 0, 1),
            Err(write_policy_violation(u8::MAX)),
        );

        // Writable CPI delegation is refused even for the DECLARED account: a
        // data-only policy carries no lamport authority to hand a callee, so
        // delegation of account 0 fails with its own index, and a foreign
        // account fails closed at u8::MAX.
        assert_eq!(
            check_lamport_delegation(accounts[0].address()),
            Err(write_policy_violation(0)),
        );
        assert_eq!(
            check_lamport_delegation(foreign.address()),
            Err(write_policy_violation(u8::MAX)),
        );
    }

    #[test]
    fn nested_gates_shadow_and_resume_like_a_stack() {
        let (_b0, a0) = make_account(40);
        let (_b1, a1) = make_account(41);
        let outer_accounts = [a0];
        let inner_accounts = [a1];
        static OUTER: WritePolicy = WritePolicy::with_lamports(&[], &[0]);
        static INNER: WritePolicy = WritePolicy::with_lamports(&[], &[]);

        let _outer = install_lamport_gate(&outer_accounts, &OUTER);
        assert!(check_lamport_mutation(outer_accounts[0].address()).is_ok());
        {
            let _inner = install_lamport_gate(&inner_accounts, &INNER);
            // The inner gate (highest token) shadows the outer one:
            // inner's empty set refuses its own account 0...
            assert_eq!(
                check_lamport_mutation(inner_accounts[0].address()),
                Err(write_policy_violation(0))
            );
        }
        // ...and dropping the inner guard frees only its slot, so the
        // outer gate resumes governing.
        assert!(lamport_gate_active());
        assert!(check_lamport_mutation(outer_accounts[0].address()).is_ok());
    }

    // ── Redesign regression tests (value store, tokens, fail-closed) ─

    // Guarded-tier semantics: installs a data-declaring policy, which the
    // `unguarded-raw-surfaces` fence refuses at install (covered by its
    // own explicit test in that shape).
    #[test]
    #[cfg(not(feature = "unguarded-raw-surfaces"))]
    fn forgotten_guard_leaves_stale_value_policy_never_ub() {
        static P: WritePolicy = WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0]);
        let stale_address;
        {
            let (_b0, a0) = make_account(50);
            let accounts = [a0];
            stale_address = *accounts[0].address();
            let guard = install_lamport_gate(&accounts, &P);
            // Safe code can always skip Drop. With the value store this
            // leaks a slot; the old pointer store turned every later
            // check into a dangling dereference.
            core::mem::forget(guard);
            // `accounts` and its backing buffer are freed here.
        }

        // The stale gate is still installed — as VALUES, so probing it
        // after the accounts are gone is plain comparison, not UB.
        assert!(lamport_gate_active());

        // A fresh, unrelated account is foreign to the stale policy:
        // fail closed (observable stale enforcement, the documented
        // residual of leaking a guard).
        let (_bf, fresh) = make_account(51);
        assert_eq!(
            check_lamport_mutation(fresh.address()),
            Err(write_policy_violation(u8::MAX))
        );

        // An address equal to the stale entry is governed by the stale
        // permission bits (allowed here) — stale-value semantics, by
        // value, no liveness required.
        assert!(check_lamport_mutation(&stale_address).is_ok());

        // The leaked slot stays occupied for this thread-local tier;
        // the test thread ends here, taking the store with it.
    }

    #[test]
    fn out_of_order_guard_drops_cannot_corrupt_other_gates() {
        let (_b0, a0) = make_account(60);
        let (_b1, a1) = make_account(61);
        let outer_accounts = [a0];
        let inner_accounts = [a1];
        static OUTER: WritePolicy = WritePolicy::with_lamports(&[], &[0]);
        static INNER: WritePolicy = WritePolicy::with_lamports(&[], &[]);

        let outer = install_lamport_gate(&outer_accounts, &OUTER);
        let inner = install_lamport_gate(&inner_accounts, &INNER);

        // Inner (most recently installed) governs while both are alive.
        assert_eq!(
            check_lamport_mutation(inner_accounts[0].address()),
            Err(write_policy_violation(0))
        );

        // Drop the OUTER guard first — out of creation order. Under the
        // old prev-chain this restored a stale snapshot over the live
        // inner gate; under the token store it frees only outer's slot.
        drop(outer);
        assert!(lamport_gate_active());
        assert_eq!(
            check_lamport_mutation(inner_accounts[0].address()),
            Err(write_policy_violation(0))
        );
        // Outer's account is now foreign to the (still governing) inner
        // gate — fail closed, not fail open.
        assert_eq!(
            check_lamport_mutation(outer_accounts[0].address()),
            Err(write_policy_violation(u8::MAX))
        );

        // Dropping inner empties the store: passthrough, nothing stale.
        drop(inner);
        assert!(!lamport_gate_active());
        assert!(check_lamport_mutation(outer_accounts[0].address()).is_ok());
    }

    #[test]
    fn depth_exhaustion_fails_closed_loudly() {
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[]);
        let (_b, a) = make_account(70);
        let accounts = [a];
        let _g1 = try_install_lamport_gate(&accounts, &P).unwrap();
        let _g2 = try_install_lamport_gate(&accounts, &P).unwrap();
        let _g3 = try_install_lamport_gate(&accounts, &P).unwrap();
        let _g4 = try_install_lamport_gate(&accounts, &P).unwrap();
        // All LAMPORT_GATE_DEPTH slots occupied: the next install is
        // refused loudly, nothing is evicted or corrupted.
        assert_eq!(
            try_install_lamport_gate(&accounts, &P).unwrap_err(),
            LAMPORT_GATE_DEPTH_EXCEEDED
        );
        // The existing gates keep enforcing.
        assert_eq!(
            check_lamport_mutation(accounts[0].address()),
            Err(write_policy_violation(0))
        );
    }

    #[test]
    fn install_fails_closed_when_accounts_exceed_capacity() {
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[]);
        let mut backings = std::vec::Vec::new();
        let mut views = std::vec::Vec::new();
        let mut i = 0;
        while i < LAMPORT_GATE_CAPACITY + 1 {
            let (backing, view) = make_account((i % 256) as u8);
            backings.push(backing);
            views.push(view);
            i += 1;
        }
        // More accounts than the gate can copy: refuse loudly rather
        // than silently truncate the governed set.
        assert_eq!(
            try_install_lamport_gate(&views, &P).unwrap_err(),
            LAMPORT_GATE_TOO_MANY_ACCOUNTS
        );
        assert!(!lamport_gate_active());
    }

    #[test]
    fn duplicate_addresses_share_one_merged_permission() {
        // Two views with the SAME address at indices 0 and 1; lamports
        // declared only on index 1. Permission is a property of the
        // account (the loader hands duplicate metas one RuntimeAccount),
        // so the merged entry allows mutation through either position.
        let (_b0, a0) = make_account(75);
        let (_b1, a1) = make_account(75);
        let accounts = [a0, a1];
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[1]);
        let _gate = install_lamport_gate(&accounts, &P);
        assert!(check_lamport_mutation(accounts[0].address()).is_ok());
        assert!(check_lamport_mutation(accounts[1].address()).is_ok());
    }

    #[test]
    fn fallback_tier_second_install_is_refused_fail_closed() {
        // Direct exercise of the host fallback tier's single-slot store
        // (the process-global tier is not active under `test`, but its
        // store type is compiled and its semantics are tier-independent:
        // GateStore<1> + spinlock).
        static P: WritePolicy = WritePolicy::with_lamports(&[], &[0]);
        let (_b0, a0) = make_account(80);
        let (_b1, a1) = make_account(81);
        let first_accounts = [a0];
        let second_accounts = [a1];

        let store = SpinlockGateStore::new();
        let token = store
            .with_lock(|s| s.install_with_args(&first_accounts, &P, &[]))
            .unwrap();

        // Second install while one gate is active: refused loudly (the
        // tier maps NoFreeSlot to LAMPORT_GATE_CONTENDED), never shared
        // and never corrupting the first gate.
        assert_eq!(
            store
                .with_lock(|s| s.install_with_args(&second_accounts, &P, &[]))
                .unwrap_err(),
            GateInstallError::NoFreeSlot
        );

        // The first gate still enforces, entirely under the lock.
        store.with_lock(|s| {
            assert!(s
                .check(first_accounts[0].address(), GateCheck::Lamports)
                .is_ok());
            assert_eq!(
                s.check(second_accounts[0].address(), GateCheck::Lamports),
                Err(write_policy_violation(u8::MAX))
            );
        });

        // Releasing the first gate frees the slot for the next install.
        store.with_lock(|s| s.remove(token));
        assert!(store
            .with_lock(|s| s.install_with_args(&second_accounts, &P, &[]))
            .is_ok());
    }
}
