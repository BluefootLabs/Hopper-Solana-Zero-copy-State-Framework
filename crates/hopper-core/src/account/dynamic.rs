//! Inline dynamic fields -- Quasar-inspired prefix-based variable-length data.
//!
//! For accounts with 1-3 variable-length fields (strings, byte arrays),
//! inline dynamic fields are more efficient than a full segment table.
//!
//! Wire format:
//! ```text
//! [fixed_prefix][prefix_1: LenType][data_1: N bytes][prefix_2: LenType][data_2: M bytes]...
//! ```
//!
//! Each dynamic field is preceded by a length prefix (u8, u16, or u32)
//! indicating the actual length of the data that follows. Maximum length
//! is set at layout definition time.
//!
//! ## Key Design
//!
//! - **Offset caching**: On first parse, walk the byte stream once and cache
//!   cumulative offsets in a `[u32; N]` stack array. All subsequent field
//!   accesses are O(1) via the cached offsets.
//! - **Layout_id integration**: Dynamic field names/types/max sizes are part
//!   of the layout_id hash, so schema changes are detected.
//! - **Zero-copy reads**: String/byte slice access returns `&[u8]` directly
//!   from account data -- no copy needed for reads.
//!
//! ## Comparison to Segments
//!
//! | | Segments | Inline Dynamic |
//! |---|---|---|
//! | Overhead per field | 12 bytes (descriptor) | 1-4 bytes (prefix) |
//! | Best for | Multiple large arrays | 1-3 small variable fields |
//! | Capacity tracking | Explicit | Implicit (max from layout) |
//! | Cross-program readable | Self-describing | Requires schema knowledge |

use hopper_runtime::error::ProgramError;

/// Read a u8-prefixed dynamic field: returns (data_slice, next_offset).
#[inline(always)]
pub fn read_dynamic_u8(data: &[u8], offset: usize) -> Result<(&[u8], usize), ProgramError> {
    if offset >= data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let len = data[offset] as usize;
    let data_start = offset + 1;
    let data_end = data_start + len;
    if data_end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok((&data[data_start..data_end], data_end))
}

/// Read a u16-prefixed dynamic field.
#[inline(always)]
pub fn read_dynamic_u16(data: &[u8], offset: usize) -> Result<(&[u8], usize), ProgramError> {
    if offset + 2 > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
    let data_start = offset + 2;
    let data_end = data_start + len;
    if data_end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok((&data[data_start..data_end], data_end))
}

/// Read a u32-prefixed dynamic field.
#[inline(always)]
pub fn read_dynamic_u32(data: &[u8], offset: usize) -> Result<(&[u8], usize), ProgramError> {
    if offset + 4 > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let len = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
    let data_start = offset + 4;
    let data_end = data_start + len;
    if data_end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok((&data[data_start..data_end], data_end))
}

/// Write a u8-prefixed dynamic field. Returns next offset after written data.
#[inline(always)]
pub fn write_dynamic_u8(
    data: &mut [u8],
    offset: usize,
    value: &[u8],
    max_len: usize,
) -> Result<usize, ProgramError> {
    if value.len() > max_len || value.len() > 255 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let data_start = offset + 1;
    let data_end = data_start + value.len();
    if data_end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    data[offset] = value.len() as u8;
    data[data_start..data_end].copy_from_slice(value);
    Ok(data_end)
}

/// Write a u16-prefixed dynamic field.
#[inline(always)]
pub fn write_dynamic_u16(
    data: &mut [u8],
    offset: usize,
    value: &[u8],
    max_len: usize,
) -> Result<usize, ProgramError> {
    if value.len() > max_len || value.len() > 65535 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let data_start = offset + 2;
    let data_end = data_start + value.len();
    if data_end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let len_bytes = (value.len() as u16).to_le_bytes();
    data[offset] = len_bytes[0];
    data[offset + 1] = len_bytes[1];
    data[data_start..data_end].copy_from_slice(value);
    Ok(data_end)
}

/// Write a u32-prefixed dynamic field.
#[inline(always)]
pub fn write_dynamic_u32(
    data: &mut [u8],
    offset: usize,
    value: &[u8],
    max_len: usize,
) -> Result<usize, ProgramError> {
    if value.len() > max_len {
        return Err(ProgramError::InvalidInstructionData);
    }
    let data_start = offset + 4;
    let data_end = data_start + value.len();
    if data_end > data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let len_bytes = (value.len() as u32).to_le_bytes();
    data[offset] = len_bytes[0];
    data[offset + 1] = len_bytes[1];
    data[offset + 2] = len_bytes[2];
    data[offset + 3] = len_bytes[3];
    data[data_start..data_end].copy_from_slice(value);
    Ok(data_end)
}

/// An inline dynamic view that caches offsets for O(1) field access.
///
/// Created by walking the prefix bytes once, then all accessors
/// use the cached offsets.
///
/// Generic over N = number of dynamic fields.
pub struct DynamicView<'a, const N: usize> {
    /// Raw account data
    data: &'a [u8],
    /// Cached byte offsets: offsets[i] = byte offset where dynamic field i starts
    /// (after its length prefix)
    offsets: [u32; N],
    /// Cached lengths for each dynamic field
    lengths: [u32; N],
}

impl<'a, const N: usize> DynamicView<'a, N> {
    /// Parse dynamic fields starting at `base_offset` in `data`.
    ///
    /// `prefix_sizes` indicates prefix type per field: 1 = u8, 2 = u16, 4 = u32.
    #[inline]
    pub fn parse(
        data: &'a [u8],
        base_offset: usize,
        prefix_sizes: &[u8; N],
    ) -> Result<Self, ProgramError> {
        let mut offsets = [0u32; N];
        let mut lengths = [0u32; N];
        let mut cursor = base_offset;

        let mut i = 0;
        while i < N {
            let prefix_size = prefix_sizes[i] as usize;
            if cursor + prefix_size > data.len() {
                return Err(ProgramError::AccountDataTooSmall);
            }
            let len = match prefix_size {
                1 => data[cursor] as u32,
                2 => {
                    u16::from_le_bytes([data[cursor], data[cursor + 1]]) as u32
                }
                4 => {
                    u32::from_le_bytes([
                        data[cursor],
                        data[cursor + 1],
                        data[cursor + 2],
                        data[cursor + 3],
                    ])
                }
                _ => return Err(ProgramError::InvalidInstructionData),
            };
            let data_start = cursor + prefix_size;
            let data_end = data_start + len as usize;
            if data_end > data.len() {
                return Err(ProgramError::AccountDataTooSmall);
            }
            offsets[i] = data_start as u32;
            lengths[i] = len;
            cursor = data_end;
            i += 1;
        }

        Ok(Self {
            data,
            offsets,
            lengths,
        })
    }

    /// Get the byte slice for dynamic field at index. O(1) after initial parse.
    #[inline(always)]
    pub fn field(&self, index: usize) -> &[u8] {
        let offset = self.offsets[index] as usize;
        let len = self.lengths[index] as usize;
        &self.data[offset..offset + len]
    }

    /// Get the length of dynamic field at index.
    #[inline(always)]
    pub fn field_len(&self, index: usize) -> usize {
        self.lengths[index] as usize
    }

    /// Try to interpret a dynamic field as a UTF-8 string.
    #[inline]
    pub fn field_as_str(&self, index: usize) -> Result<&str, ProgramError> {
        core::str::from_utf8(self.field(index)).map_err(|_| ProgramError::InvalidAccountData)
    }

    /// Total bytes consumed by all dynamic fields (including prefixes).
    #[inline]
    pub fn total_dynamic_bytes(&self) -> usize {
        if N == 0 {
            return 0;
        }
        let last_offset = self.offsets[N - 1] as usize;
        let last_len = self.lengths[N - 1] as usize;
        last_offset + last_len
    }
}

/// Mutable inline dynamic view for writing dynamic fields.
pub struct DynamicViewMut<'a, const N: usize> {
    data: &'a mut [u8],
    offsets: [u32; N],
    lengths: [u32; N],
}

impl<'a, const N: usize> DynamicViewMut<'a, N> {
    /// Parse dynamic fields starting at `base_offset` in mutable `data`.
    #[inline]
    pub fn parse(
        data: &'a mut [u8],
        base_offset: usize,
        prefix_sizes: &[u8; N],
    ) -> Result<Self, ProgramError> {
        let mut offsets = [0u32; N];
        let mut lengths = [0u32; N];
        let mut cursor = base_offset;

        let mut i = 0;
        while i < N {
            let prefix_size = prefix_sizes[i] as usize;
            if cursor + prefix_size > data.len() {
                return Err(ProgramError::AccountDataTooSmall);
            }
            let len = match prefix_size {
                1 => data[cursor] as u32,
                2 => u16::from_le_bytes([data[cursor], data[cursor + 1]]) as u32,
                4 => u32::from_le_bytes([
                    data[cursor],
                    data[cursor + 1],
                    data[cursor + 2],
                    data[cursor + 3],
                ]),
                _ => return Err(ProgramError::InvalidInstructionData),
            };
            let data_start = cursor + prefix_size;
            let data_end = data_start + len as usize;
            if data_end > data.len() {
                return Err(ProgramError::AccountDataTooSmall);
            }
            offsets[i] = data_start as u32;
            lengths[i] = len;
            cursor = data_end;
            i += 1;
        }

        Ok(Self {
            data,
            offsets,
            lengths,
        })
    }

    /// Get immutable reference to a dynamic field.
    #[inline(always)]
    pub fn field(&self, index: usize) -> &[u8] {
        let offset = self.offsets[index] as usize;
        let len = self.lengths[index] as usize;
        &self.data[offset..offset + len]
    }

    /// Get the length of a dynamic field.
    #[inline(always)]
    pub fn field_len(&self, index: usize) -> usize {
        self.lengths[index] as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_u8_roundtrip() {
        let mut buf = [0u8; 64];
        let next = write_dynamic_u8(&mut buf, 0, b"hello", 32).unwrap();
        assert_eq!(next, 6); // 1 prefix + 5 data
        let (data, next2) = read_dynamic_u8(&buf, 0).unwrap();
        assert_eq!(data, b"hello");
        assert_eq!(next2, 6);
    }

    #[test]
    fn dynamic_u16_roundtrip() {
        let mut buf = [0u8; 64];
        let next = write_dynamic_u16(&mut buf, 0, b"world!", 32).unwrap();
        assert_eq!(next, 8); // 2 prefix + 6 data
        let (data, next2) = read_dynamic_u16(&buf, 0).unwrap();
        assert_eq!(data, b"world!");
        assert_eq!(next2, 8);
    }

    #[test]
    fn dynamic_view_parse_and_access() {
        let mut buf = [0u8; 128];
        // Write two u8-prefixed fields
        let off = write_dynamic_u8(&mut buf, 0, b"alice", 32).unwrap();
        let _off2 = write_dynamic_u8(&mut buf, off, b"this is a bio", 128).unwrap();

        let view = DynamicView::<2>::parse(&buf, 0, &[1, 1]).unwrap();
        assert_eq!(view.field(0), b"alice");
        assert_eq!(view.field(1), b"this is a bio");
        assert_eq!(view.field_as_str(0).unwrap(), "alice");
    }

    #[test]
    fn dynamic_view_mixed_prefixes() {
        let mut buf = [0u8; 128];
        // u8 prefix for short string, u16 prefix for longer data
        let off1 = write_dynamic_u8(&mut buf, 0, b"hi", 32).unwrap();
        let _off2 = write_dynamic_u16(&mut buf, off1, b"longer data here", 256).unwrap();

        let view = DynamicView::<2>::parse(&buf, 0, &[1, 2]).unwrap();
        assert_eq!(view.field(0), b"hi");
        assert_eq!(view.field(1), b"longer data here");
    }
}
