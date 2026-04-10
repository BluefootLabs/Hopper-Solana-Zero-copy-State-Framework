//! Hopper-owned borrow guards for account data.
//!
//! `Ref` and `RefMut` expose Hopper-defined borrow identities while the active
//! backend still owns the release mechanics. The runtime stores a stable pointer
//! to the borrowed data and lets the backend guard drop normally when the Hopper
//! wrapper is released.

use core::marker::PhantomData;

use crate::compat::{BackendRef, BackendRefMut};

// ── Ref (shared borrow) ─────────────────────────────────────────────

/// Shared (immutable) borrow guard for account data.
///
/// Derefs to the borrowed data. On drop, the backend releases the
/// shared borrow.
pub struct Ref<'a, T: ?Sized> {
    ptr: *const T,
    guard: BackendRef<'a, T>,
    _marker: PhantomData<&'a T>,
}

impl<'a, T: ?Sized> Ref<'a, T> {
    /// Wrap an active-backend Ref into a Hopper Ref.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRef<'a, T>) -> Self {
        let ptr = (&*inner) as *const T;
        Self {
            ptr,
            guard: inner,
            _marker: PhantomData,
        }
    }
}

impl<T: ?Sized> core::ops::Deref for Ref<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        let _ = &self.guard;
        unsafe { &*self.ptr }
    }
}

// ── RefMut (exclusive borrow) ───────────────────────────────────────

/// Exclusive (mutable) borrow guard for account data.
///
/// Derefs to the borrowed data. On drop, the backend releases the
/// exclusive borrow.
pub struct RefMut<'a, T: ?Sized> {
    ptr: *mut T,
    guard: BackendRefMut<'a, T>,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> RefMut<'a, T> {
    /// Wrap an active-backend RefMut into a Hopper RefMut.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRefMut<'a, T>) -> Self {
        let ptr = (&*inner as *const T).cast_mut();
        Self {
            ptr,
            guard: inner,
            _marker: PhantomData,
        }
    }
}

impl<T: ?Sized> core::ops::Deref for RefMut<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        let _ = &self.guard;
        unsafe { &*self.ptr }
    }
}

impl<T: ?Sized> core::ops::DerefMut for RefMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        let _ = &mut self.guard;
        unsafe { &mut *self.ptr }
    }
}
