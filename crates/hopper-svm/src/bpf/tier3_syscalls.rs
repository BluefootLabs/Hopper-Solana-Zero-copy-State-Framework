//! Tier 3 syscalls — the niche surface that real on-chain
//! programs occasionally reach for. Pure-Rust `do_*` layer; the
//! adapter layer in [`super::adapters`] calls these.
//!
//! ## Coverage
//!
//! **Introspection (fully implemented)**
//! - [`do_sol_get_stack_height`] — current CPI nesting depth
//!   (1 = outermost program, 2 = first CPI, …).
//! - [`do_sol_remaining_compute_units`] — current value of the CU
//!   meter. Programs use this to short-circuit work-heavy paths
//!   when they don't have budget left.
//! - [`do_sol_get_processed_sibling_instruction`] — pull a sibling
//!   instruction from the outer transaction by reverse index.
//!   Hopper Phase 2 doesn't yet maintain a per-transaction sibling
//!   ledger so the syscall always reports "no sibling at index"
//!   (the zero-return path mainnet uses for missing indices),
//!   matching upstream's behaviour for transactions whose sibling
//!   list is empty. Tests that don't depend on sibling
//!   introspection see this as a clean zero return.
//!
//! **Obsolete-but-referenced sysvars (stub buffers)**
//! - [`do_sol_get_slothashes_sysvar`] — empty SlotHashes vec.
//! - [`do_sol_get_slothistory_sysvar`] — empty SlotHistory bitvec.
//! - [`do_sol_get_stakehistory_sysvar`] — empty StakeHistory vec.
//!
//! Programs that *read* these for compute-time behaviour will see
//! "the cluster has no history" — a clean default state. Programs
//! that need real fixtures can write the buffers directly via the
//! Phase 1 account-passing path.
//!
//! **Generic accessor (fully implemented)**
//! - [`do_sol_get_sysvar`] — dispatches by 32-byte sysvar ID into
//!   the Hopper sysvar surface. Mirrors mainnet's
//!   `sol_get_sysvar(addr, offset, length)` accessor.
//!
//! **Curve25519 (fully implemented — uses existing `curve25519-dalek` dep)**
//! - [`do_sol_curve_validate_point`] — is the 32-byte point on the
//!   curve / in the prime-order subgroup? Supports Edwards and
//!   Ristretto.
//! - [`do_sol_curve_group_op`] — add / sub / mul on Edwards and
//!   Ristretto. Mainnet wire-compatible.
//!
//! **Heavy crypto — clear-error stubs (Tier 4 work)**
//! - [`do_sol_poseidon`] — Poseidon hash. Returns `Custom` with a
//!   "Hopper Phase 2 doesn't ship Poseidon" message. Programs that
//!   need Poseidon today can fall back to bundling
//!   `light-poseidon` directly into their on-chain code.
//! - [`do_sol_big_mod_exp`] — RSA-style modular exponentiation
//!   over arbitrary-precision integers. Same clear-error stub
//!   shape; pulls in `num-bigint` once Tier 4 lands.
//! - [`do_sol_alt_bn128_group_op`] — BN254 / alt_bn128 curve
//!   group ops. Clear-error stub; `ark-bn254` is the canonical
//!   backing crate for Tier 4.
//! - [`do_sol_alt_bn128_compression`] — BN254 point compression
//!   helpers. Clear-error stub.
//!
//! ## Why "clear-error stubs" instead of "unknown syscall"
//!
//! Mainnet programs that link a Poseidon syscall but never hit the
//! code path during testing should still load and dispatch
//! cleanly. By registering the syscall name with a stub that
//! returns a structured `Custom` error only when actually called,
//! programs using these niche operations behind feature flags or
//! fallback paths can run their happy-path tests through Hopper
//! without modification. The stub message names the feature
//! gap so the failing test surfaces the actionable next step.

use crate::bpf::context::BpfContext;
use crate::bpf::syscalls::SyscallResult;

/// Per-syscall CU costs — match mainnet defaults.
mod cu {
    /// Stack-height syscall — cheap (1 register read). Match
    /// mainnet's `syscall_base_cost = 100` baseline.
    pub const SOL_GET_STACK_HEIGHT: u64 = 100;
    /// Remaining-compute-units — also a 1-register read.
    pub const SOL_REMAINING_COMPUTE_UNITS: u64 = 100;
    /// Sibling-instruction lookup — base cost 100 + per-byte copy
    /// for the data buffer. Phase 1 uses the flat 100 since we
    /// always return zero-data.
    pub const SOL_GET_PROCESSED_SIBLING_INSTRUCTION: u64 = 100;
    /// Generic sol_get_sysvar — slightly higher than the dedicated
    /// per-sysvar syscalls because the dispatch table costs an
    /// extra ID match. Match mainnet's 100 baseline.
    pub const SOL_GET_SYSVAR: u64 = 100;
    /// Obsolete-sysvar fetches.
    pub const SOL_GET_SLOTHASHES_SYSVAR: u64 = 100;
    pub const SOL_GET_SLOTHISTORY_SYSVAR: u64 = 100;
    pub const SOL_GET_STAKEHISTORY_SYSVAR: u64 = 100;
    /// Curve25519 validate-point: 159 CU. Mainnet: `curve25519_edwards_validate_point_cost`.
    pub const SOL_CURVE_VALIDATE_POINT: u64 = 159;
    /// Curve25519 group-op: ~2_000 CU. The mainnet pricing is
    /// per-op (add/sub/mul each have their own line); we average
    /// to a single constant for simplicity.
    pub const SOL_CURVE_GROUP_OP: u64 = 2_000;
    /// Heavy-crypto stubs charge 1 CU before failing — keeps the
    /// meter accounting honest if a future Tier 4 release wires
    /// up a real impl behind the same name.
    pub const HEAVY_STUB: u64 = 1;
}

/// Charge `cost` CUs against the meter. Returns `OutOfMeter` when
/// the meter would underflow. Same idiom as the other syscall
/// modules.
fn charge(ctx: &mut BpfContext, cost: u64) -> Result<(), SyscallResult> {
    if ctx.remaining_units < cost {
        return Err(SyscallResult::OutOfMeter);
    }
    ctx.remaining_units -= cost;
    Ok(())
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

/// `sol_get_stack_height` — return the current CPI nesting depth.
/// Outermost program runs at depth 1; each `sol_invoke_signed_*`
/// increments. Mirrors mainnet's accessor exactly.
pub fn do_sol_get_stack_height(ctx: &mut BpfContext) -> Result<u64, SyscallResult> {
    if let Err(err) = charge(ctx, cu::SOL_GET_STACK_HEIGHT) {
        return Err(err);
    }
    Ok(ctx.cpi_depth as u64)
}

/// `sol_remaining_compute_units` — return the meter's current
/// value AFTER charging for this syscall (so a program reading
/// the value sees the post-charge count, matching mainnet).
pub fn do_sol_remaining_compute_units(ctx: &mut BpfContext) -> Result<u64, SyscallResult> {
    if let Err(err) = charge(ctx, cu::SOL_REMAINING_COMPUTE_UNITS) {
        return Err(err);
    }
    Ok(ctx.remaining_units)
}

/// `sol_get_processed_sibling_instruction` — pull metadata for a
/// sibling instruction at `index` (zero-based, in transaction
/// order, top-level only). Returns `1` (truthy) on success.
///
/// Phase 2 doesn't yet ledger sibling instructions, so the syscall
/// always reports "no sibling at index" — returning `0` to mirror
/// the upstream behaviour for empty lists. Programs that need real
/// sibling introspection are out of scope for Tier 3.
pub fn do_sol_get_processed_sibling_instruction(
    ctx: &mut BpfContext,
    _index: u64,
) -> Result<u64, SyscallResult> {
    if let Err(err) = charge(ctx, cu::SOL_GET_PROCESSED_SIBLING_INSTRUCTION) {
        return Err(err);
    }
    // 0 = "no sibling at this index". Matches upstream behaviour
    // for empty sibling lists.
    Ok(0)
}

// ---------------------------------------------------------------------------
// Generic sol_get_sysvar
// ---------------------------------------------------------------------------

/// `sol_get_sysvar` — generic sysvar accessor. Mainnet wire:
///
/// ```text
/// sol_get_sysvar(
///     sysvar_id_addr: u64,    // 32-byte ID
///     offset: u64,            // byte offset into the sysvar
///     out_addr: u64,
///     length: u64,            // bytes to copy
/// ) -> u64
/// ```
///
/// Returns `0` on success, non-zero on offset/length out of range
/// or unknown sysvar ID.
///
/// The pure-Rust layer accepts the resolved sysvar bytes (looked
/// up from the harness sysvar surface) and copies the requested
/// `[offset, offset+length)` slice into `out`. The adapter layer
/// resolves the ID and provides the slice.
pub fn do_sol_get_sysvar_copy(
    ctx: &mut BpfContext,
    src: &[u8],
    offset: usize,
    out: &mut [u8],
) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_SYSVAR) {
        return err;
    }
    let end = offset.saturating_add(out.len());
    if end > src.len() {
        return SyscallResult::Custom(format!(
            "sol_get_sysvar: out-of-range request offset={offset} length={} > sysvar size {}",
            out.len(),
            src.len()
        ));
    }
    out.copy_from_slice(&src[offset..end]);
    SyscallResult::Ok
}

// ---------------------------------------------------------------------------
// Obsolete sysvars
// ---------------------------------------------------------------------------

/// `sol_get_slothashes_sysvar` — write SlotHashes into `out`. Phase
/// 2 maintains an empty SlotHashes (a 12-slot bincode-shaped vec
/// with `len=0`), since most Hopper tests don't simulate slot
/// progression deeply enough for the entries to matter. Programs
/// that iterate on the entries see "no recent slots".
///
/// Wire format: `len(u64 LE) + n × (slot u64 LE | hash 32 bytes)`.
/// The empty-list serialisation is just `[0u8; 8]`, which fits in
/// any buffer ≥ 8 bytes.
pub const SLOTHASHES_EMPTY_LEN: usize = 8;

pub fn do_sol_get_slothashes_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_SLOTHASHES_SYSVAR) {
        return err;
    }
    if out.len() < SLOTHASHES_EMPTY_LEN {
        return SyscallResult::Custom(format!(
            "sol_get_slothashes_sysvar: buffer {} bytes < {} required for empty list",
            out.len(),
            SLOTHASHES_EMPTY_LEN
        ));
    }
    // Write length 0; rest stays whatever the caller put there.
    out[..8].copy_from_slice(&0u64.to_le_bytes());
    SyscallResult::Ok
}

/// `sol_get_slothistory_sysvar` — SlotHistory is a bitvec
/// (~131,072 bits = 16,384 bytes per slot history) tracking
/// whether each of the most-recent N slots was rooted. Hopper's
/// stub writes the canonical "all-zero" bitvec into the buffer,
/// matching a fresh validator with no observed slots.
pub const SLOTHISTORY_EMPTY_LEN: usize = 16_392; // 16,384 bitvec + 8 next_slot

pub fn do_sol_get_slothistory_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_SLOTHISTORY_SYSVAR) {
        return err;
    }
    if out.len() < SLOTHISTORY_EMPTY_LEN {
        return SyscallResult::Custom(format!(
            "sol_get_slothistory_sysvar: buffer {} bytes < {} required",
            out.len(),
            SLOTHISTORY_EMPTY_LEN
        ));
    }
    // All-zero bitvec; next_slot = 0.
    for b in out[..SLOTHISTORY_EMPTY_LEN].iter_mut() {
        *b = 0;
    }
    SyscallResult::Ok
}

/// `sol_get_stakehistory_sysvar` — StakeHistory is a vec of
/// `(epoch, StakeHistoryEntry)` pairs covering the most recent
/// 512 epochs. Hopper's stub writes the empty-vec encoding.
pub const STAKEHISTORY_EMPTY_LEN: usize = 8;

pub fn do_sol_get_stakehistory_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_STAKEHISTORY_SYSVAR) {
        return err;
    }
    if out.len() < STAKEHISTORY_EMPTY_LEN {
        return SyscallResult::Custom(format!(
            "sol_get_stakehistory_sysvar: buffer {} bytes < {} required for empty list",
            out.len(),
            STAKEHISTORY_EMPTY_LEN
        ));
    }
    out[..8].copy_from_slice(&0u64.to_le_bytes());
    SyscallResult::Ok
}

// ---------------------------------------------------------------------------
// Curve25519
// ---------------------------------------------------------------------------

/// Curve identifier — matches mainnet's `CURVE25519_EDWARDS = 0`,
/// `CURVE25519_RISTRETTO = 1`.
pub const CURVE25519_EDWARDS: u64 = 0;
pub const CURVE25519_RISTRETTO: u64 = 1;

/// Group operation tag — matches mainnet's `ADD = 0`, `SUB = 1`,
/// `MUL = 2`.
pub const GROUP_OP_ADD: u64 = 0;
pub const GROUP_OP_SUB: u64 = 1;
pub const GROUP_OP_MUL: u64 = 2;

/// `sol_curve_validate_point` — is `point` a valid 32-byte point
/// on the requested curve? Returns `0` for valid, `1` for invalid.
/// Matches the mainnet wire convention (zero on success).
pub fn do_sol_curve_validate_point(
    ctx: &mut BpfContext,
    curve: u64,
    point: &[u8; 32],
) -> Result<u64, SyscallResult> {
    if let Err(err) = charge(ctx, cu::SOL_CURVE_VALIDATE_POINT) {
        return Err(err);
    }
    let valid = match curve {
        CURVE25519_EDWARDS => {
            use curve25519_dalek::edwards::CompressedEdwardsY;
            CompressedEdwardsY(*point)
                .decompress()
                .map(|p| p.is_torsion_free())
                .unwrap_or(false)
        }
        CURVE25519_RISTRETTO => {
            use curve25519_dalek::ristretto::CompressedRistretto;
            CompressedRistretto(*point).decompress().is_some()
        }
        other => {
            return Err(SyscallResult::Custom(format!(
                "sol_curve_validate_point: unknown curve {other} (supported: \
                 0/Edwards, 1/Ristretto)"
            )));
        }
    };
    // Mainnet returns 0 = valid, 1 = invalid (bool inverted).
    Ok(if valid { 0 } else { 1 })
}

/// `sol_curve_group_op` — add / sub / mul on Edwards or Ristretto.
/// Writes the 32-byte result into `out`. Returns `0` on success,
/// `1` on invalid input (off-curve operand or unknown op).
pub fn do_sol_curve_group_op(
    ctx: &mut BpfContext,
    curve: u64,
    op: u64,
    a: &[u8; 32],
    b: &[u8; 32],
    out: &mut [u8; 32],
) -> Result<u64, SyscallResult> {
    if let Err(err) = charge(ctx, cu::SOL_CURVE_GROUP_OP) {
        return Err(err);
    }
    match curve {
        CURVE25519_EDWARDS => edwards_group_op(op, a, b, out),
        CURVE25519_RISTRETTO => ristretto_group_op(op, a, b, out),
        other => Err(SyscallResult::Custom(format!(
            "sol_curve_group_op: unknown curve {other} (supported: \
             0/Edwards, 1/Ristretto)"
        ))),
    }
}

/// Edwards-curve group ops. Operand A is always a curve point.
/// For ADD/SUB, B is also a point. For MUL, B is a 32-byte scalar
/// (little-endian).
fn edwards_group_op(
    op: u64,
    a: &[u8; 32],
    b: &[u8; 32],
    out: &mut [u8; 32],
) -> Result<u64, SyscallResult> {
    use curve25519_dalek::edwards::CompressedEdwardsY;
    use curve25519_dalek::scalar::Scalar;
    let pa = match CompressedEdwardsY(*a).decompress() {
        Some(p) => p,
        None => return Ok(1),
    };
    let result = match op {
        GROUP_OP_ADD => {
            let pb = match CompressedEdwardsY(*b).decompress() {
                Some(p) => p,
                None => return Ok(1),
            };
            (pa + pb).compress()
        }
        GROUP_OP_SUB => {
            let pb = match CompressedEdwardsY(*b).decompress() {
                Some(p) => p,
                None => return Ok(1),
            };
            (pa - pb).compress()
        }
        GROUP_OP_MUL => {
            let scalar = match Scalar::from_canonical_bytes(*b).into_option() {
                Some(s) => s,
                None => return Ok(1),
            };
            (pa * scalar).compress()
        }
        other => {
            return Err(SyscallResult::Custom(format!(
                "sol_curve_group_op: unknown op {other} (supported: 0/Add, 1/Sub, 2/Mul)"
            )));
        }
    };
    out.copy_from_slice(result.as_bytes());
    Ok(0)
}

/// Ristretto group ops. Same shape as edwards but the operands +
/// scalar live in the Ristretto encoding.
fn ristretto_group_op(
    op: u64,
    a: &[u8; 32],
    b: &[u8; 32],
    out: &mut [u8; 32],
) -> Result<u64, SyscallResult> {
    use curve25519_dalek::ristretto::CompressedRistretto;
    use curve25519_dalek::scalar::Scalar;
    let pa = match CompressedRistretto(*a).decompress() {
        Some(p) => p,
        None => return Ok(1),
    };
    let result = match op {
        GROUP_OP_ADD => {
            let pb = match CompressedRistretto(*b).decompress() {
                Some(p) => p,
                None => return Ok(1),
            };
            (pa + pb).compress()
        }
        GROUP_OP_SUB => {
            let pb = match CompressedRistretto(*b).decompress() {
                Some(p) => p,
                None => return Ok(1),
            };
            (pa - pb).compress()
        }
        GROUP_OP_MUL => {
            let scalar = match Scalar::from_canonical_bytes(*b).into_option() {
                Some(s) => s,
                None => return Ok(1),
            };
            (pa * scalar).compress()
        }
        other => {
            return Err(SyscallResult::Custom(format!(
                "sol_curve_group_op: unknown op {other} (supported: 0/Add, 1/Sub, 2/Mul)"
            )));
        }
    };
    out.copy_from_slice(result.as_bytes());
    Ok(0)
}

// ---------------------------------------------------------------------------
// Heavy crypto stubs — Tier 4 work
// ---------------------------------------------------------------------------

/// Common formatter for stub-error messages so the wording stays
/// consistent.
fn stub_msg(name: &str) -> SyscallResult {
    SyscallResult::Custom(format!(
        "{name}: not yet implemented in Hopper Phase 2 — \
         this syscall lands in Tier 4 alongside the heavy-crypto \
         deps (light-poseidon / num-bigint / ark-bn254). \
         Programs hitting this path can either bundle the \
         primitive directly or open a Hopper feature request."
    ))
}

/// `sol_poseidon` — Poseidon hash. Stub.
pub fn do_sol_poseidon(ctx: &mut BpfContext) -> SyscallResult {
    let _ = charge(ctx, cu::HEAVY_STUB);
    stub_msg("sol_poseidon")
}

/// `sol_big_mod_exp` — big-integer modular exponentiation. Stub.
pub fn do_sol_big_mod_exp(ctx: &mut BpfContext) -> SyscallResult {
    let _ = charge(ctx, cu::HEAVY_STUB);
    stub_msg("sol_big_mod_exp")
}

/// `sol_alt_bn128_group_op` — BN254 curve group ops. Stub.
pub fn do_sol_alt_bn128_group_op(ctx: &mut BpfContext) -> SyscallResult {
    let _ = charge(ctx, cu::HEAVY_STUB);
    stub_msg("sol_alt_bn128_group_op")
}

/// `sol_alt_bn128_compression` — BN254 point compression. Stub.
pub fn do_sol_alt_bn128_compression(ctx: &mut BpfContext) -> SyscallResult {
    let _ = charge(ctx, cu::HEAVY_STUB);
    stub_msg("sol_alt_bn128_compression")
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    fn ctx_with_units(units: u64) -> BpfContext {
        BpfContext::new(Pubkey::new_unique(), units)
    }

    #[test]
    fn stack_height_returns_cpi_depth() {
        let mut ctx = ctx_with_units(1_000);
        ctx.cpi_depth = 3;
        let h = do_sol_get_stack_height(&mut ctx).unwrap();
        assert_eq!(h, 3);
        assert_eq!(ctx.remaining_units, 900);
    }

    #[test]
    fn remaining_units_post_charge() {
        let mut ctx = ctx_with_units(1_000);
        let r = do_sol_remaining_compute_units(&mut ctx).unwrap();
        // Read AFTER the charge: 1000 - 100 = 900.
        assert_eq!(r, 900);
    }

    #[test]
    fn sibling_instruction_returns_zero() {
        let mut ctx = ctx_with_units(1_000);
        let r = do_sol_get_processed_sibling_instruction(&mut ctx, 5).unwrap();
        assert_eq!(r, 0);
    }

    #[test]
    fn slothashes_writes_empty_list() {
        let mut ctx = ctx_with_units(1_000);
        let mut out = [0xFFu8; 8];
        let r = do_sol_get_slothashes_sysvar(&mut ctx, &mut out);
        assert_eq!(r, SyscallResult::Ok);
        // First 8 bytes are the LE u64 length = 0.
        assert_eq!(u64::from_le_bytes(out), 0);
    }

    #[test]
    fn slothashes_short_buffer_errors() {
        let mut ctx = ctx_with_units(1_000);
        let mut out = [0u8; 4];
        let r = do_sol_get_slothashes_sysvar(&mut ctx, &mut out);
        assert!(matches!(r, SyscallResult::Custom(_)));
    }

    #[test]
    fn get_sysvar_copies_slice() {
        let mut ctx = ctx_with_units(1_000);
        let src = b"abcdefghij";
        let mut out = [0u8; 4];
        let r = do_sol_get_sysvar_copy(&mut ctx, src, 3, &mut out);
        assert_eq!(r, SyscallResult::Ok);
        assert_eq!(&out, b"defg");
    }

    #[test]
    fn get_sysvar_out_of_range_errors() {
        let mut ctx = ctx_with_units(1_000);
        let src = b"short";
        let mut out = [0u8; 4];
        // offset 3 + length 4 = 7, beyond len 5 = error
        let r = do_sol_get_sysvar_copy(&mut ctx, src, 3, &mut out);
        assert!(matches!(r, SyscallResult::Custom(_)));
    }

    #[test]
    fn curve_validate_unknown_curve_errors() {
        let mut ctx = ctx_with_units(1_000);
        let r = do_sol_curve_validate_point(&mut ctx, 99, &[0u8; 32]);
        assert!(matches!(r, Err(SyscallResult::Custom(_))));
    }

    #[test]
    fn curve_validate_off_curve_returns_one() {
        let mut ctx = ctx_with_units(1_000);
        // 0xFF…FF is not a valid Edwards Y coordinate.
        let r = do_sol_curve_validate_point(&mut ctx, CURVE25519_EDWARDS, &[0xFFu8; 32]).unwrap();
        assert_eq!(r, 1); // 1 = invalid
    }

    #[test]
    fn curve_validate_basepoint_is_valid() {
        let mut ctx = ctx_with_units(1_000);
        // Edwards basepoint compressed encoding (canonical
        // Curve25519 generator).
        use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
        let basepoint = ED25519_BASEPOINT_POINT.compress().to_bytes();
        let r = do_sol_curve_validate_point(&mut ctx, CURVE25519_EDWARDS, &basepoint).unwrap();
        assert_eq!(r, 0); // 0 = valid
    }

    #[test]
    fn curve_group_add_basepoint_to_self_round_trips() {
        let mut ctx = ctx_with_units(10_000);
        use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
        let basepoint = ED25519_BASEPOINT_POINT.compress().to_bytes();
        let mut out = [0u8; 32];
        let r = do_sol_curve_group_op(
            &mut ctx,
            CURVE25519_EDWARDS,
            GROUP_OP_ADD,
            &basepoint,
            &basepoint,
            &mut out,
        )
        .unwrap();
        assert_eq!(r, 0);
        // Adding the basepoint to itself is the same as scalar-mul
        // by 2 — pin against that.
        let expected = (ED25519_BASEPOINT_POINT + ED25519_BASEPOINT_POINT)
            .compress()
            .to_bytes();
        assert_eq!(out, expected);
    }

    #[test]
    fn curve_group_unknown_op_errors() {
        let mut ctx = ctx_with_units(10_000);
        let mut out = [0u8; 32];
        let r = do_sol_curve_group_op(
            &mut ctx,
            CURVE25519_EDWARDS,
            99,
            &[0u8; 32],
            &[0u8; 32],
            &mut out,
        );
        assert!(matches!(r, Err(SyscallResult::Custom(_))));
    }

    #[test]
    fn poseidon_stub_returns_actionable_error() {
        let mut ctx = ctx_with_units(1_000);
        let r = do_sol_poseidon(&mut ctx);
        match r {
            SyscallResult::Custom(msg) => {
                assert!(msg.contains("Tier 4"), "{msg}");
                assert!(msg.contains("light-poseidon"), "{msg}");
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn alt_bn128_stubs_return_actionable_errors() {
        let mut ctx = ctx_with_units(1_000);
        match do_sol_alt_bn128_group_op(&mut ctx) {
            SyscallResult::Custom(msg) => assert!(msg.contains("ark-bn254")),
            _ => panic!("expected Custom"),
        }
        match do_sol_alt_bn128_compression(&mut ctx) {
            SyscallResult::Custom(msg) => assert!(msg.contains("ark-bn254")),
            _ => panic!("expected Custom"),
        }
    }

    #[test]
    fn out_of_meter_short_circuits() {
        let mut ctx = ctx_with_units(50);
        let r = do_sol_get_stack_height(&mut ctx);
        assert!(matches!(r, Err(SyscallResult::OutOfMeter)));
        // Meter not partially debited.
        assert_eq!(ctx.remaining_units, 50);
    }
}
