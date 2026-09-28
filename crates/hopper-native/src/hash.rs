//! Cryptographic hash functions via Solana syscalls.
//!
//! No existing Solana framework wraps `sol_sha256` or `sol_keccak256`
//! with ergonomic APIs at the raw substrate level. Programs that need
//! hashing either pull in heavy crates or write unsafe syscall glue
//! every time.
//!
//! Hopper wraps these syscalls with safe, zero-alloc APIs. Each wrapper
//! hands the `&[&[u8]]` straight to the syscall (its in-memory shape is
//! the `(ptr, len)` array the runtime reads), writes the digest into an
//! uninitialized output buffer, and returns without inspecting a result
//! code: the Agave hash syscalls return zero or abort the transaction
//! (compute exhaustion, an unmapped slice), so there is no error value to
//! branch on. Off-chain, `sha256` runs the const implementation in
//! [`crate::sha256`]; the other digests return all zeros, as documented on
//! each function.

use crate::error::ProgramError;
#[cfg(target_os = "solana")]
use core::mem::MaybeUninit;

/// SHA-256 hash output: 32 bytes.
pub type Sha256Hash = [u8; 32];

/// Keccak-256 hash output: 32 bytes.
pub type Keccak256Hash = [u8; 32];

/// BLAKE3 hash output: 32 bytes.
pub type Blake3Hash = [u8; 32];

/// SHA-512 hash output: 64 bytes.
#[cfg(feature = "sha512-syscall")]
pub type Sha512Hash = [u8; 64];

/// Maximum number of byte slices one hash syscall accepts: the runtime's
/// `sha256_max_slices` (Agave `execution_budget.rs`), which applies to
/// every hash syscall. Beyond it the syscall aborts the transaction, so the
/// wrappers refuse first with `InvalidArgument`. (An earlier release capped
/// this at 16 and called that the runtime limit; it was not.)
pub const MAX_HASH_SEGMENTS: usize = 20_000;

#[cfg(target_os = "solana")]
macro_rules! hash_syscall {
    ($syscall:ident, $inputs:expr, $out:expr) => {{
        // The syscall reads `inputs.len()` (ptr, len) pairs of 8-byte
        // words, exactly the in-memory shape of a `&[&[u8]]` on the SBF
        // target, so the slice is handed over directly instead of being
        // repacked through a staging buffer.
        const _: () = assert!(core::mem::size_of::<&[u8]>() == 16);
        // SAFETY: `$inputs` is `$inputs.len()` slice descriptors the
        // runtime translates; `$out` is a writable buffer of the digest's
        // exact length that the syscall fills completely before returning
        // (it returns zero or aborts the transaction, never a partial
        // write with an error code).
        unsafe {
            crate::syscalls::$syscall(
                $inputs.as_ptr() as *const u8,
                $inputs.len() as u64,
                $out.as_mut_ptr() as *mut u8,
            );
        }
    }};
}

/// Compute SHA-256 over one or more byte slices.
///
/// The Solana `sol_sha256` syscall accepts a vector of (ptr, len) pairs,
/// so multi-part hashing is done in a single syscall without concatenation.
/// Off-chain this runs the const SHA-256 in [`crate::sha256`], so host
/// tests see the real digest.
///
/// # Example
///
/// ```ignore
/// let hash = sha256(&[b"hello", b" world"])?;
/// ```
#[inline]
pub fn sha256(inputs: &[&[u8]]) -> Result<Sha256Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }
    #[cfg(target_os = "solana")]
    {
        let mut result = MaybeUninit::<Sha256Hash>::uninit();
        hash_syscall!(sol_sha256, inputs, result);
        // SAFETY: the syscall wrote all 32 bytes (see `hash_syscall!`).
        Ok(unsafe { result.assume_init() })
    }
    #[cfg(not(target_os = "solana"))]
    {
        let mut hasher = crate::sha256::ConstSha256::new();
        let mut i = 0;
        while i < inputs.len() {
            hasher = hasher.update(inputs[i]);
            i += 1;
        }
        Ok(hasher.finalize())
    }
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
/// Off-chain there is no keccak implementation in this crate, so the
/// result is all zeros; host tests that need the real digest should use a
/// software implementation.
#[inline]
pub fn keccak256(inputs: &[&[u8]]) -> Result<Keccak256Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }
    #[cfg(target_os = "solana")]
    {
        let mut result = MaybeUninit::<Keccak256Hash>::uninit();
        hash_syscall!(sol_keccak256, inputs, result);
        // SAFETY: the syscall wrote all 32 bytes (see `hash_syscall!`).
        Ok(unsafe { result.assume_init() })
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = inputs;
        Ok([0u8; 32])
    }
}

/// Compute Keccak-256 over a single byte slice.
#[inline]
pub fn keccak256_single(input: &[u8]) -> Result<Keccak256Hash, ProgramError> {
    keccak256(&[input])
}

/// Compute BLAKE3 over one or more byte slices.
///
/// Off-chain there is no BLAKE3 implementation in this crate, so the
/// result is all zeros.
#[inline]
pub fn blake3(inputs: &[&[u8]]) -> Result<Blake3Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }
    #[cfg(target_os = "solana")]
    {
        let mut result = MaybeUninit::<Blake3Hash>::uninit();
        hash_syscall!(sol_blake3, inputs, result);
        // SAFETY: the syscall wrote all 32 bytes (see `hash_syscall!`).
        Ok(unsafe { result.assume_init() })
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = inputs;
        Ok([0u8; 32])
    }
}

/// Compute BLAKE3 over a single byte slice.
#[inline]
pub fn blake3_single(input: &[u8]) -> Result<Blake3Hash, ProgramError> {
    blake3(&[input])
}

/// Compute SHA-512 over one or more byte slices through the `sol_sha512`
/// syscall (feature gate `s512oDwgx8hjMnaQjXfqqrZroVj4HvC6TkN3iSSWXCh`,
/// `enable_sha512_syscall`).
///
/// Bound only under the `sha512-syscall` cargo feature, because a program
/// that references the symbol fails to load on a cluster where the gate is
/// inactive (`Unresolved symbol`), and on 2026-09-27 the gate was active on
/// devnet and testnet but absent on mainnet-beta. Query the target cluster
/// (`hopper feature-gate`) before enabling it for a deployment. Off-chain
/// the result is all zeros.
#[cfg(feature = "sha512-syscall")]
#[inline]
pub fn sha512(inputs: &[&[u8]]) -> Result<Sha512Hash, ProgramError> {
    if inputs.len() > MAX_HASH_SEGMENTS {
        return Err(ProgramError::InvalidArgument);
    }
    #[cfg(target_os = "solana")]
    {
        let mut result = MaybeUninit::<Sha512Hash>::uninit();
        hash_syscall!(sol_sha512, inputs, result);
        // SAFETY: the syscall wrote all 64 bytes (see `hash_syscall!`).
        Ok(unsafe { result.assume_init() })
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = inputs;
        Ok([0u8; 64])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &[u8] = b"";

    #[test]
    fn sha256_off_chain_is_the_const_digest() {
        assert_eq!(
            sha256(&[b"abc"]).unwrap(),
            crate::sha256::sha256(b"abc"),
            "single segment"
        );
        assert_eq!(
            sha256(&[b"global:", b"initialize"]).unwrap(),
            crate::sha256::sha256(b"global:initialize"),
            "segments hash as one stream"
        );
        assert_eq!(sha256_single(b"").unwrap(), crate::sha256::sha256(b""));
    }

    static AT_LIMIT: [&[u8]; MAX_HASH_SEGMENTS] = [EMPTY; MAX_HASH_SEGMENTS];
    static PAST_LIMIT: [&[u8]; MAX_HASH_SEGMENTS + 1] = [EMPTY; MAX_HASH_SEGMENTS + 1];

    #[test]
    fn wrappers_accept_the_runtime_slice_limit() {
        assert_eq!(sha256(&AT_LIMIT[..]), Ok(crate::sha256::sha256(b"")));
        assert_eq!(keccak256(&AT_LIMIT[..]), Ok([0; 32]));
        assert_eq!(blake3(&AT_LIMIT[..]), Ok([0; 32]));
    }

    #[test]
    fn wrappers_refuse_beyond_the_runtime_slice_limit() {
        assert_eq!(sha256(&PAST_LIMIT[..]), Err(ProgramError::InvalidArgument));
        assert_eq!(
            keccak256(&PAST_LIMIT[..]),
            Err(ProgramError::InvalidArgument)
        );
        assert_eq!(blake3(&PAST_LIMIT[..]), Err(ProgramError::InvalidArgument));
    }
}
