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
        // SAFETY: state points to RuntimeAccount.borrow_state.
        // Restore to NOT_BORROWED when the exclusive borrow is released.
        unsafe {
            *self.state = NOT_BORROWED;
        }
    }
}
