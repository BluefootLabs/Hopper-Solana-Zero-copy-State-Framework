//! Segment-level borrow registry for fine-grained access control.
//!
//! The account-level [`BorrowRegistry`](crate::borrow_registry) prevents
//! aliasing across entire accounts. This module adds **segment-level**
//! conflict detection: two borrows of the *same* account are allowed when
//! their byte ranges don't overlap, or when both are read-only.
//!
//! ## Conflict Rules
//!
//! | Existing | New   | Overlapping? | Allowed |
//! |----------|-------|--------------|---------|
//! | Read     | Read  | yes          | ✅       |
//! | Read     | Write | yes          | ❌       |
//! | Write    | Read  | yes          | ❌       |
//! | Write    | Write | yes          | ❌       |
//! | *any*    | *any* | no           | ✅       |
//!
//! ## Zero-Cost Design
//!
//! - Fixed-capacity array (no heap)
//! - Inline conflict checks
//! - Deterministic iteration (bounded loop)

use core::mem::MaybeUninit;

use crate::address::Address;
use crate::error::ProgramError;

/// Maximum simultaneous segment borrows per instruction.
///
/// 16 covers any realistic instruction, most use 2-6 segments.
/// Keeping it fixed avoids heap allocation while staying well within
/// Solana's CU budget.  The compact entry representation keeps the
/// total stack footprint under 200 bytes.
pub const MAX_SEGMENT_BORROWS: usize = 16;

/// Read or write access intent for a segment borrow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum AccessKind {
    /// Shared (immutable) access.
    Read = 0,
    /// Exclusive (mutable) access.
    Write = 1,
}

/// First-8-byte prefix of an account address, used as a fast-path
/// comparator in the conflict scan.
///
/// The **audit-correct** model is fingerprint-then-verify: a hot-path
/// `u64` compare rejects unrelated accounts immediately; the slow-path
/// 32-byte compare fires only when the prefixes match. Because a
/// full-address compare always follows, fingerprint collisions produce
/// **no** false conflicts, they only cost one extra 32-byte compare
/// for the extremely rare collision pair.
#[inline(always)]
fn address_fingerprint(address: &Address) -> u64 {
    let bytes = address.as_array();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Full-identity equality check on the slow path.
#[inline(always)]
fn address_eq(a: &Address, b: &Address) -> bool {
    a.as_array() == b.as_array()
}

#[inline(always)]
fn borrow_eq(a: &SegmentBorrow, b: &SegmentBorrow) -> bool {
    a.key_fp == b.key_fp
        && address_eq(&a.key, &b.key)
        && a.offset == b.offset
        && a.size == b.size
        && a.kind == b.kind
}

/// A single active segment borrow.
///
/// Carries both a fast `u64` fingerprint and the full 32-byte account
/// address. The fingerprint is the hot-path comparator; the full
/// address resolves collisions so conflict detection is never
/// probabilistic.
#[derive(Clone, Copy, Debug)]
pub struct SegmentBorrow {
    /// Fast-path prefix of the account address.
    pub key_fp: u64,
    /// Full account address, authoritative identity, checked whenever
    /// the fast-path fingerprint matches. Pre-audit we relied on the
    /// fingerprint alone and claimed it was "collision-free for any
    /// realistic instruction"; that was probabilistic, not a guarantee.
    pub key: Address,
    /// Byte offset within the account data.
    pub offset: u32,
    /// Byte size of the borrowed segment.
    pub size: u32,
    /// Access kind (read or write).
    pub kind: AccessKind,
}

/// Check whether two byte ranges overlap.
#[inline(always)]
const fn ranges_overlap(a_off: u32, a_size: u32, b_off: u32, b_size: u32) -> bool {
    let a_end = a_off as u64 + a_size as u64;
    let b_end = b_off as u64 + b_size as u64;
    // Non-overlapping iff one ends before the other starts.
    !(a_end <= b_off as u64 || b_end <= a_off as u64)
}

/// Instruction-scoped segment borrow registry.
///
/// Tracks active segment borrows and enforces conflict rules. Designed
/// for inline use in an execution context, no heap, no dynamic dispatch.
///
/// Uses compact 8-byte address fingerprints and a flat array of
/// fixed-size entries.  Total stack footprint: ~280 bytes (vs ~1.3 KB
/// with full 32-byte addresses and Option wrappers).
///
/// # Example
///
/// ```ignore
/// let mut borrows = SegmentBorrowRegistry::new();
/// borrows.register_read(&vault_key, 0, 8)?;   // read balance
/// borrows.register_write(&vault_key, 8, 32)?;  // write metadata, OK, non-overlapping
/// borrows.register_write(&vault_key, 0, 8)?;   // REJECTED, overlaps read
/// ```
pub struct SegmentBorrowRegistry {
    entries: [MaybeUninit<SegmentBorrow>; MAX_SEGMENT_BORROWS],
    len: u8,
    /// Append-only touch log (innovation I7): unlike `entries`, which
    /// empties as RAII leases release, this records every distinct
    /// `(account, offset, size, kind)` the instruction ever registered,
    /// so end-of-handler introspection can emit an instruction touch map.
    #[cfg(feature = "touch-map")]
    touches: [MaybeUninit<SegmentBorrow>; MAX_TOUCH_RECORDS],
    #[cfg(feature = "touch-map")]
    touch_len: u8,
    /// Set when more distinct ranges were touched than the log can hold;
    /// consumers must treat the map as partial.
    #[cfg(feature = "touch-map")]
    touch_overflow: bool,
}

/// Capacity of the instruction touch log (`touch-map` feature).
#[cfg(feature = "touch-map")]
pub const MAX_TOUCH_RECORDS: usize = 32;

// ---------------------------------------------------------------------------
// Touch-map wire format v1 (`touch-map` feature)
// ---------------------------------------------------------------------------
//
// A touch map is emitted as ONE `sol_log_data` segment (it appears in the
// transaction log as a `Program data: <base64>` line) and is a **public,
// versioned wire format** — decoders exist in `hopper tx explain`, in the
// generated TypeScript client (`decodeHopperTouchMap`), and in this module's
// tests. Any change to the layout below requires bumping
// [`TOUCH_MAP_VERSION`].
//
// ```text
// byte 0            magic       = 0x7A  (TOUCH_MAP_MAGIC)
// byte 1            version     = 0x01  (TOUCH_MAP_VERSION)
// byte 2            flags       bit0 = touch log overflowed (map is partial)
//                               bit1 = one or more records were skipped by
//                                      the encoder (unmappable address or
//                                      offset >= 2^31)
//                               bits 2-7 reserved, zero in v1
// byte 3            count       number of records that follow (0..=32)
// bytes 4..4+9n     records     9 bytes each:
//   +0              slot        u8 account index into the instruction's
//                               account list
//   +1..+5          packed      u32 LE; top bit = write (1) / read (0),
//                               low 31 bits = byte offset in account data
//   +5..+9          size        u32 LE byte length of the touched range
// ```
//
// Total length is always exactly `4 + 9 * count` (<= 292 bytes). Decoders
// MUST verify magic, version, and the exact-length equation; together these
// make accidental collision with other `Program data:` payloads (e.g.
// Anchor's 8-byte sha256 event discriminators, whose first byte is
// uniformly distributed) practically impossible — a colliding payload would
// need byte0 = 0x7A, byte1 = 0x01, and a total length satisfying the count
// equation.
//
// Honesty rules: a map with flag bit0 set is PARTIAL — the instruction
// touched more distinct ranges than [`MAX_TOUCH_RECORDS`]; a map with flag
// bit1 set omitted at least one touched range it could not represent.
// Consumers must not treat such maps as a complete effect set.

/// Magic byte identifying a touch-map `sol_log_data` record ('z').
#[cfg(feature = "touch-map")]
pub const TOUCH_MAP_MAGIC: u8 = 0x7A;
/// Touch-map wire format version.
#[cfg(feature = "touch-map")]
pub const TOUCH_MAP_VERSION: u8 = 0x01;
/// Flags bit0: the touch log overflowed; the map is partial.
#[cfg(feature = "touch-map")]
pub const TOUCH_MAP_FLAG_OVERFLOWED: u8 = 1 << 0;
/// Flags bit1: the encoder skipped at least one record (address not among
/// the instruction accounts, slot index above `u8::MAX`, or offset not
/// representable in 31 bits).
#[cfg(feature = "touch-map")]
pub const TOUCH_MAP_FLAG_SKIPPED: u8 = 1 << 1;
/// Fixed header length (magic, version, flags, count).
#[cfg(feature = "touch-map")]
pub const TOUCH_MAP_HEADER_LEN: usize = 4;
/// Encoded length of one touch record.
#[cfg(feature = "touch-map")]
pub const TOUCH_MAP_RECORD_LEN: usize = 9;
/// Maximum encoded touch-map length (292 bytes).
#[cfg(feature = "touch-map")]
pub const TOUCH_MAP_MAX_ENCODED_LEN: usize =
    TOUCH_MAP_HEADER_LEN + MAX_TOUCH_RECORDS * TOUCH_MAP_RECORD_LEN;

/// One slot-resolved touch record, ready for wire encoding
/// (`touch-map` feature). Unlike [`SegmentBorrow`], the account is
/// identified by its index in the instruction's account list, which is
/// what an off-chain decoder can join against a fetched transaction.
#[cfg(feature = "touch-map")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchMapRecord {
    /// Account slot index into the instruction's account list.
    pub slot: u8,
    /// Byte offset within the account data (must fit in 31 bits).
    pub offset: u32,
    /// Byte length of the touched range.
    pub size: u32,
    /// Write access (`true`) or read access (`false`).
    pub write: bool,
}

/// Encode a touch map into the versioned v1 wire format documented above
/// (`touch-map` feature). Pure function: no syscalls, no allocation —
/// returns the fixed-capacity buffer plus the number of valid bytes.
///
/// `overflowed` must be the touch log's overflow flag so partial maps are
/// honestly marked. `skipped` must be `true` when the caller dropped any
/// record while resolving addresses to slots. Records whose `offset` does
/// not fit in 31 bits are skipped here (impossible for real Solana
/// accounts, which cap at 10 MiB) and reported via flag bit1 rather than
/// silently truncated. If more than [`MAX_TOUCH_RECORDS`] records are
/// passed, the excess is dropped and the overflow flag is set — the
/// record count byte never lies about the encoded payload.
#[cfg(feature = "touch-map")]
pub fn encode_touch_map(
    records: &[TouchMapRecord],
    overflowed: bool,
    skipped: bool,
) -> ([u8; TOUCH_MAP_MAX_ENCODED_LEN], usize) {
    let mut buf = [0u8; TOUCH_MAP_MAX_ENCODED_LEN];
    let mut flags = 0u8;
    if overflowed {
        flags |= TOUCH_MAP_FLAG_OVERFLOWED;
    }
    if skipped {
        flags |= TOUCH_MAP_FLAG_SKIPPED;
    }
    let mut count = 0usize;
    let mut pos = TOUCH_MAP_HEADER_LEN;
    for rec in records {
        if count >= MAX_TOUCH_RECORDS {
            flags |= TOUCH_MAP_FLAG_OVERFLOWED;
            break;
        }
        if rec.offset > i32::MAX as u32 {
            flags |= TOUCH_MAP_FLAG_SKIPPED;
            continue;
        }
        let packed = rec.offset | if rec.write { 0x8000_0000 } else { 0 };
        buf[pos] = rec.slot;
        buf[pos + 1..pos + 5].copy_from_slice(&packed.to_le_bytes());
        buf[pos + 5..pos + 9].copy_from_slice(&rec.size.to_le_bytes());
        pos += TOUCH_MAP_RECORD_LEN;
        count += 1;
    }
    buf[0] = TOUCH_MAP_MAGIC;
    buf[1] = TOUCH_MAP_VERSION;
    buf[2] = flags;
    buf[3] = count as u8;
    (buf, pos)
}

impl Default for SegmentBorrowRegistry {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl SegmentBorrowRegistry {
    /// Create an empty registry.
    #[inline(always)]
    pub const fn new() -> Self {
        const EMPTY: MaybeUninit<SegmentBorrow> = MaybeUninit::uninit();
        Self {
            entries: [EMPTY; MAX_SEGMENT_BORROWS],
            len: 0,
            #[cfg(feature = "touch-map")]
            touches: [EMPTY; MAX_TOUCH_RECORDS],
            #[cfg(feature = "touch-map")]
            touch_len: 0,
            #[cfg(feature = "touch-map")]
            touch_overflow: false,
        }
    }

    /// Record `borrow` in the append-only touch log, deduplicating by
    /// exact `(key, offset, size, kind)` identity so repeated sequential
    /// leases of the same range appear once.
    #[cfg(feature = "touch-map")]
    #[inline]
    fn record_touch(&mut self, borrow: &SegmentBorrow) {
        let len = self.touch_len as usize;
        let mut i = 0;
        while i < len {
            // SAFETY: `i < touch_len`, and every slot below `touch_len` was
            // initialized by this method before `touch_len` advanced.
            let existing = unsafe { self.touches.get_unchecked(i).assume_init_ref() };
            if borrow_eq(existing, borrow) {
                return;
            }
            i += 1;
        }
        if len >= MAX_TOUCH_RECORDS {
            self.touch_overflow = true;
            return;
        }
        // SAFETY: `len < MAX_TOUCH_RECORDS`, an in-bounds uninitialized slot.
        unsafe { self.touches.get_unchecked_mut(len).write(*borrow) };
        self.touch_len = (len + 1) as u8;
    }

    /// Record a whole-account borrow as a `(0, data_len)` entry in the
    /// touch log **without** registering a live-ledger entry
    /// (`touch-map` feature).
    ///
    /// Whole-account borrows (`try_borrow_mut` / `load_mut`) are
    /// governed by the account-level borrow byte, which already
    /// mutually excludes live segment leases (segment acquires take
    /// shared account borrows; the whole-account path takes the
    /// exclusive one). Their *liveness* therefore never belongs in this
    /// registry — only their cumulative footprint does. This closes the
    /// I7 blind spot where the most common access path was invisible to
    /// the touch map.
    #[cfg(feature = "touch-map")]
    #[inline]
    pub fn record_account_touch(&mut self, key: &Address, data_len: u32, kind: AccessKind) {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset: 0,
            size: data_len,
            kind,
        };
        self.record_touch(&borrow);
    }

    /// Visit every distinct range this instruction has touched so far
    /// (`touch-map` feature). Order is first-touch order. Use
    /// [`touch_map_overflowed`](Self::touch_map_overflowed) to detect a
    /// partial log.
    #[cfg(feature = "touch-map")]
    #[inline]
    pub fn for_each_touch<F: FnMut(&SegmentBorrow)>(&self, mut f: F) {
        let len = self.touch_len as usize;
        let mut i = 0;
        while i < len {
            // SAFETY: `i < touch_len`; slots below `touch_len` are initialized.
            f(unsafe { self.touches.get_unchecked(i).assume_init_ref() });
            i += 1;
        }
    }

    /// Number of distinct touch records captured (`touch-map` feature).
    #[cfg(feature = "touch-map")]
    #[inline(always)]
    pub const fn touch_map_len(&self) -> usize {
        self.touch_len as usize
    }

    /// Whether the touch log overflowed [`MAX_TOUCH_RECORDS`] and is
    /// therefore partial (`touch-map` feature).
    #[cfg(feature = "touch-map")]
    #[inline(always)]
    pub const fn touch_map_overflowed(&self) -> bool {
        self.touch_overflow
    }

    /// Number of active borrows.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the registry is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Register a new read borrow and return the `SegmentBorrow`
    /// record the caller can hand to `SegmentLease::new` for RAII
    /// release. This is the plumbing that makes
    /// [`crate::segment_lease::SegRef`] possible.
    #[inline(always)]
    pub fn register_leased_read(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrow, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Read,
        };
        self.register(borrow)?;
        Ok(borrow)
    }

    /// Mutable counterpart of [`register_leased_read`].
    #[inline(always)]
    pub fn register_leased_write(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrow, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Write,
        };
        self.register(borrow)?;
        Ok(borrow)
    }

    /// Register a new segment borrow, checking for conflicts.
    ///
    /// Returns `Err(AccountBorrowFailed)` if the new borrow overlaps an
    /// existing borrow with incompatible access (read+write or write+write)
    /// on the **same** account (full-address identity, not fingerprint).
    #[inline(always)]
    pub fn register(&mut self, new: SegmentBorrow) -> Result<(), ProgramError> {
        let len = self.len as usize;
        if len >= MAX_SEGMENT_BORROWS {
            return Err(ProgramError::AccountBorrowFailed);
        }

        // Check conflicts against all active borrows. Fast path on the
        // 8-byte fingerprint; slow path confirms with the full 32-byte
        // address so fingerprint collisions cannot manufacture false
        // conflicts between unrelated accounts.
        let mut i = 0;
        while i < len {
            // SAFETY: `i < len`, and every slot below `len` was initialized by
            // `register` before `self.len` was advanced.
            let existing = unsafe { self.entries.get_unchecked(i).assume_init_ref() };
            if existing.key_fp == new.key_fp
                && address_eq(&existing.key, &new.key)
                && ranges_overlap(existing.offset, existing.size, new.offset, new.size)
            {
                match (existing.kind, new.kind) {
                    (AccessKind::Read, AccessKind::Read) => {}
                    _ => return Err(ProgramError::AccountBorrowFailed),
                }
            }
            i += 1;
        }

        // SAFETY: Capacity was checked above, so `len` is an in-bounds
        // uninitialized slot owned by this registry.
        unsafe { self.entries.get_unchecked_mut(len).write(new) };
        self.len = (len + 1) as u8;
        // Touch map (I7): record the successful registration in the
        // append-only log. Releases never remove touch records — the log
        // is the instruction's cumulative footprint.
        #[cfg(feature = "touch-map")]
        self.record_touch(&new);
        Ok(())
    }

    /// Convenience: register a read borrow for the given account region.
    #[inline(always)]
    pub fn register_read(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        self.register(SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Read,
        })
    }

    /// Convenience: register a write borrow for the given account region.
    #[inline(always)]
    pub fn register_write(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        self.register(SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Write,
        })
    }

    /// Release a previously registered borrow.
    ///
    /// Finds the first matching entry and removes it, compacting the array.
    /// Identity is full-address (not fingerprint) to stay collision-safe.
    #[inline(always)]
    pub fn release(&mut self, borrow: &SegmentBorrow) -> bool {
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            // SAFETY: `i < len`, and all slots below `len` are initialized.
            let existing = unsafe { self.entries.get_unchecked(i).assume_init_ref() };
            if borrow_eq(existing, borrow) {
                // Swap-remove: move last entry into this slot.
                let new_len = len - 1;
                self.len = new_len as u8;
                if i < new_len {
                    // SAFETY: `new_len < len`, so the former last entry is
                    // initialized and available to move into the removed slot.
                    let last = unsafe { self.entries.get_unchecked(new_len).assume_init() };
                    // SAFETY: `i < new_len`, so the target slot is in-bounds.
                    unsafe { self.entries.get_unchecked_mut(i).write(last) };
                }
                return true;
            }
            i += 1;
        }
        false
    }

    /// Release a borrow that is expected to be the most recently registered one.
    ///
    /// The last slot is checked first for the hot RAII cleanup path. If the
    /// entry is no longer last, this falls back to exact removal instead of
    /// popping an unrelated borrow.
    ///
    /// # Safety
    ///
    /// The caller must ensure `borrow` was previously registered in this
    /// registry and that calling this does not violate any higher-level aliasing
    /// contract.
    #[doc(hidden)]
    #[inline(always)]
    pub unsafe fn release_last_registered(&mut self, borrow: &SegmentBorrow) -> bool {
        let len = self.len as usize;
        if len == 0 {
            return false;
        }
        // SAFETY: `len > 0`, and all slots below `len` are initialized.
        let last = unsafe { *self.entries.get_unchecked(len - 1).assume_init_ref() };
        if !borrow_eq(&last, borrow) {
            return self.release(borrow);
        }
        self.len = (len - 1) as u8;
        true
    }

    /// Reset the registry, clearing all active borrows.
    #[inline(always)]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Check if a proposed borrow would conflict, without registering it.
    ///
    /// Uses full-address identity, fingerprint collisions do not
    /// produce false positives.
    #[inline(always)]
    pub fn would_conflict(&self, proposed: &SegmentBorrow) -> bool {
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            // SAFETY: `i < len`, and all slots below `len` are initialized.
            let existing = unsafe { self.entries.get_unchecked(i).assume_init_ref() };
            if existing.key_fp == proposed.key_fp
                && address_eq(&existing.key, &proposed.key)
                && ranges_overlap(
                    existing.offset,
                    existing.size,
                    proposed.offset,
                    proposed.size,
                )
            {
                match (existing.kind, proposed.kind) {
                    (AccessKind::Read, AccessKind::Read) => {}
                    _ => return true,
                }
            }
            i += 1;
        }
        false
    }

    /// Register a borrow and return an RAII guard that auto-releases it on drop.
    ///
    /// This is the preferred way to acquire segment borrows, the guard
    /// ensures the borrow is released even if the caller returns early
    /// via `?` or encounters an error.
    ///
    /// # Example
    ///
    /// ```ignore
    /// {
    ///     let _guard = borrows.register_guard_write(&key, 0, 8)?;
    ///     // ... write to segment ...
    /// } // guard dropped → borrow released
    /// ```
    #[inline(always)]
    pub fn register_guard(
        &mut self,
        borrow: SegmentBorrow,
    ) -> Result<SegmentBorrowGuard<'_>, ProgramError> {
        self.register(borrow)?;
        Ok(SegmentBorrowGuard {
            registry: self,
            borrow,
        })
    }

    /// Register a read borrow with RAII auto-release.
    #[inline(always)]
    pub fn register_guard_read(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrowGuard<'_>, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Read,
        };
        self.register_guard(borrow)
    }

    /// Register a write borrow with RAII auto-release.
    #[inline(always)]
    pub fn register_guard_write(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrowGuard<'_>, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Write,
        };
        self.register_guard(borrow)
    }

    /// Visit each active borrow in registration order.
    ///
    /// Intended for diagnostics and for the `hopper explain`
    /// introspection path, never for hot-path decisions.
    #[inline]
    pub fn for_each<F: FnMut(&SegmentBorrow)>(&self, mut f: F) {
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            // SAFETY: `i < len`, and all slots below `len` are initialized.
            f(unsafe { self.entries.get_unchecked(i).assume_init_ref() });
            i += 1;
        }
    }

    /// Look up an active borrow by exact `(key, offset, size, kind)`.
    #[inline]
    pub fn find_exact(
        &self,
        key: &Address,
        offset: u32,
        size: u32,
        kind: AccessKind,
    ) -> Option<&SegmentBorrow> {
        let fp = address_fingerprint(key);
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            // SAFETY: `i < len`, and all slots below `len` are initialized.
            let e = unsafe { self.entries.get_unchecked(i).assume_init_ref() };
            if e.key_fp == fp
                && address_eq(&e.key, key)
                && e.offset == offset
                && e.size == size
                && e.kind == kind
            {
                return Some(e);
            }
            i += 1;
        }
        None
    }
}

/// RAII guard that releases a segment borrow when dropped.
///
/// Created by [`SegmentBorrowRegistry::register_guard()`] and its
/// convenience wrappers. The borrow is automatically released from the
/// registry on drop, preventing borrow leaks.
pub struct SegmentBorrowGuard<'a> {
    registry: &'a mut SegmentBorrowRegistry,
    borrow: SegmentBorrow,
}

impl<'a> SegmentBorrowGuard<'a> {
    /// Access kind of the guarded borrow.
    #[inline(always)]
    pub fn kind(&self) -> AccessKind {
        self.borrow.kind
    }

    /// Byte offset of the guarded segment.
    #[inline(always)]
    pub fn offset(&self) -> u32 {
        self.borrow.offset
    }

    /// Byte size of the guarded segment.
    #[inline(always)]
    pub fn size(&self) -> u32 {
        self.borrow.size
    }
}

impl<'a> Drop for SegmentBorrowGuard<'a> {
    fn drop(&mut self) {
        self.registry.release(&self.borrow);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn range_overlap_is_symmetric_for_arbitrary_u32s() {
        let a_off: u32 = kani::any();
        let a_size: u32 = kani::any();
        let b_off: u32 = kani::any();
        let b_size: u32 = kani::any();

        assert_eq!(
            ranges_overlap(a_off, a_size, b_off, b_size),
            ranges_overlap(b_off, b_size, a_off, a_size)
        );
    }

    #[kani::proof]
    fn overlapping_write_blocks_same_account_accesses() {
        let offset: u32 = kani::any();
        let size: u32 = kani::any();
        let delta: u32 = kani::any();
        kani::assume(offset <= 1024);
        kani::assume(size > 0 && size <= 64);
        kani::assume(delta < size);

        let key = Address::new([7u8; 32]);
        let probe_offset = offset + delta;
        let mut reg = SegmentBorrowRegistry::new();

        assert!(reg.register_write(&key, offset, size).is_ok());
        assert!(reg.register_read(&key, probe_offset, 1).is_err());
        assert!(reg.register_write(&key, probe_offset, 1).is_err());
        assert_eq!(reg.len(), 1);
    }

    #[kani::proof]
    fn overlapping_reads_are_shared_for_same_account() {
        let offset: u32 = kani::any();
        let size: u32 = kani::any();
        let delta: u32 = kani::any();
        kani::assume(offset <= 1024);
        kani::assume(size > 0 && size <= 64);
        kani::assume(delta < size);

        let key = Address::new([8u8; 32]);
        let probe_offset = offset + delta;
        let mut reg = SegmentBorrowRegistry::new();

        assert!(reg.register_read(&key, offset, size).is_ok());
        assert!(reg.register_read(&key, probe_offset, 1).is_ok());
        assert_eq!(reg.len(), 2);
    }

    #[kani::proof]
    fn fingerprint_collision_different_addresses_do_not_conflict() {
        let key_a = Address::new([9u8; 32]);
        let mut key_b_bytes = [9u8; 32];
        key_b_bytes[8] = 10;
        let key_b = Address::new(key_b_bytes);
        let mut reg = SegmentBorrowRegistry::new();

        assert_eq!(address_fingerprint(&key_a), address_fingerprint(&key_b));
        assert_ne!(key_a.as_array(), key_b.as_array());
        assert!(reg.register_write(&key_a, 0, 8).is_ok());
        assert!(reg.register_write(&key_b, 0, 8).is_ok());
        assert_eq!(reg.len(), 2);
    }

    #[kani::proof]
    fn release_removes_exact_borrow_and_preserves_others() {
        let key = Address::new([11u8; 32]);
        let mut reg = SegmentBorrowRegistry::new();

        let first = reg.register_leased_read(&key, 0, 8).unwrap();
        let second = reg.register_leased_write(&key, 8, 8).unwrap();
        assert!(reg.release(&first));

        assert_eq!(reg.len(), 1);
        assert!(reg.find_exact(&key, 0, 8, AccessKind::Read).is_none());
        assert!(reg.find_exact(&key, 8, 8, AccessKind::Write).is_some());
        assert!(reg.release(&second));
        assert!(reg.is_empty());
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Address;

    fn test_addr(seed: u8) -> Address {
        Address::new([seed; 32])
    }

    #[test]
    fn read_read_same_range_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn read_write_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 0, 8).is_err());
    }

    #[test]
    fn write_write_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 0, 8).is_err());
    }

    #[test]
    fn write_read_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_read(&key, 0, 8).is_err());
    }

    #[test]
    fn non_overlapping_write_write_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // balance: [0..8), metadata: [8..40)
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 8, 32).is_ok());
    }

    #[test]
    fn partially_overlapping_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // [0..16) and [8..24) overlap at [8..16)
        assert!(reg.register_write(&key, 0, 16).is_ok());
        assert!(reg.register_write(&key, 8, 16).is_err());
    }

    #[test]
    fn different_accounts_always_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        assert!(reg.register_write(&test_addr(1), 0, 8).is_ok());
        assert!(reg.register_write(&test_addr(2), 0, 8).is_ok());
    }

    #[test]
    fn release_then_reacquire() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(&key),
            key,
            offset: 0,
            size: 8,
            kind: AccessKind::Write,
        };
        assert!(reg.register(borrow).is_ok());
        assert!(reg.register_write(&key, 0, 8).is_err()); // conflict
        assert!(reg.release(&borrow));
        assert!(reg.register_write(&key, 0, 8).is_ok()); // now OK
    }

    #[test]
    fn release_last_registered_falls_back_to_exact_release() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        let first = reg.register_leased_read(&key, 0, 8).unwrap();
        let second = reg.register_leased_write(&key, 8, 8).unwrap();

        // SAFETY: `first` was returned by this registry and has not been
        // released yet.
        assert!(unsafe { reg.release_last_registered(&first) });
        assert_eq!(reg.len(), 1);
        assert!(reg.find_exact(&key, 0, 8, AccessKind::Read).is_none());
        assert!(reg.find_exact(&key, 8, 8, AccessKind::Write).is_some());

        // SAFETY: `second` was returned by this registry and has not been
        // released yet.
        assert!(unsafe { reg.release_last_registered(&second) });
        assert!(reg.is_empty());
    }

    #[test]
    fn capacity_limit() {
        let mut reg = SegmentBorrowRegistry::new();
        for i in 0..MAX_SEGMENT_BORROWS {
            assert!(reg.register_read(&test_addr(1), i as u32 * 8, 8).is_ok());
        }
        // One more should fail.
        assert!(reg.register_read(&test_addr(1), 256, 8).is_err());
    }

    #[test]
    fn would_conflict_does_not_mutate() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(&key, 0, 8).is_ok());
        let proposed = SegmentBorrow {
            key_fp: address_fingerprint(&key),
            key,
            offset: 0,
            size: 8,
            kind: AccessKind::Write,
        };
        assert!(reg.would_conflict(&proposed));
        assert_eq!(reg.len(), 1); // unchanged
    }

    #[test]
    fn adjacent_ranges_no_conflict() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // [0..8) and [8..16) are adjacent, not overlapping.
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 8, 8).is_ok());
    }

    // ── SegmentBorrowGuard RAII tests ────────────────────────────────
    //
    // The guard holds `&mut SegmentBorrowRegistry`, which provides
    // compile-time exclusion: the borrow checker prevents any registry
    // access while a guard is alive, giving *stronger* protection than
    // runtime conflict checks alone.  Tests verify the auto-release
    // behavior by inspecting the registry after the guard drops.

    #[test]
    fn guard_auto_releases_write_on_drop() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        {
            let _guard = reg.register_guard_write(&key, 0, 8).unwrap();
            // guard alive, registry exclusively borrowed at compile time
        }
        // After drop: slot freed, len back to 0.
        assert_eq!(reg.len(), 0);
        // Re-acquire the same range, proves release happened.
        assert!(reg.register_write(&key, 0, 8).is_ok());
    }

    #[test]
    fn guard_auto_releases_read_on_drop() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        {
            let _guard = reg.register_guard_read(&key, 0, 8).unwrap();
        }
        assert_eq!(reg.len(), 0);
        // Write now succeeds, the read borrow was released.
        assert!(reg.register_write(&key, 0, 8).is_ok());
    }

    #[test]
    fn sequential_guards_reuse_slot() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        for _ in 0..4 {
            let _guard = reg.register_guard_write(&key, 0, 8).unwrap();
            // each iteration: acquire, drop at end of loop body
        }
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn guard_accessors() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        let guard = reg.register_guard_write(&key, 16, 32).unwrap();
        assert_eq!(guard.kind(), AccessKind::Write);
        assert_eq!(guard.offset(), 16);
        assert_eq!(guard.size(), 32);
    }

    #[test]
    fn guard_then_manual_register_ok() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        {
            let _guard = reg.register_guard_write(&key, 0, 8).unwrap();
        }
        // Guard released, manual register on overlapping range works.
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert_eq!(reg.len(), 1);
    }
}

// ── Property tests ───────────────────────────────────────────────────
//
// The kani proofs above verify the overlap predicate and a few hand-
// chosen registry sequences exhaustively. These property tests carve
// *randomly generated* segment maps and assert the registry's runtime
// behaviour against an independent reference oracle, so a regression in
// the conflict scan is caught even on inputs nobody thought to write a
// unit test for. proptest is a host-only dev-dependency; this module is
// `#[cfg(test)]` and never reaches the no_std SBF build.
#[cfg(test)]
mod proptests {
    use super::*;
    use crate::Address;
    use proptest::prelude::*;

    /// A single carved write segment: `[offset, offset + size)`.
    #[derive(Debug, Clone, Copy)]
    struct Seg {
        offset: u32,
        size: u32,
    }

    /// Reference overlap check, written independently of the production
    /// `ranges_overlap` so the two can disagree and surface a bug.
    fn oracle_overlap(a: Seg, b: Seg) -> bool {
        let a_end = a.offset as u64 + a.size as u64;
        let b_end = b.offset as u64 + b.size as u64;
        (a.offset as u64) < b_end && (b.offset as u64) < a_end
    }

    // Small, dense ranges so collisions actually happen and we exercise
    // both the accept and reject paths. Sizes are >= 1 (a zero-size
    // borrow can never overlap and is not interesting here).
    fn seg_strategy() -> impl Strategy<Value = Seg> {
        (0u32..64, 1u32..16).prop_map(|(offset, size)| Seg { offset, size })
    }

    proptest! {
        /// Registering a sequence of write borrows on the *same* account
        /// must accept exactly the segments that are disjoint from every
        /// previously-accepted segment, and reject every one that
        /// overlaps an accepted segment. We replay the same decisions on
        /// an independent oracle and require they agree.
        #[test]
        fn write_borrows_match_disjointness_oracle(
            segs in proptest::collection::vec(seg_strategy(), 0..MAX_SEGMENT_BORROWS)
        ) {
            let key = Address::new([42u8; 32]);
            let mut reg = SegmentBorrowRegistry::new();
            let mut accepted: alloc_vec::Vec<Seg> = alloc_vec::Vec::new();

            for seg in segs {
                let conflicts = accepted.iter().any(|prev| oracle_overlap(*prev, seg));
                let result = reg.register_write(&key, seg.offset, seg.size);
                if conflicts {
                    prop_assert!(
                        result.is_err(),
                        "registry accepted an overlapping write {:?} against {:?}",
                        seg,
                        accepted
                    );
                } else {
                    prop_assert!(
                        result.is_ok(),
                        "registry rejected a disjoint write {:?} against {:?}",
                        seg,
                        accepted
                    );
                    accepted.push(seg);
                }
            }
            prop_assert_eq!(reg.len(), accepted.len());
        }

        /// Overlapping *reads* are always shareable: a read never
        /// conflicts with another read regardless of how the ranges are
        /// carved, so every read in the sequence must be accepted (up to
        /// the capacity bound, which the input size respects).
        #[test]
        fn read_borrows_never_conflict(
            segs in proptest::collection::vec(seg_strategy(), 0..MAX_SEGMENT_BORROWS)
        ) {
            let key = Address::new([7u8; 32]);
            let mut reg = SegmentBorrowRegistry::new();
            let n = segs.len();
            for seg in segs {
                prop_assert!(reg.register_read(&key, seg.offset, seg.size).is_ok());
            }
            prop_assert_eq!(reg.len(), n);
        }

        /// Borrows on distinct accounts never conflict, even when their
        /// byte ranges are identical: disjointness is per-account.
        #[test]
        fn distinct_accounts_never_conflict(seg in seg_strategy()) {
            let mut reg = SegmentBorrowRegistry::new();
            prop_assert!(reg.register_write(&Address::new([1u8; 32]), seg.offset, seg.size).is_ok());
            prop_assert!(reg.register_write(&Address::new([2u8; 32]), seg.offset, seg.size).is_ok());
            prop_assert_eq!(reg.len(), 2);
        }
    }

    // The registry is no_std and never allocates; the proptest oracle is
    // host-only, so a plain `std::vec::Vec` is fine for bookkeeping.
    mod alloc_vec {
        pub use std::vec::Vec;
    }
}

/// Host-only decode twin of [`encode_touch_map`], shared by the
/// round-trip tests here and in `context.rs`. Mirrors the validation an
/// off-chain consumer must perform: magic, version, and the exact-length
/// equation `len == 4 + 9 * count`.
#[cfg(all(test, feature = "touch-map"))]
pub(crate) fn decode_touch_map_for_tests(
    bytes: &[u8],
) -> Option<(u8, std::vec::Vec<TouchMapRecord>)> {
    if bytes.len() < TOUCH_MAP_HEADER_LEN {
        return None;
    }
    if bytes[0] != TOUCH_MAP_MAGIC || bytes[1] != TOUCH_MAP_VERSION {
        return None;
    }
    let flags = bytes[2];
    let count = bytes[3] as usize;
    if bytes.len() != TOUCH_MAP_HEADER_LEN + count * TOUCH_MAP_RECORD_LEN {
        return None;
    }
    let mut records = std::vec::Vec::with_capacity(count);
    for i in 0..count {
        let base = TOUCH_MAP_HEADER_LEN + i * TOUCH_MAP_RECORD_LEN;
        let packed = u32::from_le_bytes(bytes[base + 1..base + 5].try_into().unwrap());
        records.push(TouchMapRecord {
            slot: bytes[base],
            offset: packed & 0x7FFF_FFFF,
            size: u32::from_le_bytes(bytes[base + 5..base + 9].try_into().unwrap()),
            write: packed & 0x8000_0000 != 0,
        });
    }
    Some((flags, records))
}

#[cfg(all(test, feature = "touch-map"))]
mod touch_map_tests {
    use super::*;

    fn key(byte: u8) -> Address {
        Address::new([byte; 32])
    }

    #[test]
    fn touch_log_survives_release_and_dedups() {
        let mut reg = SegmentBorrowRegistry::new();

        // Two disjoint borrows on one account, one on another.
        let a = reg.register_leased_write(&key(1), 0, 8).unwrap();
        let b = reg.register_leased_read(&key(1), 8, 8).unwrap();
        let c = reg.register_leased_read(&key(2), 0, 4).unwrap();

        // Release everything (the RAII path): the live ledger empties...
        assert!(reg.release(&a));
        assert!(reg.release(&b));
        assert!(reg.release(&c));
        assert!(reg.is_empty());

        // ...but the touch log keeps the cumulative footprint.
        assert_eq!(reg.touch_map_len(), 3);
        assert!(!reg.touch_map_overflowed());

        // Re-registering an identical range (sequential lease) dedups.
        let a2 = reg.register_leased_write(&key(1), 0, 8).unwrap();
        assert_eq!(reg.touch_map_len(), 3);
        // A same-range borrow with a different kind is a distinct record.
        reg.release(&a2);
        let _a3 = reg.register_leased_read(&key(1), 0, 8).unwrap();
        assert_eq!(reg.touch_map_len(), 4);

        // First-touch order is preserved.
        let mut seen = std::vec::Vec::new();
        reg.for_each_touch(|t| seen.push((t.key, t.offset, t.size, t.kind)));
        assert_eq!(seen[0], (key(1), 0, 8, AccessKind::Write));
        assert_eq!(seen[1], (key(1), 8, 8, AccessKind::Read));
        assert_eq!(seen[2], (key(2), 0, 4, AccessKind::Read));
        assert_eq!(seen[3], (key(1), 0, 8, AccessKind::Read));
    }

    #[test]
    fn whole_account_touch_recorded_without_live_ledger_entry() {
        let mut reg = SegmentBorrowRegistry::new();

        // A segment lease and a whole-account borrow on the same account.
        let seg = reg.register_leased_write(&key(3), 16, 8).unwrap();
        reg.release(&seg);
        reg.record_account_touch(&key(3), 64, AccessKind::Write);

        // The whole-account record is footprint-only: the live ledger
        // stays empty (the account borrow byte owns its liveness), so a
        // later segment lease on the same bytes is not falsely blocked.
        assert!(reg.is_empty());
        assert!(reg.register_write(&key(3), 0, 8).is_ok());

        // Both access shapes appear in the touch map, and repeating the
        // whole-account borrow dedups.
        reg.record_account_touch(&key(3), 64, AccessKind::Write);
        assert_eq!(reg.touch_map_len(), 3);
        let mut seen = std::vec::Vec::new();
        reg.for_each_touch(|t| seen.push((t.offset, t.size, t.kind)));
        assert_eq!(seen[0], (16, 8, AccessKind::Write));
        assert_eq!(seen[1], (0, 64, AccessKind::Write));
        assert_eq!(seen[2], (0, 8, AccessKind::Write));
    }

    #[test]
    fn touch_log_flags_overflow_and_stays_partial_not_wrong() {
        let mut reg = SegmentBorrowRegistry::new();
        // Touch more distinct ranges than the log holds. Register/release
        // pairs keep the live ledger small while the touch log accumulates.
        let mut i: u32 = 0;
        while (i as usize) < MAX_TOUCH_RECORDS + 3 {
            let b = reg.register_leased_read(&key(9), i * 16, 8).unwrap();
            reg.release(&b);
            i += 1;
        }
        assert_eq!(reg.touch_map_len(), MAX_TOUCH_RECORDS);
        assert!(reg.touch_map_overflowed());
    }

    #[test]
    fn touch_map_encoder_round_trips_including_flags() {
        let records = [
            TouchMapRecord {
                slot: 0,
                offset: 16,
                size: 8,
                write: true,
            },
            TouchMapRecord {
                slot: 3,
                offset: 0,
                size: 64,
                write: false,
            },
            TouchMapRecord {
                slot: 255,
                offset: 0x7FFF_FFFF,
                size: 1,
                write: true,
            },
        ];
        let (buf, len) = encode_touch_map(&records, false, false);
        assert_eq!(len, TOUCH_MAP_HEADER_LEN + 3 * TOUCH_MAP_RECORD_LEN);
        let (flags, decoded) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags, 0);
        assert_eq!(decoded, records);

        // Overflow flag survives the round trip.
        let (buf, len) = encode_touch_map(&records, true, false);
        let (flags, decoded) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags, TOUCH_MAP_FLAG_OVERFLOWED);
        assert_eq!(decoded, records);

        // Skipped flag survives the round trip.
        let (buf, len) = encode_touch_map(&records, false, true);
        let (flags, _) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags, TOUCH_MAP_FLAG_SKIPPED);
    }

    #[test]
    fn touch_map_encoder_skips_unrepresentable_offsets_honestly() {
        let records = [
            TouchMapRecord {
                slot: 0,
                offset: 8,
                size: 8,
                write: false,
            },
            TouchMapRecord {
                slot: 1,
                offset: 0x8000_0000, // does not fit in 31 bits
                size: 8,
                write: true,
            },
        ];
        let (buf, len) = encode_touch_map(&records, false, false);
        let (flags, decoded) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags, TOUCH_MAP_FLAG_SKIPPED);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0], records[0]);
    }

    #[test]
    fn touch_map_encoder_never_lies_about_record_count() {
        // Feeding more records than the wire format can carry must mark
        // the map as overflowed, not overrun or misreport the count.
        let records = std::vec![
            TouchMapRecord {
                slot: 0,
                offset: 0,
                size: 1,
                write: false,
            };
            MAX_TOUCH_RECORDS + 2
        ];
        let (buf, len) = encode_touch_map(&records, false, false);
        assert_eq!(
            len,
            TOUCH_MAP_HEADER_LEN + MAX_TOUCH_RECORDS * TOUCH_MAP_RECORD_LEN
        );
        let (flags, decoded) = decode_touch_map_for_tests(&buf[..len]).unwrap();
        assert_eq!(flags & TOUCH_MAP_FLAG_OVERFLOWED, TOUCH_MAP_FLAG_OVERFLOWED);
        assert_eq!(decoded.len(), MAX_TOUCH_RECORDS);
    }

    #[test]
    fn touch_map_decoder_rejects_wrong_magic_version_and_length() {
        let (buf, len) = encode_touch_map(
            &[TouchMapRecord {
                slot: 0,
                offset: 4,
                size: 4,
                write: true,
            }],
            false,
            false,
        );
        assert!(decode_touch_map_for_tests(&buf[..len]).is_some());

        let mut bad_magic = buf;
        bad_magic[0] = 0x7B;
        assert!(decode_touch_map_for_tests(&bad_magic[..len]).is_none());

        let mut bad_version = buf;
        bad_version[1] = 0x02;
        assert!(decode_touch_map_for_tests(&bad_version[..len]).is_none());

        // Truncated and over-long payloads violate len == 4 + 9 * count.
        assert!(decode_touch_map_for_tests(&buf[..len - 1]).is_none());
        assert!(decode_touch_map_for_tests(&buf[..len + 9]).is_none());
    }
}
