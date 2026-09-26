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
    /// Narrow a guard to a field or slice without releasing its account borrow.
    #[inline]
    pub fn map<U: ?Sized>(orig: Self, f: impl FnOnce(&T) -> &U) -> Ref<'a, U> {
        let value = f(orig.value);
        let (_, state) = orig.into_raw_parts();
        Ref { value, state }
    }

    /// Narrow a guard, returning the original guard and error on failure.
    #[inline]
    pub fn try_map<U: ?Sized, E>(
        orig: Self,
        f: impl FnOnce(&T) -> Result<&U, E>,
    ) -> Result<Ref<'a, U>, (Self, E)> {
        match f(orig.value) {
            Ok(value) => {
                let (_, state) = orig.into_raw_parts();
                Ok(Ref { value, state })
            }
            Err(error) => Err((orig, error)),
        }
    }

    /// Narrow a guard, returning the original guard if the field is absent.
    #[inline]
    pub fn filter_map<U: ?Sized>(
        orig: Self,
        f: impl FnOnce(&T) -> Option<&U>,
    ) -> Result<Ref<'a, U>, Self> {
        Self::try_map(orig, |value| f(value).ok_or(())).map_err(|(orig, ())| orig)
    }

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
    /// Narrow an exclusive guard without releasing its account borrow.
    #[inline]
    pub fn map<U: ?Sized>(orig: Self, f: impl FnOnce(&mut T) -> &mut U) -> RefMut<'a, U> {
        match Self::try_map(orig, |value| Ok::<_, core::convert::Infallible>(f(value))) {
            Ok(mapped) => mapped,
            Err((_, never)) => match never {},
        }
    }

    /// Narrow a guard, preserving the original guard on an error. A closure
    /// may itself mutate data before returning an error; those edits are retained.
    #[inline]
    pub fn try_map<U: ?Sized, E>(
        orig: Self,
        f: impl FnOnce(&mut T) -> Result<&mut U, E>,
    ) -> Result<RefMut<'a, U>, (Self, E)> {
        // Move the original exclusive reference before deriving the projection.
        // Moving it afterwards would retag the parent and invalidate the child
        // pointer under Stacked Borrows. Keep unwind release separate from that
        // reference so a panicking closure does not strand the account lease.
        struct ReleaseOnUnwind(*mut u8);
        impl Drop for ReleaseOnUnwind {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    // SAFETY: this guard owns the original exclusive lease;
                    // its account header outlives the mapping call.
                    unsafe { *self.0 = crate::NOT_BORROWED };
                }
            }
        }
        let mut orig = core::mem::ManuallyDrop::new(orig);
        let state = orig.state;
        let unwind = ReleaseOnUnwind(state);
        match f(&mut **orig) {
            Ok(value) => {
                let ptr = value as *mut U;
                core::mem::forget(unwind);
                // SAFETY: the closure's reference is derived from the original
                // guard or is independently valid for that borrow. The original
                // guard is consumed without releasing its exclusive lease; the
                // new guard owns that same lease for the original lifetime.
                Ok(RefMut {
                    // SAFETY: `ptr` retains the exclusive lease described above.
                    value: unsafe { &mut *ptr },
                    state,
                })
            }
            Err(error) => {
                core::mem::forget(unwind);
                Err((core::mem::ManuallyDrop::into_inner(orig), error))
            }
        }
    }

    /// Narrow a guard, returning the original guard if the field is absent.
    #[inline]
    pub fn filter_map<U: ?Sized>(
        orig: Self,
        f: impl FnOnce(&mut T) -> Option<&mut U>,
    ) -> Result<RefMut<'a, U>, Self> {
        Self::try_map(orig, |value| f(value).ok_or(())).map_err(|(orig, ())| orig)
    }

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
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
