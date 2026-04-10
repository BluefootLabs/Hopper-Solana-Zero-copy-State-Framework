//! Zero-allocation event emission via `sol_log_data`.
//!
//! Events are emitted as raw Pod bytes through the `sol_log_data` syscall.
//! This is ~100 CU and zero-allocation. For unforgeable events, use self-CPI
//! (available via hopper-solana).

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

/// Emit a Pod event via `sol_log_data`.
///
/// The event is logged as raw bytes. Clients decode using the schema manifest
/// or known layout. Costs ~100 CU, zero allocation.
#[inline(always)]
pub fn emit_event<T: Pod + FixedLayout>(value: &T) -> Result<(), ProgramError> {
    // SAFETY: T: Pod guarantees all bit patterns valid and no padding invariants.
    // The resulting slice covers exactly T::SIZE bytes from a valid reference.
    let bytes = unsafe {
        core::slice::from_raw_parts(value as *const T as *const u8, T::SIZE)
    };
    emit_slices(&[bytes]);
    Ok(())
}

/// Emit event with a discriminator prefix for easy client-side filtering.
///
/// Layout: `[event_disc: u8][event_data: T::SIZE bytes]`
#[inline]
pub fn emit_event_tagged<T: Pod + FixedLayout>(disc: u8, value: &T) -> Result<(), ProgramError> {
    // SAFETY: T: Pod guarantees all bit patterns valid. Slice covers T::SIZE bytes.
    let value_bytes = unsafe {
        core::slice::from_raw_parts(value as *const T as *const u8, T::SIZE)
    };
    let disc_bytes = [disc];
    emit_slices(&[&disc_bytes[..], value_bytes]);
    Ok(())
}

/// Emit one or more byte slices as a single `sol_log_data` entry.
#[inline(always)]
pub fn emit_slices(segments: &[&[u8]]) {
    #[cfg(target_os = "solana")]
    {
        // SAFETY: segments is a valid slice of (ptr, len) pairs as expected
        // by the sol_log_data syscall. BPF ABI guarantees layout compatibility.
        unsafe {
            hopper_runtime::syscalls::sol_log_data(
                segments.as_ptr() as *const u8,
                segments.len() as u64,
            );
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = segments;
    }
}
