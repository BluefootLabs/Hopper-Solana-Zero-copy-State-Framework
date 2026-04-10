//! Typed, borrow-split field references for non-overlapping access to account data.
//!
//! `FieldRef` and `FieldMut` hold independent subslices of account data,
//! allowing simultaneous immutable (or individually mutable) access to
//! different fields without violating Rust's aliasing rules.

use hopper_runtime::error::ProgramError;

/// Immutable typed view over a field's bytes.
#[derive(Clone, Copy)]
pub struct FieldRef<'a> {
    data: &'a [u8],
}

impl<'a> FieldRef<'a> {
    /// Create a field reference over the given byte slice.
    #[inline(always)]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Raw bytes of this field.
    #[inline(always)]
    pub const fn as_bytes(&self) -> &[u8] {
        self.data
    }

    /// Read a `u8` from offset 0.
    #[inline(always)]
    pub fn read_u8(&self) -> Result<u8, ProgramError> {
        self.data.first().copied().ok_or(ProgramError::InvalidAccountData)
    }

    /// Read a little-endian `u16` from offset 0.
    #[inline(always)]
    pub fn read_u16(&self) -> Result<u16, ProgramError> {
        if self.data.len() < 2 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(u16::from_le_bytes([self.data[0], self.data[1]]))
    }

    /// Read a little-endian `u32` from offset 0.
    #[inline(always)]
    pub fn read_u32(&self) -> Result<u32, ProgramError> {
        if self.data.len() < 4 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(u32::from_le_bytes([
            self.data[0],
            self.data[1],
            self.data[2],
            self.data[3],
        ]))
    }

    /// Read a little-endian `u64` from offset 0.
    #[inline(always)]
    pub fn read_u64(&self) -> Result<u64, ProgramError> {
        if self.data.len() < 8 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(u64::from_le_bytes([
            self.data[0],
            self.data[1],
            self.data[2],
            self.data[3],
            self.data[4],
            self.data[5],
            self.data[6],
            self.data[7],
        ]))
    }

    /// Read a boolean from offset 0 (0 = false, non-zero = true).
    #[inline(always)]
    pub fn read_bool(&self) -> Result<bool, ProgramError> {
        self.read_u8().map(|v| v != 0)
    }

    /// Borrow as a 32-byte address reference.
    #[inline(always)]
    pub fn as_address(&self) -> Result<&[u8; 32], ProgramError> {
        if self.data.len() < 32 {
            return Err(ProgramError::InvalidAccountData);
        }
        // SAFETY: We checked length >= 32. Alignment is 1 for [u8; 32].
        Ok(unsafe { &*(self.data.as_ptr() as *const [u8; 32]) })
    }
}

/// Mutable typed view over a field's bytes.
pub struct FieldMut<'a> {
    data: &'a mut [u8],
}

impl<'a> FieldMut<'a> {
    /// Create a mutable field reference over the given byte slice.
    #[inline(always)]
    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data }
    }

    /// Read a `u8` from offset 0.
    #[inline(always)]
    pub fn read_u8(&self) -> Result<u8, ProgramError> {
        self.data.first().copied().ok_or(ProgramError::InvalidAccountData)
    }

    /// Read a little-endian `u64` from offset 0.
    #[inline(always)]
    pub fn read_u64(&self) -> Result<u64, ProgramError> {
        if self.data.len() < 8 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(u64::from_le_bytes([
            self.data[0],
            self.data[1],
            self.data[2],
            self.data[3],
            self.data[4],
            self.data[5],
            self.data[6],
            self.data[7],
        ]))
    }

    /// Write a `u8` at offset 0.
    #[inline(always)]
    pub fn write_u8(&mut self, v: u8) -> Result<(), ProgramError> {
        if self.data.is_empty() {
            return Err(ProgramError::InvalidAccountData);
        }
        self.data[0] = v;
        Ok(())
    }

    /// Write a little-endian `u16` at offset 0.
    #[inline(always)]
    pub fn write_u16(&mut self, v: u16) -> Result<(), ProgramError> {
        if self.data.len() < 2 {
            return Err(ProgramError::InvalidAccountData);
        }
        let bytes = v.to_le_bytes();
        self.data[0] = bytes[0];
        self.data[1] = bytes[1];
        Ok(())
    }

    /// Write a little-endian `u32` at offset 0.
    #[inline(always)]
    pub fn write_u32(&mut self, v: u32) -> Result<(), ProgramError> {
        if self.data.len() < 4 {
            return Err(ProgramError::InvalidAccountData);
        }
        let bytes = v.to_le_bytes();
        self.data[..4].copy_from_slice(&bytes);
        Ok(())
    }

    /// Write a little-endian `u64` at offset 0.
    #[inline(always)]
    pub fn write_u64(&mut self, v: u64) -> Result<(), ProgramError> {
        if self.data.len() < 8 {
            return Err(ProgramError::InvalidAccountData);
        }
        let bytes = v.to_le_bytes();
        self.data[..8].copy_from_slice(&bytes);
        Ok(())
    }

    /// Write a boolean (normalized to 0x00 or 0x01).
    #[inline(always)]
    pub fn write_bool(&mut self, v: bool) -> Result<(), ProgramError> {
        self.write_u8(v as u8)
    }

    /// Write a 32-byte address.
    #[inline(always)]
    pub fn write_address(&mut self, addr: &[u8; 32]) -> Result<(), ProgramError> {
        if self.data.len() < 32 {
            return Err(ProgramError::InvalidAccountData);
        }
        self.data[..32].copy_from_slice(addr);
        Ok(())
    }

    /// Copy raw bytes into this field.
    #[inline(always)]
    pub fn copy_from(&mut self, src: &[u8]) -> Result<(), ProgramError> {
        if self.data.len() < src.len() {
            return Err(ProgramError::InvalidAccountData);
        }
        self.data[..src.len()].copy_from_slice(src);
        Ok(())
    }

    /// Borrow the underlying bytes immutably.
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        self.data
    }

    /// Borrow the underlying bytes mutably.
    #[inline(always)]
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.data
    }
}
