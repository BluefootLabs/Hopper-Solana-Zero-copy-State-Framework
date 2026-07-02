//! Alignment-1 little-endian integer wire types.

use core::{fmt, ops};

/// Generate a little-endian wire integer type.
///
/// Each type is `#[repr(transparent)]` over `[u8; N]`, guaranteeing alignment 1.
/// Checked assignment helpers are provided for common handler code; anything
/// more complex should still convert to native, compute, then write back.
macro_rules! wire_int {
    (
        $(#[$meta:meta])*
        $name:ident, $native:ty, $size:literal, $canonical:literal
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Default)]
        #[repr(transparent)]
        pub struct $name([u8; $size]);

        // Compile-time guarantees
        const _: () = assert!(core::mem::size_of::<$name>() == $size);
        const _: () = assert!(core::mem::align_of::<$name>() == 1);

        impl $name {
            /// Zero value.
            pub const ZERO: Self = Self([0u8; $size]);

            /// Maximum value.
            pub const MAX: Self = Self(<$native>::MAX.to_le_bytes());

            /// Minimum value.
            pub const MIN: Self = Self(<$native>::MIN.to_le_bytes());

            /// Wrap a native value into wire format.
            #[inline(always)]
            pub const fn new(v: $native) -> Self {
                Self(v.to_le_bytes())
            }

            /// Read the native value from wire format.
            #[inline(always)]
            pub const fn get(self) -> $native {
                <$native>::from_le_bytes(self.0)
            }

            /// Write a native value into this wire slot.
            #[inline(always)]
            pub fn set(&mut self, v: $native) {
                self.0 = v.to_le_bytes();
            }

            /// Checked addition in native form, written back on success.
            #[inline(always)]
            pub fn checked_add_assign(
                &mut self,
                rhs: $native,
            ) -> ::core::result::Result<(), ::hopper_runtime::ProgramError> {
                let next = self
                    .get()
                    .checked_add(rhs)
                    .ok_or(::hopper_runtime::ProgramError::ArithmeticOverflow)?;
                self.set(next);
                Ok(())
            }

            /// Alias for [`Self::checked_add_assign`].
            #[inline(always)]
            pub fn add_assign_checked(
                &mut self,
                rhs: $native,
            ) -> ::core::result::Result<(), ::hopper_runtime::ProgramError> {
                self.checked_add_assign(rhs)
            }

            /// Checked subtraction in native form, written back on success.
            #[inline(always)]
            pub fn checked_sub_assign(
                &mut self,
                rhs: $native,
            ) -> ::core::result::Result<(), ::hopper_runtime::ProgramError> {
                let next = self
                    .get()
                    .checked_sub(rhs)
                    .ok_or(::hopper_runtime::ProgramError::ArithmeticOverflow)?;
                self.set(next);
                Ok(())
            }

            /// Alias for [`Self::checked_sub_assign`].
            #[inline(always)]
            pub fn sub_assign_checked(
                &mut self,
                rhs: $native,
            ) -> ::core::result::Result<(), ::hopper_runtime::ProgramError> {
                self.checked_sub_assign(rhs)
            }

            /// Checked multiplication in native form, written back on success.
            #[inline(always)]
            pub fn checked_mul_assign(
                &mut self,
                rhs: $native,
            ) -> ::core::result::Result<(), ::hopper_runtime::ProgramError> {
                let next = self
                    .get()
                    .checked_mul(rhs)
                    .ok_or(::hopper_runtime::ProgramError::ArithmeticOverflow)?;
                self.set(next);
                Ok(())
            }

            /// Alias for [`Self::checked_mul_assign`].
            #[inline(always)]
            pub fn mul_assign_checked(
                &mut self,
                rhs: $native,
            ) -> ::core::result::Result<(), ::hopper_runtime::ProgramError> {
                self.checked_mul_assign(rhs)
            }

            /// Raw byte access (immutable).
            #[inline(always)]
            pub const fn as_bytes(&self) -> &[u8; $size] {
                &self.0
            }

            /// Raw byte access (mutable).
            #[inline(always)]
            pub fn as_bytes_mut(&mut self) -> &mut [u8; $size] {
                &mut self.0
            }
        }

        impl From<$native> for $name {
            #[inline(always)]
            fn from(v: $native) -> Self {
                Self::new(v)
            }
        }

        impl From<$name> for $native {
            #[inline(always)]
            fn from(w: $name) -> Self {
                w.get()
            }
        }

        // Operator sugar is overflow-CHECKED: on overflow it panics,
        // which on SBF aborts the transaction — a loud fail-safe. The
        // pre-audit impls used native `+`/`-`/`*`, which wrap silently
        // in release builds: `balance += amount` compiling to wrapping
        // arithmetic on mainnet is the classic exploit shape. Handlers
        // that want a clean recoverable error should use the
        // `checked_*_assign` helpers above instead of the operators.
        impl ops::Add<$native> for $name {
            type Output = Self;

            #[inline(always)]
            fn add(self, rhs: $native) -> Self::Output {
                Self::new(self.get().checked_add(rhs).expect("wire integer addition overflowed"))
            }
        }

        impl ops::Add<Self> for $name {
            type Output = Self;

            #[inline(always)]
            fn add(self, rhs: Self) -> Self::Output {
                Self::new(self.get().checked_add(rhs.get()).expect("wire integer addition overflowed"))
            }
        }

        impl ops::AddAssign<$native> for $name {
            #[inline(always)]
            fn add_assign(&mut self, rhs: $native) {
                self.set(self.get().checked_add(rhs).expect("wire integer addition overflowed"));
            }
        }

        impl ops::AddAssign<Self> for $name {
            #[inline(always)]
            fn add_assign(&mut self, rhs: Self) {
                self.set(self.get().checked_add(rhs.get()).expect("wire integer addition overflowed"));
            }
        }

        impl ops::Sub<$native> for $name {
            type Output = Self;

            #[inline(always)]
            fn sub(self, rhs: $native) -> Self::Output {
                Self::new(self.get().checked_sub(rhs).expect("wire integer subtraction overflowed"))
            }
        }

        impl ops::Sub<Self> for $name {
            type Output = Self;

            #[inline(always)]
            fn sub(self, rhs: Self) -> Self::Output {
                Self::new(self.get().checked_sub(rhs.get()).expect("wire integer subtraction overflowed"))
            }
        }

        impl ops::SubAssign<$native> for $name {
            #[inline(always)]
            fn sub_assign(&mut self, rhs: $native) {
                self.set(self.get().checked_sub(rhs).expect("wire integer subtraction overflowed"));
            }
        }

        impl ops::SubAssign<Self> for $name {
            #[inline(always)]
            fn sub_assign(&mut self, rhs: Self) {
                self.set(self.get().checked_sub(rhs.get()).expect("wire integer subtraction overflowed"));
            }
        }

        impl ops::Mul<$native> for $name {
            type Output = Self;

            #[inline(always)]
            fn mul(self, rhs: $native) -> Self::Output {
                Self::new(self.get().checked_mul(rhs).expect("wire integer multiplication overflowed"))
            }
        }

        impl ops::Mul<Self> for $name {
            type Output = Self;

            #[inline(always)]
            fn mul(self, rhs: Self) -> Self::Output {
                Self::new(self.get().checked_mul(rhs.get()).expect("wire integer multiplication overflowed"))
            }
        }

        impl ops::MulAssign<$native> for $name {
            #[inline(always)]
            fn mul_assign(&mut self, rhs: $native) {
                self.set(self.get().checked_mul(rhs).expect("wire integer multiplication overflowed"));
            }
        }

        impl ops::MulAssign<Self> for $name {
            #[inline(always)]
            fn mul_assign(&mut self, rhs: Self) {
                self.set(self.get().checked_mul(rhs.get()).expect("wire integer multiplication overflowed"));
            }
        }

        impl PartialOrd for $name {
            #[inline(always)]
            fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $name {
            #[inline(always)]
            fn cmp(&self, other: &Self) -> core::cmp::Ordering {
                self.get().cmp(&other.get())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.get())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.get())
            }
        }

        // SAFETY: align_of == 1, all bit patterns valid, Copy, no drop glue.
        unsafe impl crate::abi::WireType for $name {
            const WIRE_SIZE: usize = $size;
            const CANONICAL_NAME: &'static str = $canonical;
        }

        // SAFETY: #[repr(transparent)] over [u8; N], all bit patterns valid.
        unsafe impl crate::account::Zeroable for $name {}
        unsafe impl crate::account::Pod for $name {}

        // Audit Step 5 seal: stamp the Hopper-authored marker so the
        // blanket `ZeroCopy` impl picks this primitive up. A user
        // bypassing the wire_int! path with their own bare
        // `unsafe impl Pod` does not get the seal.
        unsafe impl ::hopper_runtime::__sealed::HopperZeroCopySealed for $name {}

        impl crate::account::FixedLayout for $name {
            const SIZE: usize = $size;
        }
    };
}

wire_int!(
    /// 16-bit unsigned little-endian wire integer.
    WireU16, u16, 2, "u16"
);

wire_int!(
    /// 32-bit unsigned little-endian wire integer.
    WireU32, u32, 4, "u32"
);

wire_int!(
    /// 64-bit unsigned little-endian wire integer.
    WireU64, u64, 8, "u64"
);

wire_int!(
    /// 128-bit unsigned little-endian wire integer.
    WireU128, u128, 16, "u128"
);

wire_int!(
    /// 16-bit signed little-endian wire integer.
    WireI16, i16, 2, "i16"
);

wire_int!(
    /// 32-bit signed little-endian wire integer.
    WireI32, i32, 4, "i32"
);

wire_int!(
    /// 64-bit signed little-endian wire integer.
    WireI64, i64, 8, "i64"
);

wire_int!(
    /// 128-bit signed little-endian wire integer.
    WireI128, i128, 16, "i128"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_u64_roundtrip() {
        let w = WireU64::new(0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(w.get(), 0xDEAD_BEEF_CAFE_BABE);
    }

    #[test]
    fn wire_i64_negative() {
        let w = WireI64::new(-42);
        assert_eq!(w.get(), -42);
    }

    #[test]
    fn wire_u64_checked_assign_helpers() {
        let mut w = WireU64::new(10);
        w.checked_add_assign(5).unwrap();
        assert_eq!(w.get(), 15);
        w.checked_sub_assign(3).unwrap();
        assert_eq!(w.get(), 12);
        w.checked_mul_assign(2).unwrap();
        assert_eq!(w.get(), 24);
    }

    #[test]
    fn wire_u64_assignment_operators_accept_native_rhs() {
        let mut w = WireU64::new(10);
        w += 5;
        assert_eq!(w.get(), 15);
        w -= 3;
        assert_eq!(w.get(), 12);
        w *= 2;
        assert_eq!(w.get(), 24);

        assert_eq!((w + 1).get(), 25);
        assert_eq!((w - 4).get(), 20);
        assert_eq!((w * 3).get(), 72);
    }

    #[test]
    fn wire_assignment_operators_accept_wire_rhs() {
        let mut w = WireI64::new(10);
        w += WireI64::new(-3);
        assert_eq!(w.get(), 7);
        w -= WireI64::new(2);
        assert_eq!(w.get(), 5);
        w *= WireI64::new(-2);
        assert_eq!(w.get(), -10);
    }

    #[test]
    fn wire_u64_checked_assign_overflow_is_reported() {
        let mut w = WireU64::new(u64::MAX);
        assert_eq!(
            w.checked_add_assign(1),
            Err(::hopper_runtime::ProgramError::ArithmeticOverflow)
        );
        assert_eq!(w.get(), u64::MAX);
    }

    #[test]
    #[should_panic(expected = "wire integer addition overflowed")]
    fn operator_overflow_panics_instead_of_wrapping() {
        // Pre-fix the operators used native `+`, which WRAPS in release
        // builds — u64::MAX + 1 silently became 0. The operators must
        // abort loudly; the recoverable path is checked_add_assign.
        let mut w = WireU64::new(u64::MAX);
        w += 1;
    }

    #[test]
    #[should_panic(expected = "wire integer subtraction overflowed")]
    fn operator_underflow_panics_instead_of_wrapping() {
        let mut w = WireU64::new(0);
        w -= 1;
    }

    #[test]
    fn wire_ordering() {
        let a = WireU32::new(10);
        let b = WireU32::new(20);
        assert!(a < b);
    }

    #[test]
    fn wire_default_is_zero() {
        let w = WireU64::default();
        assert_eq!(w.get(), 0);
    }
}
