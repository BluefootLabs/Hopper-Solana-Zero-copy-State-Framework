//! Zero-copy, tag-validated optional values for instruction args.
//!
//! Rust's `Option<T>` has niche-optimizing layout rules that make it
//! unsafe to pointer-cast from raw instruction bytes. `Option<u8>` is
//! two bytes with an undefined tag range; `Option<&T>` uses null for
//! `None`. Neither is a layout the caller controls.
//!
//! `OptionByte<T>` is the Hopper replacement for args. Layout:
//!
//! ```text
//! #[repr(C)]
//! { tag: u8, value: T }
//! ```
//!
//! `tag == 0` is `None`, `tag == 1` is `Some`. Any other tag byte is
//! a protocol error and [`OptionByte::get`] surfaces it as
//! `ProgramError::InvalidInstructionData`. This mirrors Quasar's
//! `OptionZc<T>::validate_zc` contract with one fewer type parameter
//! and no `MaybeUninit` escape hatch.
//!
//! ## Usage
//!
//! ```ignore
//! #[hopper::args]
//! #[repr(C)]
//! pub struct SwapArgs {
//!     pub amount: u64,
//!     pub referrer: OptionByte<[u8; 32]>,
//!     pub slippage_bps: u16,
//! }
//!
//! fn handler(ctx: Context<Swap>, args: &SwapArgs) -> ProgramResult {
//!     if let Some(referrer) = args.referrer.get()? {
//!         // referrer is &[u8; 32]
//!     }
//!     Ok(())
//! }
//! ```

use crate::{error::ProgramError, result::ProgramResult};

/// Zero-copy tagged optional. See module docs for the layout and
/// usage contract.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct OptionByte<T: Copy> {
    tag: u8,
    value: T,
}

impl<T: Copy> OptionByte<T> {
    /// Construct a `None` variant. Because the struct is `#[repr(C)]`
    /// with a Pod value field, the `value` payload must still be
    /// bitwise valid; the caller provides a default value that is
    /// ignored by [`OptionByte::get`].
    #[inline(always)]
    pub const fn none(default_value: T) -> Self {
        Self { tag: 0, value: default_value }
    }

    /// Construct a `Some(value)` variant.
    #[inline(always)]
    pub const fn some(value: T) -> Self {
        Self { tag: 1, value }
    }

    /// The tag byte as the sender encoded it. Callers should never
    /// inspect this directly; use [`OptionByte::get`] so the tag is
    /// validated first.
    #[inline(always)]
    pub const fn raw_tag(&self) -> u8 {
        self.tag
    }

    /// Validate the tag byte and return the appropriate Rust `Option`.
    ///
    /// Returns `Err(ProgramError::InvalidInstructionData)` when the
    /// tag is neither `0` nor `1`. Any other byte indicates malformed
    /// instruction data and is the exact surface a Quasar `OptionZc`
    /// would flag in `validate_zc`.
    #[inline]
    pub fn get(&self) -> Result<Option<&T>, ProgramError> {
        match self.tag {
            0 => Ok(None),
            1 => Ok(Some(&self.value)),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }

    /// Validate-only: confirms the tag byte is 0 or 1. Useful for
    /// callers who want to reject malformed input early without
    /// taking a reference to the payload.
    #[inline]
    pub fn validate_tag(&self) -> ProgramResult {
        match self.tag {
            0 | 1 => Ok(()),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_reads_as_none() {
        let o: OptionByte<u64> = OptionByte::none(0);
        assert!(o.get().unwrap().is_none());
    }

    #[test]
    fn some_reads_back() {
        let o = OptionByte::some(42u64);
        assert_eq!(*o.get().unwrap().unwrap(), 42);
    }

    #[test]
    fn malformed_tag_rejects() {
        // Simulate a pointer-cast from hostile bytes: a 0xFF tag is
        // neither 0 nor 1.
        let mut buf = [0u8; 9];
        buf[0] = 0xFF;
        let o: &OptionByte<u64> = unsafe { &*(buf.as_ptr() as *const OptionByte<u64>) };
        assert_eq!(o.get().unwrap_err(), ProgramError::InvalidInstructionData);
        assert_eq!(o.validate_tag().unwrap_err(), ProgramError::InvalidInstructionData);
    }

    #[test]
    fn zero_tag_ignores_value_payload() {
        // A None with garbage value bytes still decodes cleanly.
        let mut buf = [0u8; 9];
        buf[1..9].copy_from_slice(&0x1234_5678_9ABC_DEF0u64.to_le_bytes());
        let o: &OptionByte<u64> = unsafe { &*(buf.as_ptr() as *const OptionByte<u64>) };
        assert!(o.get().unwrap().is_none());
    }
}
