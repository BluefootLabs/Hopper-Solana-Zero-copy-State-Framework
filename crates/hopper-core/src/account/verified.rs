//! Verified account wrappers -- type-safe proof of validation.
//!
//! `VerifiedAccount<T>` and `VerifiedAccountMut<T>` can only be constructed
//! through validated loading paths (tiered loading). Holding one is proof
//! that the account passed the required checks.

use hopper_runtime::{Ref, RefMut};
use hopper_runtime::error::ProgramError;
use super::pod::{Pod, FixedLayout};
use super::header::HEADER_LEN;

enum VerifiedBytes<'a> {
    Borrowed(Ref<'a, [u8]>),
    Raw(&'a [u8]),
}

impl<'a> VerifiedBytes<'a> {
    #[inline(always)]
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Raw(bytes) => bytes,
        }
    }
}

/// Immutable verified account -- proof that validation passed.
pub struct VerifiedAccount<'a, T: Pod + FixedLayout> {
    data: VerifiedBytes<'a>,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> VerifiedAccount<'a, T> {
    /// Construct from pre-validated data.
    ///
    /// Only tiered loading functions should create these.
    #[inline(always)]
    pub fn new(data: &'a [u8]) -> Result<Self, ProgramError> {
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data: VerifiedBytes::Raw(data),
            _phantom: core::marker::PhantomData,
        })
    }

    /// Construct from a Hopper borrow guard.
    #[inline(always)]
    pub fn from_ref(data: Ref<'a, [u8]>) -> Result<Self, ProgramError> {
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data: VerifiedBytes::Borrowed(data),
            _phantom: core::marker::PhantomData,
        })
    }

    /// Get an immutable reference to the overlay. Infallible after construction.
    #[inline(always)]
    pub fn get(&self) -> &T {
        // SAFETY: Size validated at construction. T: Pod, alignment-1 guaranteed.
        unsafe { &*(self.data().as_ptr() as *const T) }
    }

    /// Raw data.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Body data after header.
    #[inline(always)]
    pub fn body(&self) -> &[u8] {
        let data = self.data();
        if data.len() > HEADER_LEN {
            &data[HEADER_LEN..]
        } else {
            &[]
        }
    }

    /// Project a field from the verified overlay.
    ///
    /// The closure receives the typed overlay and returns a reference into it.
    /// The returned reference carries the lifetime of the verified data,
    /// preserving proof-of-validation provenance.
    ///
    /// ```ignore
    /// let vault = Vault::load(account, program_id)?;
    /// let authority: &[u8; 32] = vault.map(|v| &v.authority);
    /// let balance: &WireU64 = vault.map(|v| &v.balance);
    /// ```
    #[inline(always)]
    pub fn map<U, F>(&self, f: F) -> U
    where
        F: FnOnce(&T) -> U,
    {
        f(self.get())
    }

    /// Project a byte sub-slice from the verified data.
    ///
    /// Returns a sub-slice of the already-validated data at the given
    /// offset and length. Useful for accessing raw segments or embedded
    /// sub-structures without re-validation.
    #[inline]
    pub fn slice(&self, offset: usize, len: usize) -> Result<&[u8], ProgramError> {
        let end = offset.checked_add(len).ok_or(ProgramError::ArithmeticOverflow)?;
        if end > self.data().len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(&self.data()[offset..end])
    }

    /// Overlay a second Pod type at a given offset within verified data.
    ///
    /// Useful for accessing embedded sub-layouts in segmented accounts
    /// where the outer account has already been validated.
    #[inline]
    pub fn overlay_at<U: Pod + FixedLayout>(&self, offset: usize) -> Result<&U, ProgramError> {
        let end = offset.checked_add(U::SIZE).ok_or(ProgramError::ArithmeticOverflow)?;
        if end > self.data().len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: Bounds checked. U: Pod + FixedLayout guarantees align 1.
        Ok(unsafe { &*(self.data().as_ptr().add(offset) as *const U) })
    }
}

enum VerifiedBytesMut<'a> {
    Borrowed(RefMut<'a, [u8]>),
    Raw(&'a mut [u8]),
}

impl VerifiedBytesMut<'_> {
    #[inline(always)]
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Raw(bytes) => bytes,
        }
    }

    #[inline(always)]
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Raw(bytes) => bytes,
        }
    }
}

/// Mutable verified account -- proof that validation passed, with write access.
pub struct VerifiedAccountMut<'a, T: Pod + FixedLayout> {
    data: VerifiedBytesMut<'a>,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> VerifiedAccountMut<'a, T> {
    /// Construct from pre-validated mutable data.
    #[inline(always)]
    pub fn new(data: &'a mut [u8]) -> Result<Self, ProgramError> {
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data: VerifiedBytesMut::Raw(data),
            _phantom: core::marker::PhantomData,
        })
    }

    /// Construct from a Hopper mutable borrow guard.
    #[inline(always)]
    pub fn from_ref_mut(data: RefMut<'a, [u8]>) -> Result<Self, ProgramError> {
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data: VerifiedBytesMut::Borrowed(data),
            _phantom: core::marker::PhantomData,
        })
    }

    /// Get an immutable reference to the overlay.
    #[inline(always)]
    pub fn get(&self) -> &T {
        // SAFETY: Size validated at construction.
        unsafe { &*(self.data().as_ptr() as *const T) }
    }

    /// Get a mutable reference to the overlay.
    #[inline(always)]
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: Size validated at construction. We have exclusive access.
        unsafe { &mut *(self.data_mut().as_mut_ptr() as *mut T) }
    }

    /// Raw data (immutable).
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Raw data (mutable).
    #[inline(always)]
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.data.as_mut_slice()
    }

    /// Project a field from the verified overlay (immutable).
    #[inline(always)]
    pub fn map<U, F>(&self, f: F) -> U
    where
        F: FnOnce(&T) -> U,
    {
        f(self.get())
    }

    /// Project a field for mutation.
    ///
    /// ```ignore
    /// let mut vault = Vault::load_mut(account, program_id)?;
    /// vault.map_mut(|v| {
    ///     v.balance = WireU64::new(100);
    /// });
    /// ```
    #[inline(always)]
    pub fn map_mut<U, F>(&mut self, f: F) -> U
    where
        F: FnOnce(&mut T) -> U,
    {
        f(self.get_mut())
    }

    /// Overlay a second Pod type at a given offset (immutable).
    #[inline]
    pub fn overlay_at<U: Pod + FixedLayout>(&self, offset: usize) -> Result<&U, ProgramError> {
        let end = offset.checked_add(U::SIZE).ok_or(ProgramError::ArithmeticOverflow)?;
        if end > self.data().len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(unsafe { &*(self.data().as_ptr().add(offset) as *const U) })
    }

    /// Overlay a second Pod type at a given offset (mutable).
    #[inline]
    pub fn overlay_at_mut<U: Pod + FixedLayout>(&mut self, offset: usize) -> Result<&mut U, ProgramError> {
        let end = offset.checked_add(U::SIZE).ok_or(ProgramError::ArithmeticOverflow)?;
        if end > self.data().len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(unsafe { &mut *(self.data_mut().as_mut_ptr().add(offset) as *mut U) })
    }
}
