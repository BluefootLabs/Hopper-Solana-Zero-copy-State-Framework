//! RAII-leased typed segment guards.
//!
//! [`SegmentBorrowRegistry`]
//! records live byte-range borrows. [`SegmentLease`] owns one registry entry
//! and removes it on drop. [`SegRef`] and [`SegRefMut`] pair that lease with an
//! account-data guard, allowing sequential access after the previous guard is
//! dropped while rejecting incompatible live ranges.
//!
//! For example, both mutations below are sequential because the first guard is
//! dropped before the second is acquired:
//!
//! ```ignore
//! { let mut b = ctx.segment_mut::<WireU64>(0, BAL)?; *b += amount; }
//! { let mut b = ctx.segment_mut::<WireU64>(0, BAL)?; *b += more;   }
//! ```
//!
//! ## Representation
//!
//! `SegmentLease` stores a raw pointer to the registry plus a
//! `PhantomData<&'a mut SegmentBorrowRegistry>`. A raw pointer avoids extending
//! a Rust `&mut` borrow of the entire context through the returned segment
//! guard. The lifetime marker ties the lease to the registry borrow, and `Drop`
//! performs an exact entry release without allocation.
//!
//! ## Why a wrapper, not a field on `Ref`/`RefMut`
//!
//! The canonical `hopper_runtime::Ref` / `RefMut` are kept flat on
//! Solana (`{ptr, state_ptr}` = 2 words, see `borrow.rs`). Adding a
//! registry pointer to them would expand the representation
//! for all access paths, including the whole-account `load()` path that
//! doesn't touch the segment registry. Keeping the lease as a separate
//! wrapper leaves `load()` at two words while segment access carries the
//! additional lease pointer.

use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use crate::borrow::{Ref, RefMut};
use crate::segment_borrow::{SegmentBorrow, SegmentBorrowRegistry};

// ══════════════════════════════════════════════════════════════════════
//  SegmentLease
// ══════════════════════════════════════════════════════════════════════

/// RAII lease on one registered entry in a
/// [`SegmentBorrowRegistry`].
///
/// On drop, the lease removes the registered entry via exact match.
/// It is returned wrapped inside [`SegRef`] / [`SegRefMut`]; callers
/// should not construct a `SegmentLease` directly.
///
/// # Safety invariants
///
/// The raw pointer is valid for `'a` because the lease was created
/// from a `&'a mut SegmentBorrowRegistry`. No other code writes to the
/// registry while a lease exists *from the caller's perspective*,
/// because the enclosing `SegRef<T>` / `SegRefMut<T>` owns the lease.
/// Drop runs exactly once.
pub struct SegmentLease<'a> {
    registry: *mut SegmentBorrowRegistry,
    borrow: SegmentBorrow,
    _lt: PhantomData<&'a mut SegmentBorrowRegistry>,
}

impl<'a> SegmentLease<'a> {
    /// Construct a lease from a live `&mut SegmentBorrowRegistry` and
    /// the borrow that was just registered.
    ///
    /// # Safety
    ///
    /// The caller must ensure `borrow` was registered in `registry`
    /// immediately before this call, and no path other than dropping
    /// the returned lease will remove the entry.
    ///
    /// `pub` but `#[doc(hidden)]` so cross-crate Hopper code
    /// (`hopper-core`'s `Frame`, macro-generated accessors) can build
    /// leases without rebuilding the primitive; end users of Hopper
    /// should reach for `AccountView::segment_ref` / `segment_mut`
    /// instead, which wrap this constructor safely.
    #[doc(hidden)]
    #[inline(always)]
    pub unsafe fn new(registry: &'a mut SegmentBorrowRegistry, borrow: SegmentBorrow) -> Self {
        Self {
            registry: registry as *mut _,
            borrow,
            _lt: PhantomData,
        }
    }

    /// Construct a lease from a raw registry pointer.
    ///
    /// Used by batch APIs (`AccountView::split_segments_mut`) that
    /// register several disjoint borrows against one
    /// `&'a mut SegmentBorrowRegistry` and then hand back several
    /// coexisting guards. Each guard needs its own lease, but only one
    /// `&mut` exists. The batch helper takes the registry's raw pointer
    /// once and binds every lease's lifetime to that single `&'a mut`.
    ///
    /// # Safety
    ///
    /// `registry` must point to a `SegmentBorrowRegistry` borrowed
    /// mutably for `'a` (the caller holds the `&'a mut`), `borrow` must
    /// have been registered in it immediately before, and no path other
    /// than dropping the returned lease may remove that entry.
    #[doc(hidden)]
    #[inline(always)]
    pub unsafe fn from_raw(registry: *mut SegmentBorrowRegistry, borrow: SegmentBorrow) -> Self {
        Self {
            registry,
            borrow,
            _lt: PhantomData,
        }
    }

    /// The borrow entry this lease owns, for diagnostics.
    ///
    /// Inherent diagnostic accessor returning the owned `SegmentBorrow` record,
    /// not `core::borrow::Borrow` (whose blanket reflexive impl has a different
    /// shape); the name reads naturally at call sites.
    #[allow(clippy::should_implement_trait)]
    #[inline(always)]
    pub fn borrow(&self) -> &SegmentBorrow {
        &self.borrow
    }
}

impl<'a> Drop for SegmentLease<'a> {
    #[inline(always)]
    fn drop(&mut self) {
        // SAFETY: `_lt` pins `'a` to the registry borrow. The pointer remains
        // valid for the full lifetime of `self`, and exact release removes only
        // this lease's registered entry.
        unsafe {
            (*self.registry).release(&self.borrow);
        }
    }
}

impl<'a> core::fmt::Debug for SegmentLease<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegmentLease")
            .field("borrow", &self.borrow)
            .finish_non_exhaustive()
    }
}

// ══════════════════════════════════════════════════════════════════════
//  SegRef / SegRefMut
// ══════════════════════════════════════════════════════════════════════

/// Shared typed segment guard: a [`Ref<T>`](crate::borrow::Ref) paired
/// with a [`SegmentLease`] that releases the registry entry on drop.
///
/// `SegRef<T>` derefs to `T`, so call sites written against the
/// previous `Ref<T>`-returning signatures compile unchanged in the
/// vast majority of cases (pattern bindings that explicitly named
/// `Ref<'_, T>` need the one-word substitution to `SegRef<'_, T>`).
pub struct SegRef<'a, T: ?Sized> {
    inner: Ref<'a, T>,
    lease: SegmentLease<'a>,
}

impl<'a, T: ?Sized> SegRef<'a, T> {
    /// Assemble a `SegRef` from a pre-built inner guard and lease.
    ///
    /// Doc-hidden public constructor for cross-crate use (Frame,
    /// generated accessors). Prefer `AccountView::segment_ref` /
    /// `Context::segment_ref` / `Frame::segment_ref` in user code.
    #[doc(hidden)]
    #[inline(always)]
    pub fn new(inner: Ref<'a, T>, lease: SegmentLease<'a>) -> Self {
        Self { inner, lease }
    }

    /// Consume the guard and return the underlying pointer.
    ///
    /// The lease and account-level borrow are still released on drop
    /// of the returned components; this escape hatch is provided for
    /// rare generic plumbing.
    #[inline(always)]
    pub fn into_parts(self) -> (Ref<'a, T>, SegmentLease<'a>) {
        (self.inner, self.lease)
    }

    /// Raw `*const T` of the borrowed data.
    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.inner.as_ptr()
    }

    /// Access the underlying `Ref<T>` without dropping the lease.
    #[inline(always)]
    pub fn inner(&self) -> &Ref<'a, T> {
        &self.inner
    }
}

impl<T: ?Sized> Deref for SegRef<'_, T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: ?Sized> core::fmt::Debug for SegRef<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegRef")
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

/// Exclusive typed segment guard.
///
/// Mirror of [`SegRef`] for the mutable path. Derefs mutably to `T`.
pub struct SegRefMut<'a, T: ?Sized> {
    inner: RefMut<'a, T>,
    lease: SegmentLease<'a>,
}

impl<'a, T: ?Sized> SegRefMut<'a, T> {
    /// Assemble a `SegRefMut` from a pre-built inner guard and lease.
    ///
    /// Doc-hidden public constructor, see [`SegRef::new`].
    #[doc(hidden)]
    #[inline(always)]
    pub fn new(inner: RefMut<'a, T>, lease: SegmentLease<'a>) -> Self {
        Self { inner, lease }
    }

    /// Consume the guard and return its parts.
    #[inline(always)]
    pub fn into_parts(self) -> (RefMut<'a, T>, SegmentLease<'a>) {
        (self.inner, self.lease)
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.inner.as_ptr()
    }

    #[inline(always)]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.inner.as_mut_ptr()
    }
}

impl<T: ?Sized> Deref for SegRefMut<'_, T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: ?Sized> DerefMut for SegRefMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: ?Sized> core::fmt::Debug for SegRefMut<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegRefMut")
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

// ══════════════════════════════════════════════════════════════════════
//  SegmentsMut, simultaneous disjoint mutable segment access
// ══════════════════════════════════════════════════════════════════════

/// A guard over **several disjoint typed sub-ranges** of one account,
/// returned by [`AccountView::split_segments_mut`](crate::AccountView::split_segments_mut).
///
/// It holds a single exclusive byte borrow of the account plus `N`
/// registry leases (one per range) that proved pairwise disjointness at
/// construction and release on drop. Because the ranges are disjoint and
/// all sit inside the one borrow, [`all_mut`](Self::all_mut) can hand out
/// `N` independent `&mut T` simultaneously. This is the generalized
/// `split_at_mut` for account fields that ordinary `segment_mut` cannot
/// express.
///
/// Construction must go through the account's checked split API. Raw offsets
/// cannot be supplied to a safe constructor:
///
/// ```compile_fail,E0624
/// use hopper_runtime::{RefMut, SegmentLease, SegmentsMut};
/// fn unchecked<'a>(data: RefMut<'a, [u8]>, leases: [SegmentLease<'a>; 2]) {
///     let _ = SegmentsMut::<[u8; 8], 2>::new(data, [0, 0], leases);
/// }
/// ```
pub struct SegmentsMut<'a, T, const N: usize> {
    data: RefMut<'a, [u8]>,
    offsets: [usize; N],
    // Leases live for the guard; dropping them releases the registry
    // entries. Order of field drops doesn't matter (independent ranges).
    _leases: [SegmentLease<'a>; N],
    _t: PhantomData<fn() -> T>,
}

impl<'a, T: crate::Pod, const N: usize> SegmentsMut<'a, T, N> {
    /// Assemble the guard. Doc-hidden; built by `split_segments_mut`.
    #[doc(hidden)]
    #[inline(always)]
    pub(crate) fn new(
        data: RefMut<'a, [u8]>,
        offsets: [usize; N],
        leases: [SegmentLease<'a>; N],
    ) -> Self {
        Self {
            data,
            offsets,
            _leases: leases,
            _t: PhantomData,
        }
    }

    /// Number of disjoint segments held.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        N
    }

    /// Whether this split contains no ranges (`N == 0`).
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        N == 0
    }

    /// Mutably access one segment by batch index.
    #[inline(always)]
    pub fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        let off = *self.offsets.get(i)?;
        let base = self.data.as_bytes_mut_ptr();
        // SAFETY: `off` was bounds- and size-validated for `T` at
        // construction; the byte borrow backing `base` is exclusive and
        // live for `&mut self`.
        Some(unsafe { &mut *(base.add(off) as *mut T) })
    }

    /// Borrow **all** segments mutably at once as `[&mut T; N]`.
    ///
    /// Sound because the offsets are pairwise disjoint (proven by the
    /// registry at construction) and every range lies inside the single
    /// exclusive byte borrow, so the references never alias.
    #[inline(always)]
    pub fn all_mut(&mut self) -> [&mut T; N] {
        let base = self.data.as_bytes_mut_ptr();
        let offsets = self.offsets;
        // Manual MaybeUninit fill (avoids `core::array::from_fn`, keeping
        // codegen on the conservative SBPF version for broad deployability).
        // SAFETY: array of `MaybeUninit` is valid uninitialized.
        let mut out: [core::mem::MaybeUninit<&mut T>; N] =
            unsafe { core::mem::MaybeUninit::uninit().assume_init() };
        let mut i = 0;
        while i < N {
            // SAFETY: ranges are disjoint and validated; `base` is a live
            // exclusive byte borrow, so each typed pointer is unique and
            // non-overlapping.
            let r: &mut T = unsafe { &mut *(base.add(offsets[i]) as *mut T) };
            out[i] = core::mem::MaybeUninit::new(r);
            i += 1;
        }
        // SAFETY: all N slots initialized above.
        unsafe {
            let init = core::ptr::read(&out as *const _ as *const [&mut T; N]);
            // The `MaybeUninit` array does not drop its contents; the forget
            // documents that ownership moved into `init` via the read above.
            #[allow(clippy::forget_non_drop)]
            core::mem::forget(out);
            init
        }
    }
}

impl<'a, T, const N: usize> core::fmt::Debug for SegmentsMut<'a, T, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegmentsMut")
            .field("offsets", &self.offsets)
            .finish_non_exhaustive()
    }
}
