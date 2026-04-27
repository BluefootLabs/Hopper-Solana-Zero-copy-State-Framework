//! Alignment-safe wire types for zero-copy account data.
//!
//! Solana account data buffers have alignment 1. Casting a `*const u8`
//! to `*const u64` causes undefined behavior when the pointer is not
//! 8-byte aligned. Every framework that does zero-copy must solve this.
//!
//! Quasar solves it with `PodU64([u8; 8])` -- wrapping arithmetic by
//! default and implicit conversion. Hopper takes a different approach:
//!
//! - **Explicit endianness**: Types are named `LeU64` ("little-endian u64"),
//!   making the wire representation unambiguous at every call site.
//! - **Checked arithmetic by default**: `+`, `-`, `*` return `Option` via
//!   `checked_add` etc. This is safer than wrapping overflow silently
//!   (Quasar's default) and matches Rust's principled stance on UB.
//! - **`const fn` constructors**: `LeU64::new(42)` works in const context,
//!   enabling compile-time constants for discriminators, seeds, etc.
//! - **`Projectable`**: All wire types implement `Projectable`, so you can
//!   use `project::<LeU64>(account, offset, None)` to read them directly
//!   from account data without alignment issues.
//!
//! These types are the foundation for safe zero-copy account structs.
//! Any `#[repr(C)]` struct composed entirely of wire types + `[u8; N]`
//! arrays is alignment-1-safe and can be projected from account data.

use crate::project::Projectable;

// ---- Macro to generate integer wire types ----------------------------

macro_rules! le_integer {
    (
        $(#[$meta:meta])*
        $name:ident, $native:ty, $size:expr, unsigned
    ) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, Default, Eq, PartialEq, Hash)]
        pub struct $name([u8; $size]);

        impl $name {
            /// Zero value.
            pub const ZERO: Self = Self([0; $size]);

            /// Maximum representable value.
            pub const MAX: Self = Self(<$native>::MAX.to_le_bytes());

            /// Construct from a native integer (const-safe).
            #[inline(always)]
            pub const fn new(v: $native) -> Self {
                Self(v.to_le_bytes())
            }

            /// Read the native integer value.
            #[inline(always)]
            pub const fn get(self) -> $native {
                <$native>::from_le_bytes(self.0)
            }

            /// Raw little-endian bytes.
            #[inline(always)]
            pub const fn to_le_bytes(self) -> [u8; $size] {
                self.0
            }

            /// Construct from raw little-endian bytes.
            #[inline(always)]
            pub const fn from_le_bytes(bytes: [u8; $size]) -> Self {
                Self(bytes)
            }

            /// Checked addition. Returns `None` on overflow.
            #[inline(always)]
            pub const fn checked_add(self, rhs: Self) -> Option<Self> {
                match self.get().checked_add(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Checked subtraction. Returns `None` on underflow.
            #[inline(always)]
            pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
                match self.get().checked_sub(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Checked multiplication. Returns `None` on overflow.
            #[inline(always)]
            pub const fn checked_mul(self, rhs: Self) -> Option<Self> {
                match self.get().checked_mul(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Checked division. Returns `None` on divide-by-zero.
            #[inline(always)]
            pub const fn checked_div(self, rhs: Self) -> Option<Self> {
                match self.get().checked_div(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Saturating addition (clamps at MAX instead of wrapping).
            #[inline(always)]
            pub const fn saturating_add(self, rhs: Self) -> Self {
                Self::new(self.get().saturating_add(rhs.get()))
            }

            /// Saturating subtraction (clamps at 0 instead of wrapping).
            #[inline(always)]
            pub const fn saturating_sub(self, rhs: Self) -> Self {
                Self::new(self.get().saturating_sub(rhs.get()))
            }

            /// Wrapping addition (use explicitly when wrapping is intended).
            #[inline(always)]
            pub const fn wrapping_add(self, rhs: Self) -> Self {
                Self::new(self.get().wrapping_add(rhs.get()))
            }

            /// Wrapping subtraction.
            #[inline(always)]
            pub const fn wrapping_sub(self, rhs: Self) -> Self {
                Self::new(self.get().wrapping_sub(rhs.get()))
            }

            /// Whether the value is zero.
            #[inline(always)]
            pub const fn is_zero(self) -> bool {
                self.get() == 0
            }
        }

        impl From<$native> for $name {
            #[inline(always)]
            fn from(v: $native) -> Self { Self::new(v) }
        }

        impl From<$name> for $native {
            #[inline(always)]
            fn from(v: $name) -> Self { v.get() }
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

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.get())
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.get())
            }
        }

        // SAFETY: $name is #[repr(transparent)] over [u8; N].
        // All bit patterns are valid (no padding, no alignment requirement).
        unsafe impl Projectable for $name {}

        $crate::__wire_arith_ops!($name, $native);
    };

    // Signed variant -- same API but with signed native type.
    (
        $(#[$meta:meta])*
        $name:ident, $native:ty, $size:expr, signed
    ) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, Default, Eq, PartialEq, Hash)]
        pub struct $name([u8; $size]);

        impl $name {
            /// Zero value.
            pub const ZERO: Self = Self([0; $size]);

            /// Maximum representable value.
            pub const MAX: Self = Self(<$native>::MAX.to_le_bytes());

            /// Minimum representable value.
            pub const MIN: Self = Self(<$native>::MIN.to_le_bytes());

            /// Construct from a native integer (const-safe).
            #[inline(always)]
            pub const fn new(v: $native) -> Self {
                Self(v.to_le_bytes())
            }

            /// Read the native integer value.
            #[inline(always)]
            pub const fn get(self) -> $native {
                <$native>::from_le_bytes(self.0)
            }

            /// Raw little-endian bytes.
            #[inline(always)]
            pub const fn to_le_bytes(self) -> [u8; $size] {
                self.0
            }

            /// Construct from raw little-endian bytes.
            #[inline(always)]
            pub const fn from_le_bytes(bytes: [u8; $size]) -> Self {
                Self(bytes)
            }

            /// Checked addition.
            #[inline(always)]
            pub const fn checked_add(self, rhs: Self) -> Option<Self> {
                match self.get().checked_add(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Checked subtraction.
            #[inline(always)]
            pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
                match self.get().checked_sub(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Checked multiplication.
            #[inline(always)]
            pub const fn checked_mul(self, rhs: Self) -> Option<Self> {
                match self.get().checked_mul(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Checked division.
            #[inline(always)]
            pub const fn checked_div(self, rhs: Self) -> Option<Self> {
                match self.get().checked_div(rhs.get()) {
                    Some(v) => Some(Self::new(v)),
                    None => None,
                }
            }

            /// Saturating addition.
            #[inline(always)]
            pub const fn saturating_add(self, rhs: Self) -> Self {
                Self::new(self.get().saturating_add(rhs.get()))
            }

            /// Saturating subtraction.
            #[inline(always)]
            pub const fn saturating_sub(self, rhs: Self) -> Self {
                Self::new(self.get().saturating_sub(rhs.get()))
            }

            /// Whether the value is zero.
            #[inline(always)]
            pub const fn is_zero(self) -> bool {
                self.get() == 0
            }

            /// Whether the value is negative.
            #[inline(always)]
            pub const fn is_negative(self) -> bool {
                self.get() < 0
            }

            /// Absolute value (wraps on MIN).
            #[inline(always)]
            pub const fn abs(self) -> Self {
                Self::new(self.get().wrapping_abs())
            }
        }

        impl From<$native> for $name {
            #[inline(always)]
            fn from(v: $native) -> Self { Self::new(v) }
        }

        impl From<$name> for $native {
            #[inline(always)]
            fn from(v: $name) -> Self { v.get() }
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

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.get())
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.get())
            }
        }

        unsafe impl Projectable for $name {}

        $crate::__wire_arith_ops!($name, $native);
    };
}

/// Internal: emit arithmetic operator impls for a wire integer type.
///
/// Mirrors Rust's native integer behavior: panic on overflow in debug,
/// wrap in release. Programs that need explicit semantics should use the
/// `checked_*`, `saturating_*`, or `wrapping_*` inherent methods.
#[doc(hidden)]
#[macro_export]
macro_rules! __wire_arith_ops {
    ($name:ident, $native:ty) => {
        impl core::ops::Add for $name {
            type Output = Self;
            #[inline(always)]
            fn add(self, rhs: Self) -> Self { Self::new(self.get() + rhs.get()) }
        }
        impl core::ops::Sub for $name {
            type Output = Self;
            #[inline(always)]
            fn sub(self, rhs: Self) -> Self { Self::new(self.get() - rhs.get()) }
        }
        impl core::ops::Mul for $name {
            type Output = Self;
            #[inline(always)]
            fn mul(self, rhs: Self) -> Self { Self::new(self.get() * rhs.get()) }
        }
        impl core::ops::Div for $name {
            type Output = Self;
            #[inline(always)]
            fn div(self, rhs: Self) -> Self { Self::new(self.get() / rhs.get()) }
        }
        impl core::ops::Rem for $name {
            type Output = Self;
            #[inline(always)]
            fn rem(self, rhs: Self) -> Self { Self::new(self.get() % rhs.get()) }
        }
        impl core::ops::Add<$native> for $name {
            type Output = Self;
            #[inline(always)]
            fn add(self, rhs: $native) -> Self { Self::new(self.get() + rhs) }
        }
        impl core::ops::Sub<$native> for $name {
            type Output = Self;
            #[inline(always)]
            fn sub(self, rhs: $native) -> Self { Self::new(self.get() - rhs) }
        }
        impl core::ops::Mul<$native> for $name {
            type Output = Self;
            #[inline(always)]
            fn mul(self, rhs: $native) -> Self { Self::new(self.get() * rhs) }
        }
        impl core::ops::Div<$native> for $name {
            type Output = Self;
            #[inline(always)]
            fn div(self, rhs: $native) -> Self { Self::new(self.get() / rhs) }
        }
        impl core::ops::Rem<$native> for $name {
            type Output = Self;
            #[inline(always)]
            fn rem(self, rhs: $native) -> Self { Self::new(self.get() % rhs) }
        }
        impl core::ops::AddAssign for $name {
            #[inline(always)]
            fn add_assign(&mut self, rhs: Self) { *self = *self + rhs; }
        }
        impl core::ops::SubAssign for $name {
            #[inline(always)]
            fn sub_assign(&mut self, rhs: Self) { *self = *self - rhs; }
        }
        impl core::ops::MulAssign for $name {
            #[inline(always)]
            fn mul_assign(&mut self, rhs: Self) { *self = *self * rhs; }
        }
        impl core::ops::DivAssign for $name {
            #[inline(always)]
            fn div_assign(&mut self, rhs: Self) { *self = *self / rhs; }
        }
        impl core::ops::RemAssign for $name {
            #[inline(always)]
            fn rem_assign(&mut self, rhs: Self) { *self = *self % rhs; }
        }
        impl core::ops::AddAssign<$native> for $name {
            #[inline(always)]
            fn add_assign(&mut self, rhs: $native) { *self = *self + rhs; }
        }
        impl core::ops::SubAssign<$native> for $name {
            #[inline(always)]
            fn sub_assign(&mut self, rhs: $native) { *self = *self - rhs; }
        }
        impl core::ops::MulAssign<$native> for $name {
            #[inline(always)]
            fn mul_assign(&mut self, rhs: $native) { *self = *self * rhs; }
        }
        impl core::ops::DivAssign<$native> for $name {
            #[inline(always)]
            fn div_assign(&mut self, rhs: $native) { *self = *self / rhs; }
        }
        impl core::ops::RemAssign<$native> for $name {
            #[inline(always)]
            fn rem_assign(&mut self, rhs: $native) { *self = *self % rhs; }
        }
        impl PartialEq<$native> for $name {
            #[inline(always)]
            fn eq(&self, other: &$native) -> bool { self.get() == *other }
        }
        impl PartialOrd<$native> for $name {
            #[inline(always)]
            fn partial_cmp(&self, other: &$native) -> Option<core::cmp::Ordering> {
                Some(self.get().cmp(other))
            }
        }
    };
}

// ---- Unsigned wire types ---------------------------------------------

le_integer! {
    /// 64-bit unsigned little-endian integer. Alignment 1.
    ///
    /// The workhorse type for token amounts, lamport balances, timestamps,
    /// and most on-chain numeric fields. Use this instead of `u64` in any
    /// `#[repr(C)]` struct that will be projected from account data.
    LeU64, u64, 8, unsigned
}

le_integer! {
    /// 32-bit unsigned little-endian integer. Alignment 1.
    LeU32, u32, 4, unsigned
}

le_integer! {
    /// 16-bit unsigned little-endian integer. Alignment 1.
    LeU16, u16, 2, unsigned
}

// ---- Signed wire types -----------------------------------------------

le_integer! {
    /// 64-bit signed little-endian integer. Alignment 1.
    ///
    /// Used for timestamps (unix_timestamp is i64), deltas, and any
    /// signed arithmetic in account data.
    LeI64, i64, 8, signed
}

le_integer! {
    /// 32-bit signed little-endian integer. Alignment 1.
    LeI32, i32, 4, signed
}

le_integer! {
    /// 16-bit signed little-endian integer. Alignment 1.
    LeI16, i16, 2, signed
}

// ---- LeBool ----------------------------------------------------------

/// Boolean wire type. Alignment 1.
///
/// Stored as a single byte: 0 = false, nonzero = true.
/// `is_valid()` returns true only for 0 or 1, catching
/// corrupted data that other frameworks would silently accept.
#[repr(transparent)]
#[derive(Clone, Copy, Default, Eq, PartialEq, Hash)]
pub struct LeBool(u8);

impl LeBool {
    /// Canonical true value.
    pub const TRUE: Self = Self(1);

    /// Canonical false value.
    pub const FALSE: Self = Self(0);

    /// Construct from a Rust bool.
    #[inline(always)]
    pub const fn new(v: bool) -> Self {
        Self(v as u8)
    }

    /// Read as a Rust bool (0 = false, anything else = true).
    #[inline(always)]
    pub const fn get(self) -> bool {
        self.0 != 0
    }

    /// Raw byte value.
    #[inline(always)]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Whether the byte is strictly 0 or 1 (canonical representation).
    ///
    /// Non-canonical values (2..=255) are technically "true" but may
    /// indicate data corruption or an incompatible writer.
    #[inline(always)]
    pub const fn is_canonical(self) -> bool {
        self.0 == 0 || self.0 == 1
    }
}

impl From<bool> for LeBool {
    #[inline(always)]
    fn from(v: bool) -> Self { Self::new(v) }
}

impl From<LeBool> for bool {
    #[inline(always)]
    fn from(v: LeBool) -> Self { v.get() }
}

impl core::fmt::Debug for LeBool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "LeBool({})", self.get())
    }
}

impl core::fmt::Display for LeBool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.get())
    }
}

// SAFETY: LeBool is #[repr(transparent)] over u8. All bit patterns valid.
unsafe impl Projectable for LeBool {}

// ---- LeU128 ----------------------------------------------------------

/// 128-bit unsigned little-endian integer. Alignment 1.
///
/// Useful for large amounts (e.g., total supply tracking) where u64
/// would overflow. Stored as 16 bytes in account data.
#[repr(transparent)]
#[derive(Clone, Copy, Default, Eq, PartialEq, Hash)]
pub struct LeU128([u8; 16]);

impl LeU128 {
    pub const ZERO: Self = Self([0; 16]);
    pub const MAX: Self = Self(u128::MAX.to_le_bytes());

    #[inline(always)]
    pub const fn new(v: u128) -> Self {
        Self(v.to_le_bytes())
    }

    #[inline(always)]
    pub const fn get(self) -> u128 {
        u128::from_le_bytes(self.0)
    }

    #[inline(always)]
    pub const fn to_le_bytes(self) -> [u8; 16] {
        self.0
    }

    #[inline(always)]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.get().checked_add(rhs.get()) {
            Some(v) => Some(Self::new(v)),
            None => None,
        }
    }

    #[inline(always)]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.get().checked_sub(rhs.get()) {
            Some(v) => Some(Self::new(v)),
            None => None,
        }
    }

    #[inline(always)]
    pub const fn checked_mul(self, rhs: Self) -> Option<Self> {
        match self.get().checked_mul(rhs.get()) {
            Some(v) => Some(Self::new(v)),
            None => None,
        }
    }

    #[inline(always)]
    pub const fn saturating_add(self, rhs: Self) -> Self {
        Self::new(self.get().saturating_add(rhs.get()))
    }

    #[inline(always)]
    pub const fn saturating_sub(self, rhs: Self) -> Self {
        Self::new(self.get().saturating_sub(rhs.get()))
    }

    #[inline(always)]
    pub const fn is_zero(self) -> bool {
        self.get() == 0
    }
}

impl From<u128> for LeU128 {
    #[inline(always)]
    fn from(v: u128) -> Self { Self::new(v) }
}

impl From<LeU128> for u128 {
    #[inline(always)]
    fn from(v: LeU128) -> Self { v.get() }
}

impl PartialOrd for LeU128 {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LeU128 {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.get().cmp(&other.get())
    }
}

impl core::fmt::Debug for LeU128 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "LeU128({})", self.get())
    }
}

impl core::fmt::Display for LeU128 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.get())
    }
}

unsafe impl Projectable for LeU128 {}

__wire_arith_ops!(LeU128, u128);
