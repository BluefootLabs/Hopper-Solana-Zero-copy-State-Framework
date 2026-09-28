//! Integer helpers that keep the 128-bit multiply helper out of a program.
//!
//! `u64::checked_mul` and `u64::saturating_mul` lower to
//! `umul.with.overflow`, which the sBPF instruction set has no form of, so
//! LLVM links `__multi3` (344 bytes) and calls it for every such product
//! (about 50 CU each). The divide-guard idioms `a > u64::MAX / b` and
//! `(a * b) / b != a` are recognized and folded back into the same helper.
//! Splitting both operands into 32-bit halves decides overflow with 64-bit
//! arithmetic only: the product exceeds 64 bits exactly when both high
//! halves are nonzero, when the cross term reaches 2^32, or when the final
//! add carries. Measured 2026-09-21 on the framework-comparison counter,
//! where the rent product alone linked and called the helper on every
//! `init`.

/// `a * b` when it fits in 64 bits, `None` otherwise, without `__multi3`.
#[inline(always)]
pub const fn checked_mul_u64(a: u64, b: u64) -> Option<u64> {
    let (ah, al) = (a >> 32, a & 0xFFFF_FFFF);
    let (bh, bl) = (b >> 32, b & 0xFFFF_FFFF);
    if ah != 0 && bh != 0 {
        return None;
    }
    // At most one high half is nonzero, so each term is a 32 x 32 product
    // and one addend is zero: exact in 64 bits.
    let cross = ah * bl + al * bh;
    if cross >> 32 != 0 {
        return None;
    }
    (cross << 32).checked_add(al * bl)
}

/// `a * b`, saturating at `u64::MAX`, without `__multi3`.
#[inline(always)]
pub const fn saturating_mul_u64(a: u64, b: u64) -> u64 {
    match checked_mul_u64(a, b) {
        Some(product) => product,
        None => u64::MAX,
    }
}

/// Checked multiplication that routes 64-bit unsigned products through
/// [`checked_mul_u64`] and every other width through the library operator
/// (which needs no helper below 64 bits). Used by the wire integer types,
/// so `WireU64::checked_mul` and `checked_mul_assign` never link the helper.
pub trait LeanMul: Sized + Copy {
    /// `self * rhs`, or `None` on overflow.
    fn checked_mul_lean(self, rhs: Self) -> Option<Self>;
}

macro_rules! lean_mul_library {
    ( $( $t:ty ),* $(,)? ) => {
        $(
            impl LeanMul for $t {
                #[inline(always)]
                fn checked_mul_lean(self, rhs: Self) -> Option<Self> {
                    self.checked_mul(rhs)
                }
            }
        )*
    };
}

lean_mul_library!(u8, u16, u32, u128, i8, i16, i32, i64, i128, isize);

impl LeanMul for u64 {
    #[inline(always)]
    fn checked_mul_lean(self, rhs: Self) -> Option<Self> {
        checked_mul_u64(self, rhs)
    }
}

impl LeanMul for usize {
    #[inline(always)]
    fn checked_mul_lean(self, rhs: Self) -> Option<Self> {
        // `usize` is 64 bits on every target this crate builds for; the
        // cast is lossless and folds away on 32-bit hosts.
        match checked_mul_u64(self as u64, rhs as u64) {
            Some(product) if product <= usize::MAX as u64 => Some(product as usize),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLES: [u64; 18] = [
        0,
        1,
        2,
        3,
        128,
        153,
        3_480,
        5_080,
        6_333,
        10_485_888,
        u32::MAX as u64,
        u32::MAX as u64 + 1,
        1 << 40,
        u64::MAX / 3,
        u64::MAX / 2,
        u64::MAX / 2 + 1,
        u64::MAX - 1,
        u64::MAX,
    ];

    #[test]
    fn checked_mul_u64_matches_the_library_operator() {
        for &a in &SAMPLES {
            for &b in &SAMPLES {
                assert_eq!(checked_mul_u64(a, b), a.checked_mul(b), "{a} * {b}");
                assert_eq!(saturating_mul_u64(a, b), a.saturating_mul(b), "{a} * {b}");
            }
        }
    }

    #[test]
    fn lean_mul_covers_every_width_like_the_library() {
        assert_eq!(7u8.checked_mul_lean(3), Some(21));
        assert_eq!(7u8.checked_mul_lean(40), None);
        assert_eq!(300u16.checked_mul_lean(300), None);
        assert_eq!(70_000u32.checked_mul_lean(70_000), None);
        assert_eq!((1u64 << 32).checked_mul_lean(1 << 32), None);
        assert_eq!((1u64 << 31).checked_mul_lean(1 << 32), Some(1 << 63));
        assert_eq!((1usize << 20).checked_mul_lean(1 << 20), Some(1 << 40));
        assert_eq!((-3i64).checked_mul_lean(4), Some(-12));
        assert_eq!(i64::MIN.checked_mul_lean(-1), None);
        assert_eq!((1u128 << 100).checked_mul_lean(1 << 20), Some(1 << 120));
    }
}
