//! Segment table and segment slices for variable-length accounts.
//!
//! A segmented account has:
//! ```text
//! [Fixed Prefix][Segment Table][Segment 0 Data][Segment 1 Data]...
//! ```
//!
//! Each segment descriptor is 12 bytes:
//! ```text
//! [offset: u32][count: u16][capacity: u16][element_size: u16][flags: u16]
//! ```

use hopper_runtime::error::ProgramError;
use super::pod::{Pod, FixedLayout};

/// Size of one segment descriptor in bytes.
pub const SEGMENT_DESC_SIZE: usize = 12;

/// Maximum number of segments per account.
pub const MAX_SEGMENTS: usize = 256;

/// A single segment descriptor.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SegmentDescriptor {
    offset_bytes: [u8; 4],
    count_bytes: [u8; 2],
    capacity_bytes: [u8; 2],
    element_size_bytes: [u8; 2],
    flags_bytes: [u8; 2],
}

const _: () = assert!(core::mem::size_of::<SegmentDescriptor>() == SEGMENT_DESC_SIZE);
const _: () = assert!(core::mem::align_of::<SegmentDescriptor>() == 1);

// Bytemuck proof (Hopper Safety Audit Must-Fix #5).
#[cfg(feature = "hopper-native-backend")]
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Zeroable for SegmentDescriptor {}
#[cfg(feature = "hopper-native-backend")]
unsafe impl ::hopper_runtime::__hopper_native::bytemuck::Pod for SegmentDescriptor {}

// SAFETY: All fields are [u8; N], all bit patterns valid.
unsafe impl Pod for SegmentDescriptor {}
// Audit Step 5 seal: Hopper-authored primitive.
unsafe impl ::hopper_runtime::__sealed::HopperZeroCopySealed for SegmentDescriptor {}

impl FixedLayout for SegmentDescriptor {
    const SIZE: usize = SEGMENT_DESC_SIZE;
}

impl SegmentDescriptor {
    /// Data region byte offset within the account.
    #[inline(always)]
    pub fn offset(&self) -> u32 {
        u32::from_le_bytes(self.offset_bytes)
    }

    /// Current element count.
    #[inline(always)]
    pub fn count(&self) -> u16 {
        u16::from_le_bytes(self.count_bytes)
    }

    /// Maximum element capacity.
    #[inline(always)]
    pub fn capacity(&self) -> u16 {
        u16::from_le_bytes(self.capacity_bytes)
    }

    /// Size of each element in bytes.
    #[inline(always)]
    pub fn element_size(&self) -> u16 {
        u16::from_le_bytes(self.element_size_bytes)
    }

    /// Flags.
    #[inline(always)]
    pub fn flags(&self) -> u16 {
        u16::from_le_bytes(self.flags_bytes)
    }

    /// Whether the segment is at capacity.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.count() >= self.capacity()
    }

    /// Total data bytes used by this segment (count * element_size).
    #[inline(always)]
    pub fn data_len(&self) -> usize {
        (self.count() as usize) * (self.element_size() as usize)
    }

    /// Total data bytes allocated (capacity * element_size).
    #[inline(always)]
    pub fn allocated_len(&self) -> usize {
        (self.capacity() as usize) * (self.element_size() as usize)
    }

    /// Set the count.
    #[inline(always)]
    pub fn set_count(&mut self, count: u16) {
        self.count_bytes = count.to_le_bytes();
    }

    /// Set the offset.
    #[inline(always)]
    pub fn set_offset(&mut self, offset: u32) {
        self.offset_bytes = offset.to_le_bytes();
    }

    /// Set the capacity.
    #[inline(always)]
    pub fn set_capacity(&mut self, capacity: u16) {
        self.capacity_bytes = capacity.to_le_bytes();
    }

    /// Set the element size.
    #[inline(always)]
    pub fn set_element_size(&mut self, size: u16) {
        self.element_size_bytes = size.to_le_bytes();
    }
}

/// Read-only segment table.
pub struct SegmentTable<'a> {
    data: &'a [u8],
    count: usize,
}

impl<'a> SegmentTable<'a> {
    /// Parse a segment table from raw bytes.
    #[inline]
    pub fn from_bytes(data: &'a [u8], count: usize) -> Result<Self, ProgramError> {
        if data.len() < count * SEGMENT_DESC_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self { data, count })
    }

    /// Number of segments.
    #[inline(always)]
    pub fn segment_count(&self) -> usize {
        self.count
    }

    /// Get a descriptor by index.
    #[inline(always)]
    pub fn descriptor(&self, index: usize) -> Result<&SegmentDescriptor, ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = index * SEGMENT_DESC_SIZE;
        // SAFETY: Bounds checked. SegmentDescriptor is alignment-1.
        Ok(unsafe { &*(self.data.as_ptr().add(offset) as *const SegmentDescriptor) })
    }
}

/// Mutable segment table.
pub struct SegmentTableMut<'a> {
    data: &'a mut [u8],
    count: usize,
}

impl<'a> SegmentTableMut<'a> {
    /// Parse a mutable segment table from raw bytes.
    #[inline]
    pub fn from_bytes_mut(data: &'a mut [u8], count: usize) -> Result<Self, ProgramError> {
        if data.len() < count * SEGMENT_DESC_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self { data, count })
    }

    /// Get a mutable descriptor by index.
    #[inline(always)]
    pub fn descriptor_mut(&mut self, index: usize) -> Result<&mut SegmentDescriptor, ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = index * SEGMENT_DESC_SIZE;
        // SAFETY: Bounds checked. SegmentDescriptor is alignment-1. Exclusive access.
        Ok(unsafe { &mut *(self.data.as_mut_ptr().add(offset) as *mut SegmentDescriptor) })
    }

    /// Initialize segment descriptors with given specifications.
    ///
    /// `specs` is `(element_size, count, capacity)` per segment.
    /// `data_start` is the byte offset where segment data begins.
    #[inline]
    pub fn init(
        &mut self,
        data_start: u32,
        specs: &[(u16, u16, u16)],
    ) -> Result<(), ProgramError> {
        if specs.len() > self.count {
            return Err(ProgramError::InvalidArgument);
        }

        let mut current_offset = data_start;
        for (i, &(element_size, count, capacity)) in specs.iter().enumerate() {
            let desc = self.descriptor_mut(i)?;
            desc.set_offset(current_offset);
            desc.set_count(count);
            desc.set_capacity(capacity);
            desc.set_element_size(element_size);
            current_offset += (capacity as u32) * (element_size as u32);
        }
        Ok(())
    }
}

/// Type-safe immutable slice over a segment's elements.
pub struct SegmentSlice<'a, T: Pod + FixedLayout> {
    data: &'a [u8],
    count: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> SegmentSlice<'a, T> {
    /// Create from a segment descriptor and raw account data.
    #[inline]
    pub fn from_descriptor(
        account_data: &'a [u8],
        desc: &SegmentDescriptor,
    ) -> Result<Self, ProgramError> {
        let offset = desc.offset() as usize;
        let count = desc.count() as usize;
        let needed = offset + count * T::SIZE;
        if needed > account_data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data: &account_data[offset..],
            count,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Number of elements.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the segment slice is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Read element at index.
    #[inline(always)]
    pub fn read(&self, index: usize) -> Result<T, ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = index * T::SIZE;
        // SAFETY: Bounds checked. T: Pod, alignment-1.
        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T) })
    }

    /// Get element reference at index.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Result<&T, ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = index * T::SIZE;
        // SAFETY: Bounds checked. T: Pod, alignment-1.
        Ok(unsafe { &*(self.data.as_ptr().add(offset) as *const T) })
    }
}

/// Type-safe mutable slice over a segment's elements.
pub struct SegmentSliceMut<'a, T: Pod + FixedLayout> {
    data: &'a mut [u8],
    count: usize,
    capacity: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> SegmentSliceMut<'a, T> {
    /// Create from a segment descriptor and raw mutable account data.
    #[inline]
    pub fn from_descriptor(
        account_data: &'a mut [u8],
        desc: &SegmentDescriptor,
    ) -> Result<Self, ProgramError> {
        let offset = desc.offset() as usize;
        let count = desc.count() as usize;
        let capacity = desc.capacity() as usize;
        let needed = offset + capacity * T::SIZE;
        if needed > account_data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data: &mut account_data[offset..],
            count,
            capacity,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Number of elements.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the segment slice is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Maximum capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Read element at index (copy).
    #[inline(always)]
    pub fn read(&self, index: usize) -> Result<T, ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = index * T::SIZE;
        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T) })
    }

    /// Write element at index.
    #[inline(always)]
    pub fn write(&mut self, index: usize, value: T) -> Result<(), ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = index * T::SIZE;
        // SAFETY: Bounds checked. T: Pod. Exclusive access.
        unsafe {
            core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut T, value);
        }
        Ok(())
    }

    /// Swap two elements.
    #[inline]
    pub fn swap(&mut self, i: usize, j: usize) -> Result<(), ProgramError> {
        if i >= self.count || j >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        if i == j {
            return Ok(());
        }
        let size = T::SIZE;
        let oi = i * size;
        let oj = j * size;
        // Swap byte-by-byte to avoid alignment issues
        for k in 0..size {
            self.data.swap(oi + k, oj + k);
        }
        Ok(())
    }
}
