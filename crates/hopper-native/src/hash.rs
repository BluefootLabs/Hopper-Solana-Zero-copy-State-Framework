//! Cryptographic hash functions via Solana syscalls.
//!
//! No existing Solana framework wraps `sol_sha256` or `sol_keccak256`
//! with ergonomic APIs at the raw substrate level. Programs that need
//! hashing either pull in heavy crates or write unsafe syscall glue
//! every time.
//!
//! Hopper wraps these syscalls with safe, zero-alloc APIs.

use crate::error::ProgramError;

/// SHA-256 hash output: 32 bytes.
pub type Sha256Hash = [u8; 32];

/// Keccak-256 hash output: 32 bytes.
pub type Keccak256Hash = [u8; 32];

/// BLAKE3 hash output: 32 bytes.
pub type Blake3Hash = [u8; 32];

/// Maximum number of byte slices accepted by Solana hash syscalls.
pub const MAX_HASH_SEGMENTS: usize = 16;

/// Compute SHA-256 over one or more byte slices.
///
/// The Solana `sol_sha256` syscall accepts a vector of (ptr, len) pairs,
/// so multi-part hashing is done in a single syscall without concatenation.
///
/// # Example
///
/// ```ignore
/// let hash = sha256(&[b"hello", b" world"])?;
/// ```
#[inline]
#[allow(unused_mut)]
pub fn sha256(inputs: &[&[u8]]) -> Result<Sha256Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];

    #[cfg(target_os = "solana")]
    {
        // Build the parameter array: each element is (ptr, len) as two u64s.
        let count = inputs.len();
        let mut params: [u64; 32] = [0; 32];
        let mut i = 0;
        while i < count {
            params[i * 2] = inputs[i].as_ptr() as u64;
            params[i * 2 + 1] = inputs[i].len() as u64;
            i += 1;
        }

        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let rc = unsafe {
            crate::syscalls::sol_sha256(
                params.as_ptr() as *const u8,
                count as u64,
                result.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(ProgramError::InvalidArgument);
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = inputs;
        // Off-chain: return zeroed hash (tests should use a software
        // implementation if they need real hashes).
    }

    Ok(result)
}

/// Compute SHA-256 over a single byte slice.
#[inline]
pub fn sha256_single(input: &[u8]) -> Result<Sha256Hash, ProgramError> {
    sha256(&[input])
}

/// Compute Keccak-256 over one or more byte slices.
///
/// Same multi-part API as `sha256`. Keccak-256 is the hash function used
/// by Ethereum's `keccak256()` and by Solana's secp256k1 precompile.
#[inline]
#[allow(unused_mut)]
pub fn keccak256(inputs: &[&[u8]]) -> Result<Keccak256Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];

    #[cfg(target_os = "solana")]
    {
        let count = inputs.len();
        let mut params: [u64; 32] = [0; 32];
        let mut i = 0;
        while i < count {
            params[i * 2] = inputs[i].as_ptr() as u64;
            params[i * 2 + 1] = inputs[i].len() as u64;
            i += 1;
        }

        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let rc = unsafe {
            crate::syscalls::sol_keccak256(
                params.as_ptr() as *const u8,
                count as u64,
                result.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(ProgramError::InvalidArgument);
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = inputs;
    }

    Ok(result)
}

/// Compute Keccak-256 over a single byte slice.
#[inline]
pub fn keccak256_single(input: &[u8]) -> Result<Keccak256Hash, ProgramError> {
    keccak256(&[input])
}

/// Compute BLAKE3 over one or more byte slices.
#[inline]
#[allow(unused_mut)]
pub fn blake3(inputs: &[&[u8]]) -> Result<Blake3Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }

    let mut result = [0u8; 32];

    #[cfg(target_os = "solana")]
    {
        let count = inputs.len();
        let mut params: [u64; 32] = [0; 32];
        let mut i = 0;
        while i < count {
            params[i * 2] = inputs[i].as_ptr() as u64;
            params[i * 2 + 1] = inputs[i].len() as u64;
            i += 1;
        }

        // SAFETY: params contains `count` slice descriptors and `result` is
        // a 32-byte writable hash output buffer.
        let rc = unsafe {
            crate::syscalls::sol_blake3(
                params.as_ptr() as *const u8,
                count as u64,
                result.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(ProgramError::InvalidArgument);
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = inputs;
    }

    Ok(result)
}

/// Compute BLAKE3 over a single byte slice.
#[inline]
pub fn blake3_single(input: &[u8]) -> Result<Blake3Hash, ProgramError> {
    blake3(&[input])
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &[u8] = b"";

    #[test]
    fn sha256_accepts_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS];
        assert_eq!(sha256(&inputs), Ok([0; 32]));
    }

    #[test]
    fn keccak256_accepts_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS];
        assert_eq!(keccak256(&inputs), Ok([0; 32]));
    }

    #[test]
    fn blake3_accepts_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS];
        assert_eq!(blake3(&inputs), Ok([0; 32]));
    }

    #[test]
    fn sha256_rejects_more_than_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS + 1];
        assert_eq!(sha256(&inputs), Err(ProgramError::InvalidArgument));
    }

    #[test]
    fn keccak256_rejects_more_than_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS + 1];
        assert_eq!(keccak256(&inputs), Err(ProgramError::InvalidArgument));
    }

    #[test]
    fn blake3_rejects_more_than_sixteen_segments() {
        let inputs = [EMPTY; MAX_HASH_SEGMENTS + 1];
        assert_eq!(blake3(&inputs), Err(ProgramError::InvalidArgument));
    }
}
