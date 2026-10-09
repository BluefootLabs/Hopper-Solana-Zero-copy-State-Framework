//! Checked, bounded views of length-prefixed instruction elements.

use crate::{Pod, ProgramError};
use core::mem::size_of;

/// A borrowed sequence of at most `N` alignment-1 wire values.
///
/// The wire format is a little-endian `u16` element count followed by exactly
/// that many consecutive `T` values. Parsing validates every element through
/// [`Pod::validate_value`] before returning the view. It allocates no storage
/// and does not copy elements into a capacity-sized array.
///
/// Use wire integers rather than native multi-byte integers. Nested layouts
/// must implement `Pod`; zero-sized elements are refused. Representation
/// checks do not establish account ownership or application authorization.
/// This type is a borrowed view, not a stored account layout or a `TailCodec`.
///
/// Native integers cannot be borrowed at an arbitrary byte offset:
/// ```compile_fail
/// use hopper_runtime::BoundedSlice;
/// let _ = BoundedSlice::<u64, 4>::parse_exact(&[0, 0]);
/// ```
/// The view cannot outlive its input:
/// ```compile_fail
/// use hopper_runtime::BoundedSlice;
/// let values = {
///     let bytes = [1, 0, 42];
///     BoundedSlice::<u8, 4>::parse_exact(&bytes).unwrap()
/// };
/// assert_eq!(values.as_slice(), &[42]);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct BoundedSlice<'a, T: Pod, const N: usize> {
    values: &'a [T],
}

impl<'a, T: Pod, const N: usize> BoundedSlice<'a, T, N> {
    /// Parse one sequence and return its view and the unconsumed suffix.
    ///
    /// Short buffers, excessive counts, zero-sized elements, and invalid
    /// representations return `InvalidInstructionData`. No view escapes on
    /// failure, including when a later element is malformed.
    #[inline]
    pub fn parse_prefix(input: &'a [u8]) -> Result<(Self, &'a [u8]), ProgramError> {
        let prefix = input.get(..2).ok_or(ProgramError::InvalidInstructionData)?;
        let count = u16::from_le_bytes([prefix[0], prefix[1]]) as usize;
        if count > N || size_of::<T>() == 0 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let end = count
            .checked_mul(size_of::<T>())
            .and_then(|bytes| bytes.checked_add(2))
            .ok_or(ProgramError::InvalidInstructionData)?;
        let bytes = input
            .get(2..end)
            .ok_or(ProgramError::InvalidInstructionData)?;
        // SAFETY: Pod guarantees alignment 1, no padding/pointers, and valid
        // Rust values for every bit pattern. `bytes` is a live, non-null slice
        // of exactly count * size_of::<T>() initialized bytes in one allocation.
        // The shared result retains input's lifetime and cannot mutate it.
        let values = unsafe { core::slice::from_raw_parts(bytes.as_ptr().cast::<T>(), count) };
        for value in values {
            value
                .validate_value()
                .map_err(|_| ProgramError::InvalidInstructionData)?;
        }
        Ok((Self { values }, &input[end..]))
    }

    /// Parse a sequence occupying the entire input; refuse trailing bytes.
    #[inline]
    pub fn parse_exact(input: &'a [u8]) -> Result<Self, ProgramError> {
        let (value, rest) = Self::parse_prefix(input)?;
        if !rest.is_empty() {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(value)
    }

    /// The validated elements, still backed by the original input.
    #[inline(always)]
    pub const fn as_slice(&self) -> &'a [T] {
        self.values
    }

    /// Number of elements present, independent of the declared capacity.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the encoded element count is zero.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OptionByte;

    #[test]
    fn unaligned_borrow_preserves_pointer_and_suffix() {
        let input = [99, 2, 0, 1, 42, 0, 255, 77];
        let (values, rest) = BoundedSlice::<OptionByte<u8>, 4>::parse_prefix(&input[1..]).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values.as_slice()[0].get().unwrap(), Some(&42));
        assert_eq!(values.as_slice()[1].get().unwrap(), None);
        assert_eq!(values.as_slice().as_ptr().cast::<u8>(), input[3..].as_ptr());
        assert_eq!(rest.as_ptr(), input[7..].as_ptr());
        assert_eq!(rest, &[77]);
        assert!(BoundedSlice::<OptionByte<u8>, 4>::parse_exact(&input[1..]).is_err());
    }

    #[test]
    fn length_and_representation_checks_are_atomic() {
        let valid = [2, 0, 1, 42, 1, 7];
        for end in 0..valid.len() {
            assert!(BoundedSlice::<OptionByte<u8>, 2>::parse_exact(&valid[..end]).is_err());
        }
        for tag in 0..=255 {
            let mut input = valid;
            input[4] = tag;
            assert_eq!(
                BoundedSlice::<OptionByte<u8>, 2>::parse_exact(&input).is_ok(),
                tag <= 1
            );
        }
        assert!(BoundedSlice::<OptionByte<u8>, 1>::parse_exact(&valid).is_err());
        assert!(BoundedSlice::<u8, 0>::parse_exact(&[0, 0])
            .unwrap()
            .is_empty());
        assert!(BoundedSlice::<u8, 0>::parse_exact(&[1, 0, 0]).is_err());
        assert!(BoundedSlice::<(), 2>::parse_exact(&[0, 0]).is_err());
        assert!(BoundedSlice::<[u8; 0], 2>::parse_exact(&[1, 0]).is_err());
        assert!(BoundedSlice::<u8, 65535>::parse_exact(&[255, 255]).is_err());
    }
}
