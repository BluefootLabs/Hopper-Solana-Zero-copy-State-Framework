//! Sysvar-fetch syscalls — Phase 2.1.
//!
//! `sol_get_*_sysvar` writes a fixed-size struct from the harness's
//! [`crate::Sysvars`] state into a guest buffer. The buffer layout
//! matches `solana_sdk::sysvar::*`'s `#[repr(C)]` shape so a
//! program reading the buffer through
//! `*(out as *const Clock)` sees the same field offsets the
//! upstream runtime would write.
//!
//! ## Wire layouts (all little-endian, matches the runtime's
//! `solana_sdk::sysvar::*` `#[repr(C)]` structures)
//!
//! ```text
//! Clock (40 bytes)
//!   0..7    slot                      u64 LE
//!   8..15   epoch_start_timestamp     i64 LE
//!  16..23   epoch                     u64 LE
//!  24..31   leader_schedule_epoch     u64 LE
//!  32..39   unix_timestamp            i64 LE
//!
//! Rent (24 bytes — 17 used, 7 trailing pad)
//!   0..7    lamports_per_byte_year    u64 LE
//!   8..15   exemption_threshold       f64 LE
//!  16       burn_percent              u8
//!  17..23   padding                   zero
//!
//! EpochSchedule (40 bytes)
//!   0..7    slots_per_epoch           u64 LE
//!   8..15   leader_schedule_slot_offset u64 LE
//!  16       warmup                    u8 (0 or 1)
//!  17..23   padding                   zero
//!  24..31   first_normal_epoch        u64 LE
//!  32..39   first_normal_slot         u64 LE
//!
//! LastRestartSlot (8 bytes)
//!   0..7    last_restart_slot         u64 LE
//! ```
//!
//! Phase 2.1 ships the four most-used sysvars; `EpochRewards` and
//! the obsolete `Fees` / `RecentBlockhashes` / `SlotHashes` /
//! `SlotHistory` / `StakeHistory` sysvars are not in scope (most
//! Hopper programs don't read them; tests that need them go
//! through Phase 1's account-passing path until a future
//! release).

use crate::bpf::context::BpfContext;
use crate::bpf::syscalls::SyscallResult;

/// Wire size of each sysvar in bytes. Programs that pass an
/// undersized buffer get a `Custom` error rather than a partial
/// write — matches runtime behaviour.
pub const CLOCK_BYTES: usize = 40;
pub const RENT_BYTES: usize = 24;
pub const EPOCH_SCHEDULE_BYTES: usize = 40;
pub const LAST_RESTART_SLOT_BYTES: usize = 8;
/// EpochRewards layout (96 bytes):
///   0..7    distribution_starting_block_height u64 LE
///   8..15   num_partitions                     u64 LE
///  16..47   parent_blockhash                   [u8; 32]
///  48..63   total_points                       u128 LE
///  64..71   total_rewards                      u64 LE
///  72..79   distributed_rewards                u64 LE
///  80       active                             u8 (0 or 1)
///  81..95   padding                            zero
pub const EPOCH_REWARDS_BYTES: usize = 96;

/// Per-syscall CU costs. Match the production runtime defaults so
/// Phase 2 CU readouts equal mainnet figures.
mod cu {
    pub const SOL_GET_CLOCK_SYSVAR: u64 = 100;
    pub const SOL_GET_RENT_SYSVAR: u64 = 100;
    pub const SOL_GET_EPOCH_SCHEDULE_SYSVAR: u64 = 100;
    pub const SOL_GET_LAST_RESTART_SLOT_SYSVAR: u64 = 100;
    pub const SOL_GET_EPOCH_REWARDS_SYSVAR: u64 = 100;
}

/// Charge `cost` CUs against the context's meter. Returns
/// `OutOfMeter` if the meter would go below zero. Matches the
/// idiom in `bpf/syscalls.rs` so the two modules stay
/// architecturally consistent.
fn charge(ctx: &mut BpfContext, cost: u64) -> Result<(), SyscallResult> {
    if ctx.remaining_units < cost {
        return Err(SyscallResult::OutOfMeter);
    }
    ctx.remaining_units -= cost;
    Ok(())
}

// ---------------------------------------------------------------------------
// Sysvar syscalls
// ---------------------------------------------------------------------------

/// `sol_get_clock_sysvar` — write the current Clock state into
/// `out` (must be ≥ [`CLOCK_BYTES`]). Returns `Custom` on short
/// buffer, `OutOfMeter` on budget exhaustion, otherwise `Ok`.
pub fn do_sol_get_clock_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_CLOCK_SYSVAR) {
        return err;
    }
    if out.len() < CLOCK_BYTES {
        return SyscallResult::Custom(format!(
            "sol_get_clock_sysvar: out buffer {} bytes < required {CLOCK_BYTES}",
            out.len()
        ));
    }
    let c = &ctx.sysvars.clock;
    out[0..8].copy_from_slice(&c.slot.to_le_bytes());
    out[8..16].copy_from_slice(&c.epoch_start_timestamp.to_le_bytes());
    out[16..24].copy_from_slice(&c.epoch.to_le_bytes());
    out[24..32].copy_from_slice(&c.leader_schedule_epoch.to_le_bytes());
    out[32..40].copy_from_slice(&c.unix_timestamp.to_le_bytes());
    SyscallResult::Ok
}

/// `sol_get_rent_sysvar` — write the current Rent state.
pub fn do_sol_get_rent_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_RENT_SYSVAR) {
        return err;
    }
    if out.len() < RENT_BYTES {
        return SyscallResult::Custom(format!(
            "sol_get_rent_sysvar: out buffer {} bytes < required {RENT_BYTES}",
            out.len()
        ));
    }
    let r = &ctx.sysvars.rent;
    out[0..8].copy_from_slice(&r.lamports_per_byte_year.to_le_bytes());
    out[8..16].copy_from_slice(&r.exemption_threshold.to_le_bytes());
    out[16] = r.burn_percent;
    // Trailing 7 bytes of padding — zero-fill so a subsequent read
    // through the host's `Rent` struct doesn't pick up uninitialised
    // bits.
    for b in out[17..24].iter_mut() {
        *b = 0;
    }
    SyscallResult::Ok
}

/// `sol_get_epoch_schedule_sysvar` — write the current
/// EpochSchedule state.
pub fn do_sol_get_epoch_schedule_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_EPOCH_SCHEDULE_SYSVAR) {
        return err;
    }
    if out.len() < EPOCH_SCHEDULE_BYTES {
        return SyscallResult::Custom(format!(
            "sol_get_epoch_schedule_sysvar: out buffer {} bytes < required {EPOCH_SCHEDULE_BYTES}",
            out.len()
        ));
    }
    let e = &ctx.sysvars.epoch_schedule;
    out[0..8].copy_from_slice(&e.slots_per_epoch.to_le_bytes());
    out[8..16].copy_from_slice(&e.leader_schedule_slot_offset.to_le_bytes());
    out[16] = u8::from(e.warmup);
    for b in out[17..24].iter_mut() {
        *b = 0;
    }
    out[24..32].copy_from_slice(&e.first_normal_epoch.to_le_bytes());
    out[32..40].copy_from_slice(&e.first_normal_slot.to_le_bytes());
    SyscallResult::Ok
}

/// `sol_get_last_restart_slot_sysvar` — write the LastRestartSlot
/// state. The smallest sysvar (single u64).
pub fn do_sol_get_last_restart_slot_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_LAST_RESTART_SLOT_SYSVAR) {
        return err;
    }
    if out.len() < LAST_RESTART_SLOT_BYTES {
        return SyscallResult::Custom(format!(
            "sol_get_last_restart_slot_sysvar: out buffer {} bytes < required {LAST_RESTART_SLOT_BYTES}",
            out.len()
        ));
    }
    out[0..8].copy_from_slice(
        &ctx.sysvars
            .last_restart_slot
            .last_restart_slot
            .to_le_bytes(),
    );
    SyscallResult::Ok
}

/// `sol_get_epoch_rewards_sysvar` — write the EpochRewards state.
/// Wire format: 96 bytes per the upstream `#[repr(C)]` layout
/// (u64 + u64 + [u8;32] + u128 + u64 + u64 + bool + 15-byte pad).
pub fn do_sol_get_epoch_rewards_sysvar(ctx: &mut BpfContext, out: &mut [u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_GET_EPOCH_REWARDS_SYSVAR) {
        return err;
    }
    if out.len() < EPOCH_REWARDS_BYTES {
        return SyscallResult::Custom(format!(
            "sol_get_epoch_rewards_sysvar: out buffer {} bytes < required {EPOCH_REWARDS_BYTES}",
            out.len()
        ));
    }
    let r = &ctx.sysvars.epoch_rewards;
    out[0..8].copy_from_slice(&r.distribution_starting_block_height.to_le_bytes());
    out[8..16].copy_from_slice(&r.num_partitions.to_le_bytes());
    out[16..48].copy_from_slice(&r.parent_blockhash);
    out[48..64].copy_from_slice(&r.total_points.to_le_bytes());
    out[64..72].copy_from_slice(&r.total_rewards.to_le_bytes());
    out[72..80].copy_from_slice(&r.distributed_rewards.to_le_bytes());
    out[80] = u8::from(r.active);
    // Trailing 15-byte zero pad.
    for b in out[81..96].iter_mut() {
        *b = 0;
    }
    SyscallResult::Ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysvar::{Clock, EpochSchedule, LastRestartSlot, Rent, Sysvars};
    use solana_sdk::pubkey::Pubkey;

    fn ctx_with(units: u64, sysvars: Sysvars) -> BpfContext {
        BpfContext::new_with_sysvars(Pubkey::new_unique(), units, sysvars)
    }

    /// Clock layout: every field at the documented offset, every
    /// LE encoded. Pin against the wire format the production
    /// runtime emits.
    #[test]
    fn clock_sysvar_layout_is_canonical() {
        let mut sv = Sysvars::default();
        sv.clock = Clock {
            slot: 0x0102_0304_0506_0708,
            epoch_start_timestamp: 0x1112_1314_1516_1718,
            epoch: 0x2122_2324_2526_2728,
            leader_schedule_epoch: 0x3132_3334_3536_3738,
            unix_timestamp: 0x4142_4344_4546_4748,
        };
        let mut ctx = ctx_with(10_000, sv);
        let mut buf = [0u8; CLOCK_BYTES];
        let r = do_sol_get_clock_sysvar(&mut ctx, &mut buf);
        assert_eq!(r, SyscallResult::Ok);
        assert_eq!(
            u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            0x0102_0304_0506_0708
        );
        assert_eq!(
            i64::from_le_bytes(buf[8..16].try_into().unwrap()),
            0x1112_1314_1516_1718
        );
        assert_eq!(
            u64::from_le_bytes(buf[16..24].try_into().unwrap()),
            0x2122_2324_2526_2728
        );
        assert_eq!(
            u64::from_le_bytes(buf[24..32].try_into().unwrap()),
            0x3132_3334_3536_3738
        );
        assert_eq!(
            i64::from_le_bytes(buf[32..40].try_into().unwrap()),
            0x4142_4344_4546_4748
        );
    }

    /// Rent layout: u64 + f64 + u8 + 7 zero bytes pad. Pin the
    /// padding zeros so a future change to `Rent` can't silently
    /// leak garbage into the wire format.
    #[test]
    fn rent_sysvar_layout_zero_pads() {
        let mut sv = Sysvars::default();
        sv.rent = Rent {
            lamports_per_byte_year: 3480,
            exemption_threshold: 2.5,
            burn_percent: 50,
        };
        let mut ctx = ctx_with(10_000, sv);
        // Fill buffer with non-zero pre-write so we can check the
        // padding bytes were zeroed by the syscall, not pre-existing.
        let mut buf = [0xFFu8; RENT_BYTES];
        do_sol_get_rent_sysvar(&mut ctx, &mut buf);
        assert_eq!(u64::from_le_bytes(buf[0..8].try_into().unwrap()), 3480);
        assert_eq!(f64::from_le_bytes(buf[8..16].try_into().unwrap()), 2.5);
        assert_eq!(buf[16], 50);
        assert_eq!(&buf[17..24], &[0u8; 7]);
    }

    /// EpochSchedule layout: bool warmup at offset 16, 7 bytes of
    /// padding, two u64s at offsets 24 and 32.
    #[test]
    fn epoch_schedule_layout_padded_correctly() {
        let mut sv = Sysvars::default();
        sv.epoch_schedule = EpochSchedule {
            slots_per_epoch: 32,
            leader_schedule_slot_offset: 32,
            warmup: true,
            first_normal_epoch: 14,
            first_normal_slot: 32 * 14,
        };
        let mut ctx = ctx_with(10_000, sv);
        let mut buf = [0xFFu8; EPOCH_SCHEDULE_BYTES];
        do_sol_get_epoch_schedule_sysvar(&mut ctx, &mut buf);
        assert_eq!(u64::from_le_bytes(buf[0..8].try_into().unwrap()), 32);
        assert_eq!(u64::from_le_bytes(buf[8..16].try_into().unwrap()), 32);
        assert_eq!(buf[16], 1); // warmup = true
        assert_eq!(&buf[17..24], &[0u8; 7]);
        assert_eq!(u64::from_le_bytes(buf[24..32].try_into().unwrap()), 14);
        assert_eq!(u64::from_le_bytes(buf[32..40].try_into().unwrap()), 32 * 14);
    }

    /// LastRestartSlot is a single u64.
    #[test]
    fn last_restart_slot_layout_is_single_u64() {
        let mut sv = Sysvars::default();
        sv.last_restart_slot = LastRestartSlot {
            last_restart_slot: 0xDEAD_BEEF_CAFE_F00D,
        };
        let mut ctx = ctx_with(10_000, sv);
        let mut buf = [0u8; LAST_RESTART_SLOT_BYTES];
        do_sol_get_last_restart_slot_sysvar(&mut ctx, &mut buf);
        assert_eq!(
            u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            0xDEAD_BEEF_CAFE_F00D
        );
    }

    /// Short buffer must error. Pin against the failure-mode
    /// invariant — a too-small buffer means the program doesn't
    /// have room for the full sysvar struct, so partial writes
    /// would silently corrupt the program's stack frame.
    #[test]
    fn short_clock_buffer_returns_custom() {
        let mut ctx = ctx_with(10_000, Sysvars::default());
        let mut small = [0u8; CLOCK_BYTES - 1];
        let r = do_sol_get_clock_sysvar(&mut ctx, &mut small);
        assert!(matches!(r, SyscallResult::Custom(_)));
    }

    /// Out-of-meter on a sysvar fetch terminates without
    /// debiting partially.
    #[test]
    fn sysvar_out_of_meter_short_circuits() {
        let mut ctx = ctx_with(50, Sysvars::default());
        let mut buf = [0u8; CLOCK_BYTES];
        let r = do_sol_get_clock_sysvar(&mut ctx, &mut buf);
        assert_eq!(r, SyscallResult::OutOfMeter);
        assert_eq!(ctx.remaining_units, 50); // not partially debited
                                             // Buffer untouched.
        assert!(buf.iter().all(|&b| b == 0));
    }

    /// EpochRewards layout: u64+u64+[u8;32]+u128+u64+u64+bool+15-byte
    /// pad. Pin against the wire format the production runtime
    /// emits.
    #[test]
    fn epoch_rewards_layout_canonical() {
        use crate::sysvar::EpochRewards;
        let mut sv = Sysvars::default();
        sv.epoch_rewards = EpochRewards {
            distribution_starting_block_height: 0xAABB_CCDD_EEFF_0011,
            num_partitions: 0x1122_3344_5566_7788,
            parent_blockhash: [0xCDu8; 32],
            total_points: 0xFEED_BABE_DEAD_BEEF_FEED_BABE_DEAD_BEEF,
            total_rewards: 0x9988_7766_5544_3322,
            distributed_rewards: 0x1234_5678_9ABC_DEF0,
            active: true,
        };
        let mut ctx = ctx_with(10_000, sv);
        let mut buf = [0xFFu8; EPOCH_REWARDS_BYTES];
        let r = do_sol_get_epoch_rewards_sysvar(&mut ctx, &mut buf);
        assert_eq!(r, SyscallResult::Ok);
        assert_eq!(
            u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            0xAABB_CCDD_EEFF_0011
        );
        assert_eq!(
            u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            0x1122_3344_5566_7788
        );
        assert_eq!(buf[16..48], [0xCDu8; 32]);
        assert_eq!(
            u128::from_le_bytes(buf[48..64].try_into().unwrap()),
            0xFEED_BABE_DEAD_BEEF_FEED_BABE_DEAD_BEEF
        );
        assert_eq!(
            u64::from_le_bytes(buf[64..72].try_into().unwrap()),
            0x9988_7766_5544_3322
        );
        assert_eq!(
            u64::from_le_bytes(buf[72..80].try_into().unwrap()),
            0x1234_5678_9ABC_DEF0
        );
        assert_eq!(buf[80], 1);
        // Trailing 15-byte pad must be zero.
        assert_eq!(&buf[81..96], &[0u8; 15]);
    }
}
