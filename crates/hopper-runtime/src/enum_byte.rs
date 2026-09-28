//! Unit enums in zero-copy layouts and instruction arguments.
//!
//! A Rust enum is not `Pod`: a `#[repr(u8)]` enum with three variants has
//! 253 byte values that are not a valid value of the type, so overlaying
//! it on account bytes is undefined behaviour the moment an account holds
//! one of them. The usual workaround is a bare `u8` field and a
//! hand-written `match`, which loses the type in the layout and lets a
//! handler forget the validation.
//!
//! [`EnumByte<E>`] is the field type instead: one byte, alignment 1, every
//! bit pattern valid as far as memory safety goes, and the enum recovered
//! through [`EnumByte::get`], which refuses a byte that names no variant.
//! `#[hopper::unit_enum]` implements [`UnitEnum`] for a fieldless enum, so
//! the mapping between variants and bytes is generated, not written.
//!
//! ```ignore
//! #[hopper::unit_enum]
//! pub enum Status {
//!     Open = 1,
//!     Settled = 2,
//!     Cancelled = 3,
//! }
//!
//! #[hopper::state(disc = 5, version = 1)]
//! #[derive(Clone, Copy)]
//! #[repr(C)]
//! pub struct Order {
//!     pub maker: Address,
//!     pub status: EnumByte<Status>,
//! }
//!
//! if order.status.get()? == Status::Open {
//!     order.status.set(Status::Settled);
//! }
//! ```

use crate::error::ProgramError;
use crate::pod::{Pod, Zeroable};
use crate::result::ProgramResult;
use core::marker::PhantomData;

/// A fieldless enum with a one-byte representation. Implemented by
/// `#[hopper::unit_enum]`; hand-written impls must keep `from_byte` the
/// exact inverse of `to_byte` on every variant.
pub trait UnitEnum: Copy + Sized {
    /// The variant's byte.
    fn to_byte(self) -> u8;

    /// The variant this byte names, if any.
    fn from_byte(byte: u8) -> Option<Self>;
}

/// One byte that stores a [`UnitEnum`]. `Pod`, so it can sit in a
/// `#[hopper::state]` layout, a `#[hopper::pod]` struct, or
/// `#[hopper::args]`; the enum is validated when it is read.
#[repr(transparent)]
pub struct EnumByte<E: UnitEnum> {
    byte: u8,
    _enum: PhantomData<E>,
}

impl<E: UnitEnum> Clone for EnumByte<E> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: UnitEnum> Copy for EnumByte<E> {}

impl<E: UnitEnum> EnumByte<E> {
    /// Store `value`.
    #[inline(always)]
    pub fn new(value: E) -> Self {
        Self {
            byte: value.to_byte(),
            _enum: PhantomData,
        }
    }

    /// Wrap a raw byte without checking that it names a variant;
    /// [`get`](Self::get) checks on every read.
    #[inline(always)]
    pub const fn from_raw(byte: u8) -> Self {
        Self {
            byte,
            _enum: PhantomData,
        }
    }

    /// The stored variant. A byte that names no variant is refused with
    /// `InvalidAccountData`.
    #[inline(always)]
    pub fn get(&self) -> Result<E, ProgramError> {
        E::from_byte(self.byte).ok_or(ProgramError::InvalidAccountData)
    }

    /// Whether the stored byte names a variant.
    #[inline(always)]
    pub fn validate(&self) -> ProgramResult {
        self.get().map(|_| ())
    }

    /// Whether the stored byte is exactly `value`'s. Never fails: an
    /// unknown byte is simply not `value`.
    #[inline(always)]
    pub fn is(&self, value: E) -> bool {
        self.byte == value.to_byte()
    }

    /// Replace the stored variant.
    #[inline(always)]
    pub fn set(&mut self, value: E) {
        self.byte = value.to_byte();
    }

    /// The raw byte.
    #[inline(always)]
    pub const fn raw(&self) -> u8 {
        self.byte
    }
}

impl<E: UnitEnum> From<E> for EnumByte<E> {
    #[inline(always)]
    fn from(value: E) -> Self {
        Self::new(value)
    }
}

impl<E: UnitEnum> PartialEq for EnumByte<E> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.byte == other.byte
    }
}

impl<E: UnitEnum> Eq for EnumByte<E> {}

impl<E: UnitEnum> PartialEq<E> for EnumByte<E> {
    #[inline(always)]
    fn eq(&self, other: &E) -> bool {
        self.is(*other)
    }
}

impl<E: UnitEnum + core::fmt::Debug> core::fmt::Debug for EnumByte<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match E::from_byte(self.byte) {
            Some(value) => value.fmt(f),
            None => write!(f, "EnumByte(invalid {})", self.byte),
        }
    }
}

// SAFETY: `EnumByte` is `repr(transparent)` over one `u8` (the marker is
// zero-sized): alignment 1, no padding, no pointers, and every byte value
// is a valid `EnumByte`; the enum itself is only produced by `get`, which
// validates. `Copy + Sized` holds through the manual impls above.
unsafe impl<E: UnitEnum> Zeroable for EnumByte<E> {}
// SAFETY: as above.
unsafe impl<E: UnitEnum> Pod for EnumByte<E> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(u8)]
    enum Status {
        Open = 1,
        Settled = 2,
        Cancelled = 7,
    }

    impl UnitEnum for Status {
        fn to_byte(self) -> u8 {
            self as u8
        }
        fn from_byte(byte: u8) -> Option<Self> {
            match byte {
                1 => Some(Self::Open),
                2 => Some(Self::Settled),
                7 => Some(Self::Cancelled),
                _ => None,
            }
        }
    }

    #[test]
    fn layout_is_one_byte_with_alignment_one() {
        assert_eq!(core::mem::size_of::<EnumByte<Status>>(), 1);
        assert_eq!(core::mem::align_of::<EnumByte<Status>>(), 1);
    }

    #[test]
    fn reads_validate_and_writes_store_the_variant_byte() {
        let mut field = EnumByte::new(Status::Open);
        assert_eq!(field.raw(), 1);
        assert_eq!(field.get(), Ok(Status::Open));
        assert!(field == Status::Open);
        assert!(field.is(Status::Open) && !field.is(Status::Settled));
        field.set(Status::Cancelled);
        assert_eq!(field.raw(), 7);
        assert_eq!(field.get(), Ok(Status::Cancelled));
        assert_eq!(EnumByte::from(Status::Settled), EnumByte::from_raw(2));
    }

    #[test]
    fn a_byte_that_names_no_variant_is_refused_not_transmuted() {
        for byte in [0u8, 3, 6, 8, 255] {
            let field = EnumByte::<Status>::from_raw(byte);
            assert_eq!(field.get(), Err(ProgramError::InvalidAccountData));
            assert!(field.validate().is_err());
            assert!(!field.is(Status::Open));
        }
        assert_eq!(
            std::format!("{:?}", EnumByte::<Status>::from_raw(9)),
            "EnumByte(invalid 9)"
        );
        assert_eq!(std::format!("{:?}", EnumByte::new(Status::Open)), "Open");
    }

    #[test]
    fn overlays_on_account_bytes() {
        let bytes = [2u8, 7, 9];
        // SAFETY: `EnumByte` is `repr(transparent)` over `u8`; the slice
        // holds three initialized bytes.
        let fields: &[EnumByte<Status>; 3] =
            unsafe { &*(bytes.as_ptr() as *const [EnumByte<Status>; 3]) };
        assert_eq!(fields[0].get(), Ok(Status::Settled));
        assert_eq!(fields[1].get(), Ok(Status::Cancelled));
        assert!(fields[2].get().is_err());
    }
}
