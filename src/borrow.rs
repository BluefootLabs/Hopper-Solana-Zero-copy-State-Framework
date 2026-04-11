//! Hopper-owned borrow guards for account data.
//!
//! `Ref` and `RefMut` expose Hopper-defined borrow identities while the active
//! backend still owns the release mechanics. The runtime stores a stable pointer
//! to the borrowed data and lets the backend guard drop normally when the Hopper
//! wrapper is released.

use core::marker::PhantomData;

use crate::borrow_registry::BorrowToken;
use crate::compat::{BackendRef, BackendRefMut};
use crate::error::ProgramError;

// ── Ref (shared borrow) ─────────────────────────────────────────────

/// Shared (immutable) borrow guard for account data.
///
/// Derefs to the borrowed data. On drop, the backend releases the
/// shared borrow.
pub struct Ref<'a, T: ?Sized> {
    ptr: *const T,
    guard: BackendRef<'a, [u8]>,
    token: BorrowToken,
    _marker: PhantomData<&'a T>,
}

impl<'a> Ref<'a, [u8]> {
    /// Wrap an active-backend byte borrow into a Hopper Ref.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRef<'a, [u8]>, token: BorrowToken) -> Self {
        let ptr = (&*inner) as *const [u8];
        Self {
            ptr,
            guard: inner,
            token,
            _marker: PhantomData,
        }
    }

    /// Project a byte borrow into another view over the same underlying bytes.
    #[inline(always)]
    pub(crate) unsafe fn project<U: ?Sized>(self, ptr: *const U) -> Ref<'a, U> {
        let Self {
            guard,
            token,
            ..
        } = self;
        Ref {
            ptr,
            guard,
            token,
            _marker: PhantomData,
        }
    }

    /// Narrow a shared byte-slice borrow to a tail starting at `offset`.
    #[inline(always)]
    pub fn slice_from(self, offset: usize) -> Ref<'a, [u8]> {
        let Self { ptr, guard, token, .. } = self;
        let bytes = unsafe { &*ptr };
        Ref {
            ptr: &bytes[offset..] as *const [u8],
            guard,
            token,
            _marker: PhantomData,
        }
    }

    /// Narrow a shared byte-slice borrow to a checked sub-slice.
    #[inline(always)]
    pub fn slice(self, offset: usize, len: usize) -> Result<Ref<'a, [u8]>, ProgramError> {
        let Self { ptr, guard, token, .. } = self;
        let bytes = unsafe { &*ptr };
        let end = offset
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end > bytes.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Ref {
            ptr: &bytes[offset..end] as *const [u8],
            guard,
            token,
            _marker: PhantomData,
        })
    }

    #[inline(always)]
    pub fn as_bytes_ptr(&self) -> *const u8 {
        let bytes: &[u8] = self;
        bytes.as_ptr()
    }
}

impl<T: ?Sized> Ref<'_, T> {
    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.ptr
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
    guard: BackendRefMut<'a, [u8]>,
    token: BorrowToken,
    _marker: PhantomData<&'a mut T>,
}

impl<'a> RefMut<'a, [u8]> {
    /// Wrap an active-backend byte borrow into a Hopper RefMut.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRefMut<'a, [u8]>, token: BorrowToken) -> Self {
        let ptr = (&*inner as *const [u8]).cast_mut();
        Self {
            ptr,
            guard: inner,
            token,
            _marker: PhantomData,
        }
    }

    /// Project a mutable byte borrow into another mutable view over the same bytes.
    #[inline(always)]
    pub(crate) unsafe fn project<U: ?Sized>(self, ptr: *mut U) -> RefMut<'a, U> {
        let Self {
            guard,
            token,
            ..
        } = self;
        RefMut {
            ptr,
            guard,
            token,
            _marker: PhantomData,
        }
    }

    /// Narrow an exclusive byte-slice borrow to a tail starting at `offset`.
    #[inline(always)]
    pub fn slice_from(self, offset: usize) -> RefMut<'a, [u8]> {
        let Self { ptr, guard, token, .. } = self;
        let bytes = unsafe { &mut *ptr };
        RefMut {
            ptr: &mut bytes[offset..] as *mut [u8],
            guard,
            token,
            _marker: PhantomData,
        }
    }

    /// Narrow an exclusive byte-slice borrow to a checked sub-slice.
    #[inline(always)]
    pub fn slice(self, offset: usize, len: usize) -> Result<RefMut<'a, [u8]>, ProgramError> {
        let Self { ptr, guard, token, .. } = self;
        let bytes = unsafe { &mut *ptr };
        let end = offset
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end > bytes.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(RefMut {
            ptr: &mut bytes[offset..end] as *mut [u8],
            guard,
            token,
            _marker: PhantomData,
        })
    }

    #[inline(always)]
    pub fn as_bytes_mut_ptr(&mut self) -> *mut u8 {
        let bytes: &mut [u8] = self;
        bytes.as_mut_ptr()
    }
}

impl<T: ?Sized> RefMut<'_, T> {
    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }

    #[inline(always)]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr
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

impl<T: ?Sized> core::fmt::Debug for Ref<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ref")
            .field("ptr", &self.ptr)
            .finish_non_exhaustive()
    }
}

impl<T: ?Sized> core::fmt::Debug for RefMut<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RefMut")
            .field("ptr", &self.ptr)
            .finish_non_exhaustive()
    }
}
