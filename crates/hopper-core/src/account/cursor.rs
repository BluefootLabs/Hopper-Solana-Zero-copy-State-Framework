//! Sequential read/write cursors for zero-copy instruction and account data.

use hopper_runtime::error::ProgramError;

/// Sequential read cursor over a byte slice.
///
/// Tracks a position and provides typed reads that advance it.
/// Used for parsing instruction data and reading account fields sequentially.
pub struct SliceCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SliceCursor<'a> {
    /// Create a new cursor at position 0.
    #[inline(always)]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Create a cursor from instruction data, validating minimum length.
    #[inline(always)]
    pub fn from_instruction(data: &'a [u8], min_len: usize) -> Result<Self, ProgramError> {
        if data.len() < min_len {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(Self::new(data))
    }

    /// Current byte position.
    #[inline(always)]
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// Remaining bytes after current position.
    #[inline(always)]
    pub const fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Reference to remaining data from current position.
    #[inline(always)]
    pub fn data_from_position(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    /// Skip `n` bytes forward.
    #[inline(always)]
    pub fn skip(&mut self, n: usize) -> Result<(), ProgramError> {
        if self.remaining() < n {
            return Err(ProgramError::InvalidAccountData);
        }
        self.pos += n;
        Ok(())
    }

    /// Read a `u8` and advance.
    #[inline(always)]
    pub fn read_u8(&mut self) -> Result<u8, ProgramError> {
        if self.remaining() < 1 {
            return Err(ProgramError::InvalidAccountData);
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    /// Read a little-endian `u16` and advance.
    #[inline(always)]
    pub fn read_u16(&mut self) -> Result<u16, ProgramError> {
        if self.remaining() < 2 {
            return Err(ProgramError::InvalidAccountData);
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    /// Read a little-endian `u32` and advance.
    #[inline(always)]
    pub fn read_u32(&mut self) -> Result<u32, ProgramError> {
        if self.remaining() < 4 {
            return Err(ProgramError::InvalidAccountData);
        }
        let v = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    /// Read a little-endian `u64` and advance.
    #[inline(always)]
    pub fn read_u64(&mut self) -> Result<u64, ProgramError> {
        if self.remaining() < 8 {
            return Err(ProgramError::InvalidAccountData);
        }
        let v = u64::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
            self.data[self.pos + 4],
            self.data[self.pos + 5],
            self.data[self.pos + 6],
            self.data[self.pos + 7],
        ]);
        self.pos += 8;
        Ok(v)
    }

    /// Read a little-endian `i64` and advance.
    #[inline(always)]
    pub fn read_i64(&mut self) -> Result<i64, ProgramError> {
        self.read_u64().map(|v| v as i64)
    }

    /// Read a boolean (0 = false, non-zero = true) and advance.
    #[inline(always)]
    pub fn read_bool(&mut self) -> Result<bool, ProgramError> {
        self.read_u8().map(|v| v != 0)
    }

    /// Read a 32-byte address and advance.
    #[inline(always)]
    pub fn read_address(&mut self) -> Result<&'a [u8; 32], ProgramError> {
        if self.remaining() < 32 {
            return Err(ProgramError::InvalidAccountData);
        }
        // SAFETY: We checked length. [u8; 32] has alignment 1.
        let addr = unsafe { &*(self.data.as_ptr().add(self.pos) as *const [u8; 32]) };
        self.pos += 32;
        Ok(addr)
    }

    /// Read a fixed-size byte array and advance.
    #[inline(always)]
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], ProgramError> {
        if self.remaining() < len {
            return Err(ProgramError::InvalidAccountData);
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }
}

/// Sequential write cursor over a mutable byte slice.
pub struct DataWriter<'a> {
    data: &'a mut [u8],
    pos: usize,
}

impl<'a> DataWriter<'a> {
    /// Create a new writer at position 0.
    #[inline(always)]
    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// How many bytes have been written.
    #[inline(always)]
    pub const fn written(&self) -> usize {
        self.pos
    }

    /// Remaining writable bytes.
    #[inline(always)]
    pub const fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Write a `u8` and advance.
    #[inline(always)]
    pub fn write_u8(&mut self, v: u8) -> Result<(), ProgramError> {
        if self.remaining() < 1 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        self.data[self.pos] = v;
        self.pos += 1;
        Ok(())
    }

    /// Write a little-endian `u16` and advance.
    #[inline(always)]
    pub fn write_u16(&mut self, v: u16) -> Result<(), ProgramError> {
        if self.remaining() < 2 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let bytes = v.to_le_bytes();
        self.data[self.pos] = bytes[0];
        self.data[self.pos + 1] = bytes[1];
        self.pos += 2;
        Ok(())
    }

    /// Write a little-endian `u32` and advance.
    #[inline(always)]
    pub fn write_u32(&mut self, v: u32) -> Result<(), ProgramError> {
        if self.remaining() < 4 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let bytes = v.to_le_bytes();
        self.data[self.pos..self.pos + 4].copy_from_slice(&bytes);
        self.pos += 4;
        Ok(())
    }

    /// Write a little-endian `u64` and advance.
    #[inline(always)]
    pub fn write_u64(&mut self, v: u64) -> Result<(), ProgramError> {
        if self.remaining() < 8 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let bytes = v.to_le_bytes();
        self.data[self.pos..self.pos + 8].copy_from_slice(&bytes);
        self.pos += 8;
        Ok(())
    }

    /// Write a boolean (normalized to 0x00 or 0x01) and advance.
    #[inline(always)]
    pub fn write_bool(&mut self, v: bool) -> Result<(), ProgramError> {
        self.write_u8(v as u8)
    }

    /// Write a 32-byte address and advance.
    #[inline(always)]
    pub fn write_address(&mut self, addr: &[u8; 32]) -> Result<(), ProgramError> {
        if self.remaining() < 32 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        self.data[self.pos..self.pos + 32].copy_from_slice(addr);
        self.pos += 32;
        Ok(())
    }

    /// Write raw bytes and advance.
    #[inline(always)]
    pub fn write_bytes(&mut self, src: &[u8]) -> Result<(), ProgramError> {
        if self.remaining() < src.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        self.data[self.pos..self.pos + src.len()].copy_from_slice(src);
        self.pos += src.len();
        Ok(())
    }
}
