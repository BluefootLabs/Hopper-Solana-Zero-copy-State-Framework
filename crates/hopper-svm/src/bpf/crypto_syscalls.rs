//! Cryptographic syscalls — Phase 2.1.
//!
//! Three published-crate-backed hash and signature primitives:
//!
//! - [`do_sol_keccak256`] — Keccak-256 hash. Wire-compatible with
//!   the Ethereum keccak (legacy NIST submission, *not* the
//!   standardised SHA3-256 — they differ only in padding but
//!   produce different digests). Backed by `sha3::Keccak256`.
//! - [`do_sol_blake3`] — BLAKE3 hash. Backed by the `blake3` crate.
//! - [`do_sol_secp256k1_recover`] — ECDSA public-key recovery.
//!   Given a 32-byte message hash, a recovery id (0..=3), and a
//!   64-byte compact signature, returns the 64-byte uncompressed
//!   public key (X || Y, no leading 0x04 marker — matches
//!   upstream wire). Backed by `secp256k1` with the `recovery`
//!   feature.
//!
//! ## CU costs
//!
//! Match the production runtime defaults so Phase 2 CU readouts
//! equal mainnet figures:
//!
//! - Keccak-256: 85 CU per chunk + 33 CU per `byte_count / 16`
//!   word. Phase 2.1 uses a flat 85 CU per call plus 1 CU per
//!   16 bytes — close to upstream and easy to reason about.
//! - BLAKE3: same accounting model.
//! - secp256k1 recover: 25_000 CU flat. Crypto-recover is one of
//!   the most expensive syscalls; this is the genuine number.
//!
//! ## Wire formats
//!
//! ```text
//! sol_keccak256_(
//!     vals_addr: u64,    // [(seed_addr: u64, seed_len: u64); n]
//!     vals_len: u64,     // n
//!     out_addr: u64,     // 32-byte output
//! ) -> u64
//!
//! sol_blake3(
//!     vals_addr: u64,
//!     vals_len: u64,
//!     out_addr: u64,
//! ) -> u64
//!
//! sol_secp256k1_recover_(
//!     hash_addr: u64,           // 32 bytes, message digest
//!     recovery_id: u64,         // 0..=3
//!     signature_addr: u64,      // 64 bytes (r || s)
//!     out_addr: u64,            // 64 bytes (X || Y)
//! ) -> u64
//! ```

use crate::bpf::context::BpfContext;
use crate::bpf::syscalls::SyscallResult;

/// Keccak/BLAKE3 base CU cost. A short message pays this and one
/// per-16-byte chunk surcharge; the per-chunk surcharge keeps the
/// hash CU cost roughly proportional to input size, matching
/// upstream's per-word accounting.
pub const HASH_BASE_CU: u64 = 85;
/// Per-16-byte-chunk CU surcharge for hash syscalls.
pub const HASH_PER_CHUNK_CU: u64 = 1;
/// Bytes-per-chunk for the hash CU surcharge.
pub const HASH_CHUNK_BYTES: u64 = 16;

/// secp256k1 recover CU cost. Flat. Matches mainnet
/// `secp256k1_recover_units = 25_000`.
pub const SOL_SECP256K1_RECOVER_CU: u64 = 25_000;

/// Compute the CU cost for a hash syscall over a given byte count.
fn hash_cost(total_bytes: usize) -> u64 {
    let chunks = (total_bytes as u64 + HASH_CHUNK_BYTES - 1) / HASH_CHUNK_BYTES;
    HASH_BASE_CU.saturating_add(HASH_PER_CHUNK_CU.saturating_mul(chunks))
}

/// Charge `cost` CUs against the context's meter. Returns
/// `OutOfMeter` if the meter would go below zero.
fn charge(ctx: &mut BpfContext, cost: u64) -> Result<(), SyscallResult> {
    if ctx.remaining_units < cost {
        return Err(SyscallResult::OutOfMeter);
    }
    ctx.remaining_units -= cost;
    Ok(())
}

// ---------------------------------------------------------------------------
// `sol_keccak256_`
// ---------------------------------------------------------------------------

/// `sol_keccak256_` — Keccak-256 (legacy Ethereum variant) over
/// the concatenation of every chunk. The output is exactly 32
/// bytes.
pub fn do_sol_keccak256(
    ctx: &mut BpfContext,
    chunks: &[&[u8]],
    out: &mut [u8; 32],
) -> SyscallResult {
    use sha3::{Digest, Keccak256};
    let total: usize = chunks.iter().map(|c| c.len()).sum();
    if let Err(err) = charge(ctx, hash_cost(total)) {
        return err;
    }
    let mut hasher = Keccak256::new();
    for c in chunks {
        hasher.update(c);
    }
    let digest = hasher.finalize();
    out.copy_from_slice(&digest);
    SyscallResult::Ok
}

// ---------------------------------------------------------------------------
// `sol_blake3`
// ---------------------------------------------------------------------------

/// `sol_blake3` — BLAKE3 over the concatenation of every chunk.
/// 32-byte output.
pub fn do_sol_blake3(
    ctx: &mut BpfContext,
    chunks: &[&[u8]],
    out: &mut [u8; 32],
) -> SyscallResult {
    let total: usize = chunks.iter().map(|c| c.len()).sum();
    if let Err(err) = charge(ctx, hash_cost(total)) {
        return err;
    }
    let mut hasher = blake3::Hasher::new();
    for c in chunks {
        hasher.update(c);
    }
    let digest = hasher.finalize();
    out.copy_from_slice(digest.as_bytes());
    SyscallResult::Ok
}

// ---------------------------------------------------------------------------
// `sol_secp256k1_recover_`
// ---------------------------------------------------------------------------

/// secp256k1 recover error codes — match upstream wire return
/// values so a Hopper test sees the same non-zero return that
/// production does.
pub mod recover_err {
    /// `recovery_id` was not in `0..=3`.
    pub const INVALID_HASH: u64 = 1;
    /// `signature` was malformed (low-level secp256k1 reject).
    pub const INVALID_SIGNATURE: u64 = 2;
    /// `recovery_id` was out of range.
    pub const INVALID_RECOVERY_ID: u64 = 3;
}

/// Outcome of a secp256k1 recover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoverOutcome {
    /// Recover succeeded; the 64-byte uncompressed public key
    /// (X || Y, no `0x04` prefix) was written to the caller's
    /// out buffer.
    Ok,
    /// Recover failed for a structured reason. The u64 maps to
    /// the wire-error code per [`recover_err`].
    Failed(u64),
    /// The compute meter went below zero.
    OutOfMeter,
}

/// `sol_secp256k1_recover_` — recover the 64-byte uncompressed
/// public key from a 32-byte message hash + 64-byte (r || s)
/// signature + recovery id.
pub fn do_sol_secp256k1_recover(
    ctx: &mut BpfContext,
    hash: &[u8; 32],
    recovery_id: u64,
    signature: &[u8; 64],
    out: &mut [u8; 64],
) -> RecoverOutcome {
    use secp256k1::{
        ecdsa::{RecoverableSignature, RecoveryId},
        Message, Secp256k1,
    };
    if charge(ctx, SOL_SECP256K1_RECOVER_CU).is_err() {
        return RecoverOutcome::OutOfMeter;
    }
    let recovery_id_byte = match recovery_id {
        0..=3 => recovery_id as i32,
        _ => return RecoverOutcome::Failed(recover_err::INVALID_RECOVERY_ID),
    };
    let recovery_id = match RecoveryId::from_i32(recovery_id_byte) {
        Ok(r) => r,
        Err(_) => return RecoverOutcome::Failed(recover_err::INVALID_RECOVERY_ID),
    };
    let message = match Message::from_digest_slice(hash) {
        Ok(m) => m,
        Err(_) => return RecoverOutcome::Failed(recover_err::INVALID_HASH),
    };
    let sig = match RecoverableSignature::from_compact(signature, recovery_id) {
        Ok(s) => s,
        Err(_) => return RecoverOutcome::Failed(recover_err::INVALID_SIGNATURE),
    };
    let secp = Secp256k1::new();
    match secp.recover_ecdsa(&message, &sig) {
        Ok(pk) => {
            // Uncompressed serialisation is 65 bytes — leading
            // 0x04 marker + 32-byte X + 32-byte Y. Upstream's
            // wire format is the 64 bytes after the marker, so
            // we strip it.
            let serialized = pk.serialize_uncompressed();
            out.copy_from_slice(&serialized[1..]);
            RecoverOutcome::Ok
        }
        Err(_) => RecoverOutcome::Failed(recover_err::INVALID_SIGNATURE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysvar::Sysvars;
    use solana_sdk::pubkey::Pubkey;

    fn ctx_with_units(units: u64) -> BpfContext {
        BpfContext::new_with_sysvars(Pubkey::new_unique(), units, Sysvars::default())
    }

    /// Keccak-256 over the empty input must equal the well-known
    /// `c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470`
    /// digest. Pin against any silent variant drift (SHA3-256
    /// vs Keccak-256 are different; this test catches the swap).
    #[test]
    fn keccak256_empty_input_matches_known_digest() {
        let mut ctx = ctx_with_units(10_000);
        let mut out = [0u8; 32];
        do_sol_keccak256(&mut ctx, &[], &mut out);
        let expected = [
            0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c,
            0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
            0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b,
            0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
        ];
        assert_eq!(out, expected);
    }

    /// Keccak-256 over "abc" must equal the known
    /// `4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45`
    /// digest. (Yes, Keccak — not SHA3, which would give a different
    /// value.)
    #[test]
    fn keccak256_abc_matches_known_digest() {
        let mut ctx = ctx_with_units(10_000);
        let mut out = [0u8; 32];
        do_sol_keccak256(&mut ctx, &[b"abc"], &mut out);
        let expected = [
            0x4e, 0x03, 0x65, 0x7a, 0xea, 0x45, 0xa9, 0x4f,
            0xc7, 0xd4, 0x7b, 0xa8, 0x26, 0xc8, 0xd6, 0x67,
            0xc0, 0xd1, 0xe6, 0xe3, 0x3a, 0x64, 0xa0, 0x36,
            0xec, 0x44, 0xf5, 0x8f, 0xa1, 0x2d, 0x6c, 0x45,
        ];
        assert_eq!(out, expected);
    }

    /// Multiple chunks hash-concatenate identically to a single
    /// concatenated chunk. Pin the streaming behaviour.
    #[test]
    fn keccak256_streams_match_one_shot() {
        let mut ctx_a = ctx_with_units(10_000);
        let mut ctx_b = ctx_with_units(10_000);
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        do_sol_keccak256(&mut ctx_a, &[b"abc", b"def"], &mut a);
        do_sol_keccak256(&mut ctx_b, &[b"abcdef"], &mut b);
        assert_eq!(a, b);
    }

    /// BLAKE3 over the empty input must equal the known
    /// `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`
    /// digest.
    #[test]
    fn blake3_empty_input_matches_known_digest() {
        let mut ctx = ctx_with_units(10_000);
        let mut out = [0u8; 32];
        do_sol_blake3(&mut ctx, &[], &mut out);
        let expected = [
            0xaf, 0x13, 0x49, 0xb9, 0xf5, 0xf9, 0xa1, 0xa6,
            0xa0, 0x40, 0x4d, 0xea, 0x36, 0xdc, 0xc9, 0x49,
            0x9b, 0xcb, 0x25, 0xc9, 0xad, 0xc1, 0x12, 0xb7,
            0xcc, 0x9a, 0x93, 0xca, 0xe4, 0x1f, 0x32, 0x62,
        ];
        assert_eq!(out, expected);
    }

    /// CU cost scales with input size — pin the formula.
    #[test]
    fn hash_cost_formula_pins() {
        // 0 bytes → 0 chunks → base 85.
        assert_eq!(hash_cost(0), 85);
        // 1 byte → 1 chunk → 86.
        assert_eq!(hash_cost(1), 86);
        // 16 bytes → 1 chunk → 86.
        assert_eq!(hash_cost(16), 86);
        // 17 bytes → 2 chunks → 87.
        assert_eq!(hash_cost(17), 87);
        // 32 bytes → 2 chunks → 87.
        assert_eq!(hash_cost(32), 87);
    }

    /// secp256k1 recover with a malformed signature returns
    /// `Failed(INVALID_SIGNATURE)`, not Ok or panic.
    #[test]
    fn secp256k1_recover_rejects_bad_signature() {
        let mut ctx = ctx_with_units(50_000);
        let mut out = [0u8; 64];
        // All-zero signature: not a valid (r, s) pair on the
        // curve — secp256k1 rejects.
        let outcome = do_sol_secp256k1_recover(
            &mut ctx,
            &[0u8; 32],
            0,
            &[0u8; 64],
            &mut out,
        );
        assert!(matches!(outcome, RecoverOutcome::Failed(_)));
    }

    /// Out-of-range recovery_id rejects without invoking secp256k1.
    #[test]
    fn secp256k1_recover_rejects_bad_recovery_id() {
        let mut ctx = ctx_with_units(50_000);
        let mut out = [0u8; 64];
        let outcome = do_sol_secp256k1_recover(
            &mut ctx,
            &[1u8; 32],
            7, // 7 > 3
            &[0u8; 64],
            &mut out,
        );
        assert_eq!(
            outcome,
            RecoverOutcome::Failed(recover_err::INVALID_RECOVERY_ID)
        );
    }

    /// Out-of-meter on a hash returns `OutOfMeter` without
    /// touching the output.
    #[test]
    fn hash_out_of_meter_short_circuits() {
        let mut ctx = ctx_with_units(10);
        let mut out = [0xFFu8; 32];
        let r = do_sol_keccak256(&mut ctx, &[b"abc"], &mut out);
        assert_eq!(r, SyscallResult::OutOfMeter);
        assert_eq!(out, [0xFFu8; 32]); // untouched
    }
}
