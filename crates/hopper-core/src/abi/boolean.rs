//! Alignment-1 boolean wire type.

use core::fmt;

/// Boolean wire type stored as a single byte.
///
/// `0x00` = `false`, any non-zero = `true`.
/// Normalizes to `0x01` on write.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct WireBool([u8; 1]);

const _: () = assert!(core::mem::size_of::<WireBool>() == 1);
const _: () = assert!(core::mem::align_of::<WireBool>() == 1);

impl WireBool {
    pub const FALSE: Self = Self([0]);
    pub const TRUE: Self = Self([1]);

    #[inline(always)]
    pub const fn new(v: bool) -> Self {
        Self([v as u8])
    }

    #[inline(always)]
    pub const fn get(self) -> bool {
        self.0[0] != 0
    }

    #[inline(always)]
    pub fn set(&mut self, v: bool) {
        self.0[0] = v as u8;
    }
}

impl From<bool> for WireBool {
    #[inline(always)]
    fn from(v: bool) -> Self {
        Self::new(v)
    }
}

impl From<WireBool> for bool {
    #[inline(always)]
    fn from(w: WireBool) -> Self {
        w.get()
    }
}

impl fmt::Debug for WireBool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WireBool({})", self.get())
    }
}

impl fmt::Display for WireBool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

// SAFETY: align_of == 1, all bit patterns valid (any non-zero = true), Copy, no drop.
unsafe impl crate::abi::WireType for WireBool {
    const WIRE_SIZE: usize = 1;
    const CANONICAL_NAME: &'static str = "bool";
}

// Bytemuck proof (Hopper Safety Audit Must-Fix #5). `#[repr(transparent)]`
// over `[u8; 1]` with every bit pattern decoding to a valid `WireBool`
// satisfies the `bytemuck::Pod + Zeroable` obligations.
#[cfg(feature = "hopper-native-backend")]
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for WireBool {}
#[cfg(feature = "hopper-native-backend")]
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for WireBool {}

// SAFETY: #[repr(transparent)] over [u8; 1], all bit patterns valid.
unsafe impl crate::account::Pod for WireBool {}
// Audit Step 5 seal: Hopper-authored primitive.
unsafe impl ::hopper_runtime::__sealed::HopperZeroCopySealed for WireBool {}

impl crate::account::FixedLayout for WireBool {
    const SIZE: usize = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_roundtrip() {
        assert!(WireBool::new(true).get());
        assert!(!WireBool::new(false).get());
    }

    #[test]
    fn nonzero_is_true() {
        let w = WireBool([0xFF]);
        assert!(w.get());
    }
}
