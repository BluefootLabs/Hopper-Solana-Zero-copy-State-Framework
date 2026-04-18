//! Hopper-owned borrow guards for account data.
//!
//! `Ref` and `RefMut` are the safe, drop-guarded handles returned by every
//! Hopper access path: `load()`, `segment_ref()`, `raw_ref()`, and the
//! mutable variants. The representation is backend-sensitive so the hot
//! path stays tight:
//!
//! - **Solana (on-chain)** — `{ ptr, state_ptr }`. Two pointer words, no
//!   extra guards, no slice fat-pointer, no ZSTs. Drop decrements or
//!   restores the single `borrow_state` byte on the `RuntimeAccount`
//!   directly. This matches Pinocchio's pointer shape while adding the
//!   deterministic RAII release that Pinocchio pushes onto the caller.
//!
//! - **non-Solana (host tests, pinocchio-backend, solana-program)** —
//!   `{ ptr, guard, token, _marker }`. Richer because host tests rely on
//!   the active backend's borrow machinery (RefCell, etc.) plus Hopper's
//!   own cross-handle alias registry (`BorrowToken`). Both are real RAII
//!   and must live until the runtime guard drops.
//!
//! Both reprs expose the same surface: `Deref`/`DerefMut` into `T`,
//! `as_ptr` / `as_mut_ptr`, byte-slice narrowing (`slice`, `slice_from`),
//! and byte-level pointer projection (`project`). Whoever reads a
//! generated accessor like `ctx.vault_balance_mut()` cannot tell which
//! repr is in use — and on Solana the compiler collapses every hop to
//! `ptr + offset -> cast`, exactly the shape the finish-line audit
//! demanded.

use core::marker::PhantomData;

use crate::borrow_registry::BorrowToken;
use crate::compat::{BackendRef, BackendRefMut};
use crate::error::ProgramError;

// ══════════════════════════════════════════════════════════════════════
//  Ref (shared borrow)
// ══════════════════════════════════════════════════════════════════════

/// Shared (immutable) borrow guard for account data.
///
/// Derefs to the borrowed data. On drop, the shared borrow is released
/// — on Solana by decrementing the single `RuntimeAccount.borrow_state`
/// byte, on host targets by dropping the backend guard and the
/// cross-handle alias token.
#[cfg(target_os = "solana")]
pub struct Ref<'a, T: ?Sized> {
    ptr: *const T,
    state: *mut u8,
    _marker: PhantomData<&'a T>,
}

#[cfg(not(target_os = "solana"))]
pub struct Ref<'a, T: ?Sized> {
    ptr: *const T,
    guard: BackendRef<'a, [u8]>,
    token: BorrowToken,
    _marker: PhantomData<&'a T>,
}

impl<'a> Ref<'a, [u8]> {
    /// Wrap an active-backend byte borrow into a Hopper Ref.
    ///
    /// On Solana this extracts the shared-borrow state pointer from the
    /// native guard without any further wrapping — the resulting `Ref`
    /// is `{ ptr, state }` only.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRef<'a, [u8]>, token: BorrowToken) -> Self {
        #[cfg(target_os = "solana")]
        {
            let _ = token; // ZST on Solana, dropped immediately.
            let (bytes, state) = inner.into_raw_parts();
            Self {
                ptr: bytes as *const [u8],
                state,
                _marker: PhantomData,
            }
        }
        #[cfg(not(target_os = "solana"))]
        {
            let ptr = (&*inner) as *const [u8];
            Self {
                ptr,
                guard: inner,
                token,
                _marker: PhantomData,
            }
        }
    }

    /// Project a byte borrow into another typed view over the same
    /// underlying bytes. The new guard owns the same release mechanics
    /// — when the returned `Ref<U>` drops, the underlying account
    /// borrow is released exactly as if the original byte borrow had
    /// dropped.
    ///
    /// # Safety
    ///
    /// `ptr` must point inside the byte slice that this `Ref<[u8]>`
    /// guards (offset bounds checked by the caller), the pointee must
    /// be valid `U` for any bit pattern (`U: Pod`-style), and no
    /// alignment beyond the source slice's may be assumed for `U`. The
    /// returned `Ref<U>` inherits the source guard's lifetime, so the
    /// account stays read-borrowed for as long as the typed view lives.
    #[inline(always)]
    pub unsafe fn project<U: ?Sized>(self, ptr: *const U) -> Ref<'a, U> {
        #[cfg(target_os = "solana")]
        {
            let state = self.state;
            core::mem::forget(self);
            Ref {
                ptr,
                state,
                _marker: PhantomData,
            }
        }
        #[cfg(not(target_os = "solana"))]
        {
            let Self { guard, token, .. } = self;
            Ref {
                ptr,
                guard,
                token,
                _marker: PhantomData,
            }
        }
    }

    /// Narrow a shared byte-slice borrow to a tail starting at `offset`.
    #[inline(always)]
    pub fn slice_from(self, offset: usize) -> Ref<'a, [u8]> {
        // SAFETY: `self.ptr` is a valid slice pointer projected from the
        // currently-held shared borrow; the subslice inherits the same
        // borrow lifetime.
        let bytes = unsafe { &*self.ptr };
        let new_ptr = &bytes[offset..] as *const [u8];
        unsafe { self.project(new_ptr) }
    }

    /// Narrow a shared byte-slice borrow to a checked sub-slice.
    #[inline(always)]
    pub fn slice(self, offset: usize, len: usize) -> Result<Ref<'a, [u8]>, ProgramError> {
        // SAFETY: see `slice_from`.
        let bytes = unsafe { &*self.ptr };
        let end = offset
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end > bytes.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let new_ptr = &bytes[offset..end] as *const [u8];
        Ok(unsafe { self.project(new_ptr) })
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

impl<'a, T> Ref<'a, T> {
    /// Construct a lean Ref from a direct segment pointer plus the
    /// shared-borrow state pointer that manages the RAII release.
    ///
    /// This is the Solana-native segment path: skips every intermediate
    /// wrapper and materializes the final `{ptr, state}` shape directly.
    #[cfg(target_os = "solana")]
    #[inline(always)]
    pub(crate) fn from_segment(ptr: *const T, state: *mut u8) -> Self {
        Self {
            ptr,
            state,
            _marker: PhantomData,
        }
    }
}

impl<T: ?Sized> core::ops::Deref for Ref<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        // SAFETY: `self.ptr` was projected from a live shared borrow. On
        // Solana the borrow is kept alive by the `state` field's Drop
        // impl; on host targets by the `guard` + `token` fields. Field
        // drop order guarantees the pointee outlives the `&self` borrow.
        unsafe { &*self.ptr }
    }
}

#[cfg(target_os = "solana")]
impl<T: ?Sized> Drop for Ref<'_, T> {
    #[inline(always)]
    fn drop(&mut self) {
        // Mirror `hopper_native::borrow::Ref::drop`: decrement the
        // shared count, restoring NOT_BORROWED on the last release.
        unsafe {
            let current = *self.state;
            if current == 1 {
                *self.state = hopper_native::NOT_BORROWED;
            } else {
                *self.state = current - 1;
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
//  RefMut (exclusive borrow)
// ══════════════════════════════════════════════════════════════════════

/// Exclusive (mutable) borrow guard for account data.
///
/// See the [module docs](self) for the representation split. On Solana
/// the guard is `{ptr, state}`; on host targets the full backend-guard
/// stack is kept so test harnesses behave identically to real runtime.
#[cfg(target_os = "solana")]
pub struct RefMut<'a, T: ?Sized> {
    ptr: *mut T,
    state: *mut u8,
    _marker: PhantomData<&'a mut T>,
}

#[cfg(not(target_os = "solana"))]
pub struct RefMut<'a, T: ?Sized> {
    ptr: *mut T,
    guard: BackendRefMut<'a, [u8]>,
    token: BorrowToken,
    _marker: PhantomData<&'a mut T>,
}

impl<'a> RefMut<'a, [u8]> {
    /// Wrap an active-backend mutable byte borrow into a Hopper RefMut.
    #[inline(always)]
    pub(crate) fn from_backend(inner: BackendRefMut<'a, [u8]>, token: BorrowToken) -> Self {
        #[cfg(target_os = "solana")]
        {
            let _ = token;
            let (bytes, state) = inner.into_raw_parts();
            Self {
                ptr: bytes as *mut [u8],
                state,
                _marker: PhantomData,
            }
        }
        #[cfg(not(target_os = "solana"))]
        {
            let ptr = (&*inner as *const [u8]).cast_mut();
            Self {
                ptr,
                guard: inner,
                token,
                _marker: PhantomData,
            }
        }
    }

    /// Project a mutable byte borrow into another mutable view over the
    /// same underlying bytes. The new guard owns the same release
    /// mechanics — the exclusive borrow stays held until the returned
    /// `RefMut<U>` drops.
    ///
    /// # Safety
    ///
    /// Same contract as [`Ref::project`]: `ptr` must point inside the
    /// byte slice this guard owns, and the pointee must be valid `U`
    /// for any bit pattern (`U: Pod`-style). The returned `RefMut<U>`
    /// inherits the source guard's lifetime so the account stays
    /// exclusively borrowed for as long as the typed view lives.
    #[inline(always)]
    pub unsafe fn project<U: ?Sized>(self, ptr: *mut U) -> RefMut<'a, U> {
        #[cfg(target_os = "solana")]
        {
            let state = self.state;
            core::mem::forget(self);
            RefMut {
                ptr,
                state,
                _marker: PhantomData,
            }
        }
        #[cfg(not(target_os = "solana"))]
        {
            let Self { guard, token, .. } = self;
            RefMut {
                ptr,
                guard,
                token,
                _marker: PhantomData,
            }
        }
    }

    /// Narrow an exclusive byte-slice borrow to a tail starting at `offset`.
    #[inline(always)]
    pub fn slice_from(self, offset: usize) -> RefMut<'a, [u8]> {
        let bytes = unsafe { &mut *self.ptr };
        let new_ptr = &mut bytes[offset..] as *mut [u8];
        unsafe { self.project(new_ptr) }
    }

    /// Narrow an exclusive byte-slice borrow to a checked sub-slice.
    #[inline(always)]
    pub fn slice(self, offset: usize, len: usize) -> Result<RefMut<'a, [u8]>, ProgramError> {
        let bytes = unsafe { &mut *self.ptr };
        let end = offset
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end > bytes.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let new_ptr = &mut bytes[offset..end] as *mut [u8];
        Ok(unsafe { self.project(new_ptr) })
    }

    #[inline(always)]
    pub fn as_bytes_mut_ptr(&mut self) -> *mut u8 {
        let bytes: &mut [u8] = self;
        bytes.as_mut_ptr()
    }
}

impl<'a, T> RefMut<'a, T> {
    /// Construct a lean RefMut from a direct segment pointer plus the
    /// exclusive-borrow state pointer.
    #[cfg(target_os = "solana")]
    #[inline(always)]
    pub(crate) fn from_segment(ptr: *mut T, state: *mut u8) -> Self {
        Self {
            ptr,
            state,
            _marker: PhantomData,
        }
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
        // SAFETY: see `Ref::deref`.
        unsafe { &*self.ptr }
    }
}

impl<T: ?Sized> core::ops::DerefMut for RefMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: exclusive borrow guaranteed by the guard's lifetime.
        unsafe { &mut *self.ptr }
    }
}

#[cfg(target_os = "solana")]
impl<T: ?Sized> Drop for RefMut<'_, T> {
    #[inline(always)]
    fn drop(&mut self) {
        // Exclusive borrow — restore NOT_BORROWED.
        unsafe {
            *self.state = hopper_native::NOT_BORROWED;
        }
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

// ══════════════════════════════════════════════════════════════════════
//  Size invariants
// ══════════════════════════════════════════════════════════════════════
//
// These `const _: ()` blocks bake the flat-wrapper promise into the
// build. If a future refactor adds another pointer or RAII field the
// build fails here, loudly, rather than silently re-inflating the hot
// path. On Solana a `Ref<u64>` must be exactly two pointer-words
// (ptr + state); a `Ref<[u8]>` takes one extra word for the slice-ptr
// length component.

#[cfg(target_os = "solana")]
const _: () = {
    assert!(
        core::mem::size_of::<Ref<'static, u64>>()
            == core::mem::size_of::<usize>() * 2,
        "Ref<T: Sized> on Solana must be exactly {ptr, state} = 2 words",
    );
    assert!(
        core::mem::size_of::<RefMut<'static, u64>>()
            == core::mem::size_of::<usize>() * 2,
        "RefMut<T: Sized> on Solana must be exactly {ptr, state} = 2 words",
    );
};
