//! State receipts and event emission.
//!
//! Receipts are Hopper's way of making state changes observable.
//! After mutating state, programs emit a receipt that can be:
//! - logged to the runtime for indexers/explorers
//! - returned as instruction data for CPI callers
//! - used by testing and auditing tools
//!
//! This is Hopper's current state-observability surface.
//! Raw receipts, tagged receipts, typed receipts, and CPI return data
//! are all available today, and the same wire formats feed schema and
//! manager tooling.

use hopper_runtime::ProgramResult;

/// A byte slice descriptor matching the Solana runtime's `SolBytes` ABI.
#[cfg(target_os = "solana")]
#[repr(C)]
struct SolBytes {
    ptr: *const u8,
    len: u64,
}

#[cfg(target_os = "solana")]
extern "C" {
    fn sol_log_data(data: *const SolBytes, data_len: u64);
    fn sol_set_return_data(data: *const u8, length: u64);
}

/// Emit a raw receipt (log the bytes to the runtime).
///
/// Uses `sol_log_data` on BPF, which emits structured binary data
/// that indexers and explorers can parse.
///
/// # Example
///
/// ```ignore
/// let data = amount.to_le_bytes();
/// emit_receipt(&data)?;
/// ```
#[inline(always)]
pub fn emit_receipt(data: &[u8]) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let field = SolBytes { ptr: data.as_ptr(), len: data.len() as u64 };
        unsafe {
            sol_log_data(&field as *const SolBytes, 1);
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = data;
    }
    Ok(())
}

/// Emit a receipt with a type tag prefix.
///
/// Prepends a single byte tag before the data, allowing indexers to
/// distinguish different receipt types from the same program.
#[inline(always)]
pub fn emit_tagged_receipt(tag: u8, data: &[u8]) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let tag_byte: [u8; 1] = [tag];
        let fields: [SolBytes; 2] = [
            SolBytes { ptr: tag_byte.as_ptr(), len: 1 },
            SolBytes { ptr: data.as_ptr(), len: data.len() as u64 },
        ];
        unsafe {
            sol_log_data(fields.as_ptr(), 2);
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (tag, data);
    }
    Ok(())
}

/// Set return data for the current instruction.
///
/// Makes data available to the calling program via `get_return_data()`.
/// Maximum 1024 bytes per Solana runtime limits.
#[inline(always)]
pub fn set_return_data(data: &[u8]) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        unsafe {
            sol_set_return_data(data.as_ptr(), data.len() as u64);
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = data;
    }
    Ok(())
}

/// Trait for types that can be emitted as receipts.
///
/// Implement this for your domain-specific receipt structs to get
/// typed emission via `emit_typed_receipt()`.
pub trait Receipt {
    /// Receipt type tag for indexer discrimination.
    const TAG: u8;

    /// Serialize the receipt to bytes.
    ///
    /// The returned slice must be valid for the lifetime of `self`.
    fn as_bytes(&self) -> &[u8];
}

/// Emit a typed receipt.
///
/// Automatically prepends the type tag from `Receipt::TAG`.
#[inline(always)]
pub fn emit_typed_receipt<T: Receipt>(receipt: &T) -> ProgramResult {
    emit_tagged_receipt(T::TAG, receipt.as_bytes())
}
