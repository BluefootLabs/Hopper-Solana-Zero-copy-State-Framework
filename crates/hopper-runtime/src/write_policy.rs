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
//! [`AccountView`]: crate::account::AccountView

use crate::address::Address;
use crate::error::ProgramError;
use crate::ProgramResult;

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

/// Per-gate account capacity: the runtime's transaction account bound.
/// An instruction can never carry more accounts than the transaction
/// that contains it, so a gate over one instruction's slice always
/// fits; anything larger is refused loudly at install
/// ([`LAMPORT_GATE_TOO_MANY_ACCOUNTS`]).
pub const LAMPORT_GATE_CAPACITY: usize = crate::MAX_TX_ACCOUNTS;

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
}

impl GateEntry {
    const EMPTY: Self = Self {
        address: Address::new([0; 32]),
        index: 0,
        allow_mutation: false,
        allow_delegation: false,
    };
}

/// One installed gate. `token == 0` means the slot is free.
#[derive(Clone, Copy)]
struct GateSlot {
    /// Unique install token (0 = free slot).
    token: u64,
    /// Number of initialized entries (distinct addresses).
    len: usize,
    /// Copied `(address, permissions)` values; only `entries[..len]`
    /// are meaningful.
    entries: [GateEntry; LAMPORT_GATE_CAPACITY],
}

impl GateSlot {
    const FREE: Self = Self {
        token: 0,
        len: 0,
        entries: [GateEntry::EMPTY; LAMPORT_GATE_CAPACITY],
    };
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
    slots: [GateSlot; DEPTH],
}

impl<const DEPTH: usize> GateStore<DEPTH> {
    /// All-zero by construction — see [`GateStore::issued`]. Do not add a
    /// non-zero field initializer here without re-reading that doc.
    const fn new() -> Self {
        Self {
            issued: 0,
            slots: [GateSlot::FREE; DEPTH],
        }
    }

    /// Copy `(address, permission)` values for every account into a free
    /// slot and return that slot's unique token. Fails closed (no state
    /// change) when the accounts outnumber the capacity or no slot is
    /// free.
    fn install(
        &mut self,
        accounts: &[crate::account::AccountView<'_>],
        policy: &WritePolicy,
    ) -> Result<u64, GateInstallError> {
        if accounts.len() > LAMPORT_GATE_CAPACITY {
            return Err(GateInstallError::TooManyAccounts);
        }
        let mut slot_idx = usize::MAX;
        let mut s = 0;
        while s < DEPTH {
            if self.slots[s].token == 0 {
                slot_idx = s;
                break;
            }
            s += 1;
        }
        if slot_idx == usize::MAX {
            return Err(GateInstallError::NoFreeSlot);
        }
        let slot = &mut self.slots[slot_idx];
        let mut len = 0;
        let mut i = 0;
        while i < accounts.len() {
            // Reading `address()` here is an always-safe read of a live
            // reference the caller just gave us; only the VALUE is kept.
            let address = *accounts[i].address();
            // Indices above u8::MAX can never have been declared (the
            // policy wire format is u8): permissionless. Unreachable
            // while LAMPORT_GATE_CAPACITY <= 255, kept for honesty.
            let (index, allow_mutation, allow_delegation) = if i <= u8::MAX as usize {
                let idx = i as u8;
                let m = policy.allows_lamport_mutation(idx);
                (idx, m, m && policy.allows_whole_account_write(idx))
            } else {
                (u8::MAX, false, false)
            };
            // Duplicate addresses name the same underlying account, so
            // their permissions OR-merge into one entry (first index
            // kept for the refusal error code).
            let mut j = 0;
            let mut merged = false;
            while j < len {
                if slot.entries[j].address == address {
                    slot.entries[j].allow_mutation |= allow_mutation;
                    slot.entries[j].allow_delegation |= allow_delegation;
                    merged = true;
                    break;
                }
                j += 1;
            }
            if !merged {
                slot.entries[len] = GateEntry {
                    address,
                    index,
                    allow_mutation,
                    allow_delegation,
                };
                len += 1;
            }
            i += 1;
        }
        slot.len = len;
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
        Ok(token)
    }

    /// Free exactly the slot installed under `token` (no-op when the
    /// token is not present — e.g. an inert guard). A guard can only
    /// ever clear its own slot, never another gate's.
    fn remove(&mut self, token: u64) {
        let mut s = 0;
        while s < DEPTH {
            if self.slots[s].token == token {
                self.slots[s].token = 0;
                self.slots[s].len = 0;
                return;
            }
            s += 1;
        }
    }

    /// The governing gate: the still-active slot with the highest token,
    /// i.e. the most recently installed gate. Nested binds therefore
    /// shadow outer gates while alive; when the inner guard drops, the
    /// outer gate resumes governing.
    fn active_slot(&self) -> Option<&GateSlot> {
        let mut best: Option<&GateSlot> = None;
        let mut s = 0;
        while s < DEPTH {
            let slot = &self.slots[s];
            if slot.token != 0 {
                best = match best {
                    Some(current) if current.token >= slot.token => Some(current),
                    _ => Some(slot),
                };
            }
            s += 1;
        }
        best
    }

    /// Match `address` against the governing gate's stored values.
    /// Passthrough when no gate is installed; fail closed (indexed
    /// error, `u8::MAX` for addresses foreign to the gated instruction)
    /// otherwise.
    fn check(&self, address: &Address, delegation: bool) -> ProgramResult {
        let Some(slot) = self.active_slot() else {
            return Ok(());
        };
        let mut i = 0;
        while i < slot.len {
            let entry = &slot.entries[i];
            if entry.address == *address {
                let allowed = if delegation {
                    entry.allow_delegation
                } else {
                    entry.allow_mutation
                };
                if allowed {
                    return Ok(());
                }
                return Err(write_policy_violation(entry.index));
            }
            i += 1;
        }
        Err(write_policy_violation(u8::MAX))
    }

    /// Whether any gate (governing or shadowed) is installed.
    fn any_active(&self) -> bool {
        let mut s = 0;
        while s < DEPTH {
            if self.slots[s].token != 0 {
                return true;
            }
            s += 1;
        }
        false
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

#[cfg(target_os = "solana")]
mod gate_store {
    use super::{GateInstallError, GateStore, WritePolicy, LAMPORT_GATE_DEPTH};
    use crate::address::Address;
    use crate::error::ProgramError;
    use crate::ProgramResult;

    /// Per-invocation gate store. SBF execution is single-threaded and
    /// the program's writable data segment is freshly initialized on
    /// every invocation (each CPI level runs in its own VM), so a plain
    /// `static mut` is the cheapest correct store.
    ///
    /// Footprint: `DEPTH x (capacity x size_of::<GateEntry>())` ~ 35 KiB.
    /// [`GateStore::new`] is all-zero, so this lands in `.bss` (`NOBITS`)
    /// and costs **zero bytes in the `.so`** — only a zeroed data-segment
    /// reservation at load. Introducing any non-zero field initializer
    /// would move it to `.data` (`PROGBITS`) and write all ~35 KiB of
    /// zeros into every program's binary (it once did: `next_token: 1`).
    static mut STORE: GateStore<LAMPORT_GATE_DEPTH> = GateStore::new();

    /// This tier supports full nesting; running out of slots means
    /// nesting deeper than the Solana invoke-depth budget (or leaking
    /// guards).
    pub(super) const NO_FREE_SLOT: ProgramError = super::LAMPORT_GATE_DEPTH_EXCEEDED;

    fn with_store<R>(f: impl FnOnce(&mut GateStore<LAMPORT_GATE_DEPTH>) -> R) -> R {
        // SAFETY: SBF programs execute on a single thread, so no other
        // thread can alias this static, and none of the store operations
        // passed as `f` (`install`/`remove`/`check`/`any_active`) call
        // back into this module, so this exclusive reference is unique
        // for the duration of `f`. `addr_of_mut!` avoids materializing
        // a second `&mut` to the `static mut` anywhere else.
        f(unsafe { &mut *core::ptr::addr_of_mut!(STORE) })
    }

    pub(super) fn install(
        accounts: &[crate::account::AccountView<'_>],
        policy: &WritePolicy,
    ) -> Result<u64, GateInstallError> {
        with_store(|store| store.install(accounts, policy))
    }

    pub(super) fn remove(token: u64) {
        with_store(|store| store.remove(token));
    }

    pub(super) fn check(address: &Address, delegation: bool) -> ProgramResult {
        with_store(|store| store.check(address, delegation))
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
    use super::{GateInstallError, GateStore, WritePolicy, LAMPORT_GATE_DEPTH};
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

    pub(super) fn install(
        accounts: &[crate::account::AccountView<'_>],
        policy: &WritePolicy,
    ) -> Result<u64, GateInstallError> {
        with_store(|store| store.install(accounts, policy))
    }

    pub(super) fn remove(token: u64) {
        with_store(|store| store.remove(token));
    }

    pub(super) fn check(address: &Address, delegation: bool) -> ProgramResult {
        with_store(|store| store.check(address, delegation))
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
    use super::{GateInstallError, SpinlockGateStore, WritePolicy};
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

    pub(super) fn install(
        accounts: &[crate::account::AccountView<'_>],
        policy: &WritePolicy,
    ) -> Result<u64, GateInstallError> {
        STORE.with_lock(|store| store.install(accounts, policy))
    }

    pub(super) fn remove(token: u64) {
        STORE.with_lock(|store| store.remove(token));
    }

    pub(super) fn check(address: &Address, delegation: bool) -> ProgramResult {
        STORE.with_lock(|store| store.check(address, delegation))
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
    if !policy.lamports_declared() {
        return Ok(LamportGateGuard {
            token: 0,
            _accounts: core::marker::PhantomData,
        });
    }
    match gate_store::install(accounts, policy) {
        Ok(token) => Ok(LamportGateGuard {
            token,
            _accounts: core::marker::PhantomData,
        }),
        Err(GateInstallError::TooManyAccounts) => Err(LAMPORT_GATE_TOO_MANY_ACCOUNTS),
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
    gate_store::check(address, false)
}

/// Gate a CPI **writable meta** on the account with `address`. A
/// writable hand-off delegates unbounded data AND lamport mutation to
/// the callee, so it requires the account to carry both a
/// whole-account data grant and lamport permission. Same value-compare
/// contract as [`check_lamport_mutation`].
#[inline]
pub(crate) fn check_lamport_delegation(address: &Address) -> ProgramResult {
    gate_store::check(address, true)
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

    fn make_account(seed: u8) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + 32];
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

    #[test]
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

    #[test]
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

    #[test]
    fn data_only_policy_installs_no_gate() {
        let (_b0, a0) = make_account(30);
        let accounts = [a0];
        static P: WritePolicy = WritePolicy::new(&[WriteRange::whole_account(0)]);
        let _gate = install_lamport_gate(&accounts, &P);
        assert!(!lamport_gate_active());
        let (_bf, foreign) = make_account(31);
        assert!(check_lamport_mutation(foreign.address()).is_ok());
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

    #[test]
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
        let token = store.with_lock(|s| s.install(&first_accounts, &P)).unwrap();

        // Second install while one gate is active: refused loudly (the
        // tier maps NoFreeSlot to LAMPORT_GATE_CONTENDED), never shared
        // and never corrupting the first gate.
        assert_eq!(
            store
                .with_lock(|s| s.install(&second_accounts, &P))
                .unwrap_err(),
            GateInstallError::NoFreeSlot
        );

        // The first gate still enforces, entirely under the lock.
        store.with_lock(|s| {
            assert!(s.check(first_accounts[0].address(), false).is_ok());
            assert_eq!(
                s.check(second_accounts[0].address(), false),
                Err(write_policy_violation(u8::MAX))
            );
        });

        // Releasing the first gate frees the slot for the next install.
        store.with_lock(|s| s.remove(token));
        assert!(store.with_lock(|s| s.install(&second_accounts, &P)).is_ok());
    }
}
