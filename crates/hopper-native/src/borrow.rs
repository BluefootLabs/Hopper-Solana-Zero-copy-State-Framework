//! Deterministic borrow guards for account data.
//!
//! `Ref` and `RefMut` provide RAII borrow tracking on the `borrow_state`
//! field of `RuntimeAccount`. When dropped, they restore the borrow
//! state, preventing use-after-free and double-mutable-borrow bugs.
//!
//! These replace `core::cell::RefCell` without requiring alloc.

use crate::NOT_BORROWED;

/// Shared (immutable) borrow guard for account data.
///
/// On drop, decrements the borrow count in `RuntimeAccount.borrow_state`.
pub struct Ref<'a, T: ?Sized> {
    value: &'a T,
    state: *mut u8,
}

impl<'a, T: ?Sized> Ref<'a, T> {
    /// Create a new shared borrow guard.
    ///
    /// The caller must have already incremented `*state` to reflect
    /// the new shared borrow.
    #[inline(always)]
    pub(crate) fn new(value: &'a T, state: *mut u8) -> Self {
        Self { value, state }
    }

    /// Create a shared guard whose aliasing is enforced outside the native
    /// account borrow byte.
    ///
    /// Runtime segment access uses this after `SegmentBorrowRegistry` has
    /// leased the exact byte range. Drop must therefore avoid changing the
    /// whole-account `borrow_state` byte.
    #[inline(always)]
    pub(crate) fn new_external(value: &'a T) -> Self {
        Self {
            value,
            state: core::ptr::null_mut(),
        }
    }

    /// Create a shared borrow guard from raw parts.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The borrow state at `state` was already incremented
    /// - `value` is valid for lifetime `'a`
    /// - `state` points to a valid `RuntimeAccount.borrow_state`
    #[inline(always)]
    pub unsafe fn from_raw_parts(value: &'a T, state: *mut u8) -> Self {
        Self { value, state }
    }

    /// Decompose into raw parts without running the destructor.
    ///
    /// The caller takes responsibility for eventually releasing the
    /// borrow (decrementing `*state`).
    #[inline(always)]
    pub fn into_raw_parts(self) -> (&'a T, *mut u8) {
        let value = self.value;
        let state = self.state;
        core::mem::forget(self);
        (value, state)
    }
}

impl<T: ?Sized> core::ops::Deref for Ref<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: ?Sized> Drop for Ref<'_, T> {
    fn drop(&mut self) {
        if self.state.is_null() {
            return;
        }
        // SAFETY: state points to RuntimeAccount.borrow_state in the
        // BPF input buffer. We decrement the shared borrow count,
        // restoring NOT_BORROWED when the last shared borrow is released.
        unsafe {
            let current = *self.state;
            if current == 1 {
                *self.state = NOT_BORROWED;
            } else {
                *self.state = current - 1;
            }
        }
    }
}

/// Exclusive (mutable) borrow guard for account data.
///
/// On drop, restores `RuntimeAccount.borrow_state` to `NOT_BORROWED`.
pub struct RefMut<'a, T: ?Sized> {
    value: &'a mut T,
    state: *mut u8,
}

impl<'a, T: ?Sized> RefMut<'a, T> {
    /// Create a new exclusive borrow guard.
    ///
    /// The caller must have already set `*state = 0` to indicate
    /// exclusive borrow.
    #[inline(always)]
    pub(crate) fn new(value: &'a mut T, state: *mut u8) -> Self {
        Self { value, state }
    }

    /// Create an exclusive guard whose aliasing is enforced by an external
    /// segment lease rather than the whole-account borrow byte.
    #[inline(always)]
    pub(crate) fn new_external(value: &'a mut T) -> Self {
        Self {
            value,
            state: core::ptr::null_mut(),
        }
    }

    /// Create an exclusive borrow guard from raw parts.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The borrow state at `state` was set to 0 (exclusive)
    /// - `value` is valid and unique for lifetime `'a`
    /// - `state` points to a valid `RuntimeAccount.borrow_state`
    #[inline(always)]
    pub unsafe fn from_raw_parts(value: &'a mut T, state: *mut u8) -> Self {
        Self { value, state }
    }

    /// Decompose into raw parts without running the destructor.
    ///
    /// The caller takes responsibility for eventually releasing the
    /// borrow (restoring `*state` to `NOT_BORROWED`).
    #[inline(always)]
    pub fn into_raw_parts(self) -> (&'a mut T, *mut u8) {
        let manual = core::mem::ManuallyDrop::new(self);
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let value = unsafe { core::ptr::read(&manual.value) };
        let state = manual.state;
        (value, state)
    }
}

impl<T: ?Sized> core::ops::Deref for RefMut<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: ?Sized> core::ops::DerefMut for RefMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<T: ?Sized> Drop for RefMut<'_, T> {
    fn drop(&mut self) {
        if self.state.is_null() {
            return;
        }
        // SAFETY: state points to RuntimeAccount.borrow_state.
        // Restore to NOT_BORROWED when the exclusive borrow is released.
        unsafe {
            *self.state = NOT_BORROWED;
        }
    }
}
