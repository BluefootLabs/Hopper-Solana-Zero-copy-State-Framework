//! Zero-copy overlay utilities.

use hopper_runtime::error::ProgramError;
use super::pod::{Pod, FixedLayout};

/// Overlay immutable reference to a Pod type at the start of a byte slice.
#[inline(always)]
pub fn overlay<T: Pod + FixedLayout>(data: &[u8]) -> Result<&T, ProgramError> {
    super::pod::pod_from_bytes(data)
}

/// Overlay mutable reference to a Pod type at the start of a byte slice.
#[inline(always)]
pub fn overlay_mut<T: Pod + FixedLayout>(data: &mut [u8]) -> Result<&mut T, ProgramError> {
    super::pod::pod_from_bytes_mut(data)
}

/// Overlay at a specific offset (immutable).
#[inline(always)]
#[allow(dead_code)]
pub fn overlay_at<T: Pod + FixedLayout>(data: &[u8], offset: usize) -> Result<&T, ProgramError> {
    if offset > data.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    super::pod::pod_from_bytes(&data[offset..])
}

/// Overlay at a specific offset (mutable).
#[inline(always)]
#[allow(dead_code)]
pub fn overlay_at_mut<T: Pod + FixedLayout>(
    data: &mut [u8],
    offset: usize,
) -> Result<&mut T, ProgramError> {
    if offset > data.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    super::pod::pod_from_bytes_mut(&mut data[offset..])
}
