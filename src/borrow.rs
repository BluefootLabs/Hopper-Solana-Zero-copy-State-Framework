//! Hopper-owned borrow guards for account data.
//!
//! `Ref` and `RefMut` are transparent wrappers over backend borrow
//! guards. They provide RAII borrow tracking through the backend
//! implementation while exposing a Hopper-owned type surface.

use crate::compat::{BackendRef, BackendRefMut};

// ── Ref (shared borrow) ─────────────────────────────────────────────

/// Shared (immutable) borrow guard for account data.
///
/// Derefs to the borrowed data. On drop, the backend releases the
/// shared borrow.
#[repr(transparent)]
pub struct Ref<'a, T: ?Sized> {
    inner: BackendRef<'a, T>,
}

impl<'a, T: ?Sized> Ref<'a, T> {
    /// Wrap an active-backend Ref into a Hopper Ref.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRef<'a, T>) -> Self {
        Self { inner }
    }
}

impl<T: ?Sized> core::ops::Deref for Ref<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        &self.inner
    }
}

// ── RefMut (exclusive borrow) ───────────────────────────────────────

/// Exclusive (mutable) borrow guard for account data.
///
/// Derefs to the borrowed data. On drop, the backend releases the
/// exclusive borrow.
#[repr(transparent)]
pub struct RefMut<'a, T: ?Sized> {
    inner: BackendRefMut<'a, T>,
}

impl<'a, T: ?Sized> RefMut<'a, T> {
    /// Wrap an active-backend RefMut into a Hopper RefMut.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRefMut<'a, T>) -> Self {
        Self { inner }
    }
}

impl<T: ?Sized> core::ops::Deref for RefMut<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: ?Sized> core::ops::DerefMut for RefMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}
