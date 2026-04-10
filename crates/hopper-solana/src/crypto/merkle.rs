//! Zero-alloc Merkle proof verification via `sol_sha256`.
//!
//! Verify inclusion proofs against a known root. Pass the root, leaf,
//! and proof slices. Stack only, uses the BPF `sol_sha256` syscall for
//! each hash step.
//!
//! ```rust,ignore
//! let leaf_hash = sha256_leaf(&user_data);
//! verify_merkle_proof(&root, &leaf_hash, &proof_hashes)?;
//! ```

use hopper_runtime::error::ProgramError;

/// Verify a Merkle proof against a known root.
///
/// `root` is the expected 32-byte tree root.
/// `leaf` is the 32-byte hash of the leaf data.
/// `proof` is a slice of 32-byte sibling hashes, from leaf to root.
///
/// Standard Solana convention: at each level, the smaller hash goes
/// first (sorted pair hashing). Matches OpenZeppelin / SPL convention.
///
/// ```rust,ignore
/// verify_merkle_proof(&expected_root, &leaf_hash, &[proof_0, proof_1, proof_2])?;
/// ```
#[inline]
pub fn verify_merkle_proof(
    root: &[u8; 32],
    leaf: &[u8; 32],
    proof: &[[u8; 32]],
) -> Result<(), ProgramError> {
    let mut computed = *leaf;

    for sibling in proof {
        computed = hash_sorted_pair(&computed, sibling);
    }

    if computed != *root {
        return Err(ProgramError::InvalidArgument);
    }

    Ok(())
}

/// Hash a single leaf value using SHA-256.
///
/// Prepends a `0x00` byte before the data (leaf domain separator) to
/// prevent second-preimage attacks.
#[inline(always)]
pub fn sha256_leaf(data: &[u8]) -> [u8; 32] {
    let prefix = [0x00u8];
    sha256_two_slices(&prefix, data)
}

/// Hash two 32-byte values in sorted order via `sol_sha256`.
///
/// Sorts the pair so the lexicographically smaller hash is first.
#[inline(always)]
fn hash_sorted_pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let prefix = [0x01u8];
    if a <= b {
        sha256_three_slices(&prefix, a, b)
    } else {
        sha256_three_slices(&prefix, b, a)
    }
}

/// SHA-256 of two byte slices concatenated, via the BPF syscall.
#[inline(always)]
fn sha256_two_slices(a: &[u8], b: &[u8]) -> [u8; 32] {
    #[cfg(target_os = "solana")]
    {
        use core::mem::MaybeUninit;
        let slices: [&[u8]; 2] = [a, b];
        let mut result = MaybeUninit::<[u8; 32]>::uninit();
        // SAFETY: sol_sha256 writes exactly 32 bytes into the output buffer.
        // Slice pointers are valid for the duration of the call.
        unsafe {
            hopper_runtime::syscalls::sol_sha256(
                slices.as_ptr() as *const u8,
                2u64,
                result.as_mut_ptr() as *mut u8,
            );
            result.assume_init()
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (a, b);
        unreachable!("sol_sha256 is only available on target `solana`");
    }
}

/// SHA-256 of three byte slices concatenated, via the BPF syscall.
#[inline(always)]
fn sha256_three_slices(a: &[u8], b: &[u8], c: &[u8]) -> [u8; 32] {
    #[cfg(target_os = "solana")]
    {
        use core::mem::MaybeUninit;
        let slices: [&[u8]; 3] = [a, b, c];
        let mut result = MaybeUninit::<[u8; 32]>::uninit();
        // SAFETY: sol_sha256 writes exactly 32 bytes into the output buffer.
        // Slice pointers are valid for the duration of the call.
        unsafe {
            hopper_runtime::syscalls::sol_sha256(
                slices.as_ptr() as *const u8,
                3u64,
                result.as_mut_ptr() as *mut u8,
            );
            result.assume_init()
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (a, b, c);
        unreachable!("sol_sha256 is only available on target `solana`");
    }
}
