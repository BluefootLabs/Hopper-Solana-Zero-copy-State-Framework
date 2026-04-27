//! Alignment-1 little-endian integer wire types.

use core::fmt;

/// Generate a little-endian wire integer type.
///
/// Each type is `#[repr(transparent)]` over `[u8; N]`, guaranteeing alignment 1.
/// Arithmetic operations are not provided -- convert to native, compute, convert back.
/// This keeps the wire layer honest: it's a storage format, not a compute type.
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

        // Bytemuck proof (Hopper Safety Audit Must-Fix #5): the Pod
        // supertrait bound requires these impls. `#[repr(transparent)]`
        // over `[u8; N]` satisfies every bytemuck obligation, all
        // bit patterns valid, no padding, align-1 inherited from the
        // inner array.
        #[cfg(feature = "hopper-native-backend")]
        unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for $name {}
        #[cfg(feature = "hopper-native-backend")]
        unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for $name {}

        // SAFETY: #[repr(transparent)] over [u8; N], all bit patterns valid.
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
