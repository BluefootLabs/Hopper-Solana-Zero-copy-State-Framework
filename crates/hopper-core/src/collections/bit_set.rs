//! Compact bit array for flags and bitmask operations.
//!
//! Wire layout: raw bytes, each holding 8 bits. Bit 0 of byte 0 is index 0.
//! No header -- capacity is derived from the byte slice length.

use hopper_runtime::error::ProgramError;

/// Compact bit array overlaid on a byte slice.
///
/// - O(1) get/set/clear/toggle per bit
/// - No overhead -- 1 bit per flag
/// - Used for feature flags, user permission masks, state bitfields
pub struct BitSet<'a> {
    data: &'a mut [u8],
}

impl<'a> BitSet<'a> {
    /// Overlay a BitSet on a mutable byte slice.
    #[inline(always)]
    pub fn from_bytes(data: &'a mut [u8]) -> Self {
        Self { data }
    }

    /// Number of bits available.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.data.len() * 8
    }

    /// Get a bit by index.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Result<bool, ProgramError> {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        if byte_idx >= self.data.len() {
            return Err(ProgramError::InvalidArgument);
        }
        Ok((self.data[byte_idx] >> bit_idx) & 1 == 1)
    }

    /// Set a bit to 1.
    #[inline(always)]
    pub fn set(&mut self, index: usize) -> Result<(), ProgramError> {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        if byte_idx >= self.data.len() {
            return Err(ProgramError::InvalidArgument);
        }
        self.data[byte_idx] |= 1 << bit_idx;
        Ok(())
    }

    /// Clear a bit to 0.
    #[inline(always)]
    pub fn clear(&mut self, index: usize) -> Result<(), ProgramError> {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        if byte_idx >= self.data.len() {
            return Err(ProgramError::InvalidArgument);
        }
        self.data[byte_idx] &= !(1 << bit_idx);
        Ok(())
    }

    /// Toggle a bit.
    #[inline(always)]
    pub fn toggle(&mut self, index: usize) -> Result<(), ProgramError> {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        if byte_idx >= self.data.len() {
            return Err(ProgramError::InvalidArgument);
        }
        self.data[byte_idx] ^= 1 << bit_idx;
        Ok(())
    }

    /// Count the number of set bits (popcount).
    #[inline]
    pub fn count_ones(&self) -> usize {
        let mut count = 0usize;
        for &byte in self.data.iter() {
            count += byte.count_ones() as usize;
        }
        count
    }

    /// Count the number of clear bits.
    #[inline]
    pub fn count_zeros(&self) -> usize {
        self.capacity() - self.count_ones()
    }

    /// Check if ALL bits in a mask are set (starting at byte offset).
    #[inline]
    pub fn check_flags(&self, byte_offset: usize, required: u8) -> Result<(), ProgramError> {
        if byte_offset >= self.data.len() {
            return Err(ProgramError::InvalidArgument);
        }
        if self.data[byte_offset] & required != required {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Check if ANY bit in a mask is set.
    #[inline]
    pub fn check_any_flag(&self, byte_offset: usize, any_of: u8) -> Result<(), ProgramError> {
        if byte_offset >= self.data.len() {
            return Err(ProgramError::InvalidArgument);
        }
        if self.data[byte_offset] & any_of == 0 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Compute the byte size needed for a BitSet with the given number of bits.
    #[inline(always)]
    pub const fn required_bytes(num_bits: usize) -> usize {
        num_bits.div_ceil(8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_clear() {
        let mut buf = [0u8; 4]; // 32 bits
        let mut bs = BitSet::from_bytes(&mut buf);

        assert!(!bs.get(0).unwrap());
        bs.set(0).unwrap();
        assert!(bs.get(0).unwrap());
        bs.clear(0).unwrap();
        assert!(!bs.get(0).unwrap());
    }

    #[test]
    fn toggle() {
        let mut buf = [0u8; 1];
        let mut bs = BitSet::from_bytes(&mut buf);

        bs.toggle(3).unwrap();
        assert!(bs.get(3).unwrap());
        bs.toggle(3).unwrap();
        assert!(!bs.get(3).unwrap());
    }

    #[test]
    fn count_ones() {
        let mut buf = [0b1010_0101u8, 0b1111_0000];
        let bs = BitSet::from_bytes(&mut buf);
        assert_eq!(bs.count_ones(), 4 + 4);
    }

    #[test]
    fn out_of_bounds() {
        let mut buf = [0u8; 1]; // 8 bits
        let mut bs = BitSet::from_bytes(&mut buf);
        assert!(bs.get(8).is_err());
        assert!(bs.set(8).is_err());
    }
}
