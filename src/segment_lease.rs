//! RAII-leased typed segment guards.
//!
//! The Hopper Safety Audit called out that `SegmentBorrowRegistry` was
//! being used as an **instruction-sticky ledger**: every `segment_ref` /
//! `segment_mut` call appended an entry, nothing ever released, and the
//! entries outlived the returned `Ref<T>` / `RefMut<T>` for the rest of
//! the instruction. That model makes legitimate sequential patterns
//! like
//!
//! ```ignore
//! { let mut b = ctx.segment_mut::<WireU64>(0, BAL)?; *b += amount; }
//! { let mut b = ctx.segment_mut::<WireU64>(0, BAL)?; *b += more;   }
//! ```
//!
//! impossible inside one instruction, because the second call would
//! collide with the lingering entry from the first.
//!
//! [`SegmentLease`], [`SegRef`], and [`SegRefMut`] replace that model
//! with real RAII: the registry entry lives exactly as long as the
//! returned typed guard, and dropping the guard releases the entry.
//! Sequential non-overlapping (and sequential same-region-read-then-
//! write) patterns now behave exactly the way Rust borrowers expect.
//!
//! ## Representation
//!
//! `SegmentLease` stores a raw pointer to the registry plus a
//! `PhantomData<&'a mut SegmentBorrowRegistry>`. Raw is necessary
//! because the returned `SegRef<T>` otherwise exclusively borrows the
//! whole `Context`, which would prevent even reading *another* account
//!, a regression far worse than the sticky behavior we are fixing.
//! The `PhantomData` ties the lease's lifetime to the registry's, so
//! use-after-free is impossible at the type level. Drop performs a
//! single swap-remove; no allocation, no heap touch.
//!
//! ## Why a wrapper, not a field on `Ref`/`RefMut`
//!
//! The canonical `hopper_runtime::Ref` / `RefMut` are kept flat on
//! Solana (`{ptr, state_ptr}` = 2 words, see `borrow.rs`). Adding a
//! registry pointer to them would re-inflate the flat representation
//! for every access path, even the whole-account `load()` path that
//! doesn't touch the segment registry. Keeping the lease as a separate
//! wrapper means `load()` stays at 2 words and only segment access
//! pays for the lease (one extra pointer-word on Solana).

use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use crate::borrow::{Ref, RefMut};
use crate::segment_borrow::{SegmentBorrow, SegmentBorrowRegistry};

// ══════════════════════════════════════════════════════════════════════
//  SegmentLease
// ══════════════════════════════════════════════════════════════════════

/// RAII lease on one registered entry in a
/// [`SegmentBorrowRegistry`](crate::segment_borrow::SegmentBorrowRegistry).
///
/// On drop, the lease removes the registered entry via swap-remove.
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
    pub unsafe fn new(
        registry: &'a mut SegmentBorrowRegistry,
        borrow: SegmentBorrow,
    ) -> Self {
        Self {
            registry: registry as *mut _,
            borrow,
            _lt: PhantomData,
        }
    }

    /// The borrow entry this lease owns, for diagnostics.
    #[inline(always)]
    pub fn borrow(&self) -> &SegmentBorrow {
        &self.borrow
    }
}

impl<'a> Drop for SegmentLease<'a> {
    #[inline(always)]
    fn drop(&mut self) {
        // SAFETY: `_lt` pins `'a` to the registry's borrow; the pointer
        // is valid for the full lifetime of `self`. `release` is a
        // bounded-array swap-remove, no allocation, no panic path.
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
        &*self.inner
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
        &*self.inner
    }
}

impl<T: ?Sized> DerefMut for SegRefMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        &mut *self.inner
    }
}

impl<T: ?Sized> core::fmt::Debug for SegRefMut<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegRefMut")
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}
