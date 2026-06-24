//! Sysvar access via direct syscalls.
//!
//! Provides zero-alloc, zero-deserialization access to Solana sysvars
//! by reading them directly into stack buffers via syscalls. No framework
//! wraps the epoch schedule sysvar at the native level.

use crate::address::Address;
use crate::error::ProgramError;

// ── Clock ────────────────────────────────────────────────────────────

/// Clock sysvar data, read directly from the runtime.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    pub slot: u64,
    pub epoch_start_timestamp: i64,
    pub epoch: u64,
    pub leader_schedule_epoch: u64,
    pub unix_timestamp: i64,
}

/// Read the Clock sysvar.
#[inline]
pub fn get_clock() -> Result<Clock, ProgramError> {
    #[allow(unused_mut)]
    let mut clock = Clock::default();

    #[cfg(target_os = "solana")]
    {
        let rc =
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            unsafe { crate::syscalls::sol_get_clock_sysvar(&mut clock as *mut Clock as *mut u8) };
        if rc != 0 {
            return Err(ProgramError::UnsupportedSysvar);
        }
    }

    Ok(clock)
}

impl Clock {
    /// Read the Clock sysvar.
    ///
    /// Method-style alias for [`get_clock`], matching the `Sysvar::get()`
    /// ergonomics other Solana frameworks expose (`Clock::get()`).
    #[inline]
    pub fn get() -> Result<Self, ProgramError> {
        get_clock()
    }
}

// ── Rent ─────────────────────────────────────────────────────────────

/// Rent sysvar data.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rent {
    pub lamports_per_byte_year: u64,
    pub exemption_threshold: f64,
    pub burn_percent: u8,
}

/// Lamports charged per byte of account storage per year.
pub const LAMPORTS_PER_BYTE_YEAR: u64 = 3_480;

/// Years of rent an account must prepay to be rent-exempt.
pub const EXEMPTION_THRESHOLD_YEARS: u64 = 2;

/// Fixed per-account storage overhead charged by the cluster.
pub const ACCOUNT_STORAGE_OVERHEAD: u64 = 128;

/// Minimum balance for rent exemption under the current Solana cluster constants.
#[inline]
pub const fn rent_exempt_minimum(data_len: usize) -> u64 {
    (data_len as u64 + ACCOUNT_STORAGE_OVERHEAD)
        * LAMPORTS_PER_BYTE_YEAR
        * EXEMPTION_THRESHOLD_YEARS
}

/// Read the Rent sysvar.
#[inline]
pub fn get_rent() -> Result<Rent, ProgramError> {
    #[allow(unused_mut)]
    let mut rent = Rent::default();

    #[cfg(target_os = "solana")]
    {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let rc = unsafe { crate::syscalls::sol_get_rent_sysvar(&mut rent as *mut Rent as *mut u8) };
        if rc != 0 {
            return Err(ProgramError::UnsupportedSysvar);
        }
    }

    Ok(rent)
}

impl Rent {
    /// Read the Rent sysvar.
    ///
    /// Method-style alias for [`get_rent`], matching the `Sysvar::get()`
    /// ergonomics other Solana frameworks expose (`Rent::get()`).
    #[inline]
    pub fn get() -> Result<Self, ProgramError> {
        get_rent()
    }

    /// Calculate the minimum lamports for rent exemption at the given data size.
    #[inline]
    pub fn minimum_balance(&self, data_len: usize) -> u64 {
        let total_size = (data_len as u64).saturating_add(ACCOUNT_STORAGE_OVERHEAD);
        (total_size as f64 * self.lamports_per_byte_year as f64 * self.exemption_threshold) as u64
    }
}

// ── Epoch Schedule ───────────────────────────────────────────────────

/// Epoch schedule sysvar data.
///
/// Nobody wraps this at the native level. Useful for programs that
/// need to reason about epoch boundaries (staking, vesting, time locks).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EpochSchedule {
    pub slots_per_epoch: u64,
    pub leader_schedule_slot_offset: u64,
    pub warmup: bool,
    pub first_normal_epoch: u64,
    pub first_normal_slot: u64,
}

// ABI lock: `sol_get_epoch_schedule_sysvar` memcpy's the runtime's
// `#[repr(C)]` `EpochSchedule` into this buffer. The canonical Agave
// definition (solana-sdk `epoch-schedule`) is `#[repr(C)]` with field
// order `slots_per_epoch, leader_schedule_slot_offset, warmup,
// first_normal_epoch, first_normal_slot`. The `bool` sits between two
// u64 fields, so the layout depends on `repr(C)` padding — any drift in
// field order or repr here silently misreads every field after `warmup`.
// These asserts fail the build if that ever happens.
const _: () = {
    assert!(core::mem::size_of::<EpochSchedule>() == 40);
    assert!(core::mem::align_of::<EpochSchedule>() == 8);
    assert!(core::mem::offset_of!(EpochSchedule, slots_per_epoch) == 0);
    assert!(core::mem::offset_of!(EpochSchedule, leader_schedule_slot_offset) == 8);
    assert!(core::mem::offset_of!(EpochSchedule, warmup) == 16);
    assert!(core::mem::offset_of!(EpochSchedule, first_normal_epoch) == 24);
    assert!(core::mem::offset_of!(EpochSchedule, first_normal_slot) == 32);
};

// The Clock sysvar is also memcpy'd from a `#[repr(C)]` runtime struct.
const _: () = {
    assert!(core::mem::size_of::<Clock>() == 40);
    assert!(core::mem::offset_of!(Clock, slot) == 0);
    assert!(core::mem::offset_of!(Clock, epoch_start_timestamp) == 8);
    assert!(core::mem::offset_of!(Clock, epoch) == 16);
    assert!(core::mem::offset_of!(Clock, leader_schedule_epoch) == 24);
    assert!(core::mem::offset_of!(Clock, unix_timestamp) == 32);
};

/// Read the EpochSchedule sysvar.
#[inline]
pub fn get_epoch_schedule() -> Result<EpochSchedule, ProgramError> {
    #[allow(unused_mut)]
    let mut schedule = EpochSchedule::default();

    #[cfg(target_os = "solana")]
    {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let rc = unsafe {
            crate::syscalls::sol_get_epoch_schedule_sysvar(
                &mut schedule as *mut EpochSchedule as *mut u8,
            )
        };
        if rc != 0 {
            return Err(ProgramError::UnsupportedSysvar);
        }
    }

    Ok(schedule)
}

impl EpochSchedule {
    /// Get the epoch for a given slot.
    #[inline]
    pub fn get_epoch(&self, slot: u64) -> u64 {
        if slot < self.first_normal_slot {
            // During warmup, epoch length doubles each epoch.
            // Initial epoch has 32 slots (MINIMUM_SLOTS_PER_EPOCH).
            if slot == 0 {
                return 0;
            }
            // log2(slot / 32) + 1, clamped.
            let mut epoch_len: u64 = 32; // MINIMUM_SLOTS_PER_EPOCH
            let mut epoch: u64 = 0;
            let mut slot_remaining = slot;
            while slot_remaining >= epoch_len {
                slot_remaining -= epoch_len;
                epoch += 1;
                epoch_len = epoch_len.saturating_mul(2);
            }
            epoch
        } else {
            let normal_slot_index = slot - self.first_normal_slot;
            self.first_normal_epoch + normal_slot_index / self.slots_per_epoch
        }
    }

    /// Get the first slot in the given epoch.
    #[inline]
    pub fn get_first_slot_in_epoch(&self, epoch: u64) -> u64 {
        if epoch <= self.first_normal_epoch {
            // Warmup: each epoch doubles in length starting from 32.
            if epoch == 0 {
                return 0;
            }
            // First slot = sum of all previous epoch lengths.
            // = 32 * (2^epoch - 1)
            let shift = epoch.min(63);
            32_u64.saturating_mul((1_u64 << shift).saturating_sub(1))
        } else {
            let normal_epoch_index = epoch - self.first_normal_epoch;
            self.first_normal_slot + normal_epoch_index * self.slots_per_epoch
        }
    }
}

// ── Well-known sysvar addresses ──────────────────────────────────────

/// Clock sysvar address.
pub const CLOCK_ID: Address = crate::address!("SysvarC1ock11111111111111111111111111111111");

/// Rent sysvar address.
pub const RENT_ID: Address = crate::address!("SysvarRent111111111111111111111111111111111");

/// Epoch schedule sysvar address.
pub const EPOCH_SCHEDULE_ID: Address =
    crate::address!("SysvarEpochSchedu1e111111111111111111111111");

/// SlotHashes sysvar address.
pub const SLOT_HASHES_ID: Address = crate::address!("SysvarS1otHashes111111111111111111111111111");

/// StakeHistory sysvar address.
pub const STAKE_HISTORY_ID: Address =
    crate::address!("SysvarStakeHistory1111111111111111111111111");

/// Instructions sysvar address (for instruction introspection).
pub const INSTRUCTIONS_ID: Address = crate::address!("Sysvar1nstructions1111111111111111111111111");

/// EpochRewards sysvar address (SIMD-0118).
pub const EPOCH_REWARDS_ID: Address =
    crate::address!("SysvarEpochRewards1111111111111111111111111");

// ── Generalized sysvar access (sol_get_sysvar) ──────────────────────

/// Copy `dst.len()` bytes starting at `offset` from the sysvar identified
/// by `sysvar_id` into `dst`.
///
/// This wraps the modern `sol_get_sysvar` syscall, the only zero-copy way
/// to read large sysvars (SlotHashes, StakeHistory) without passing them
/// as instruction accounts. Returns `Err(UnsupportedSysvar)` on syscall
/// failure (e.g. reading past the sysvar's length).
#[inline]
pub fn get_sysvar_into(
    sysvar_id: &Address,
    offset: u64,
    dst: &mut [u8],
) -> Result<(), ProgramError> {
    #[cfg(target_os = "solana")]
    {
        // SAFETY: `sysvar_id` is a 32-byte address; `dst` is valid for its
        // own length; the syscall copies exactly `dst.len()` bytes.
        let rc = unsafe {
            crate::syscalls::sol_get_sysvar(
                sysvar_id.as_array().as_ptr(),
                dst.as_mut_ptr(),
                offset,
                dst.len() as u64,
            )
        };
        if rc != 0 {
            return Err(ProgramError::UnsupportedSysvar);
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (sysvar_id, offset, dst);
    }
    Ok(())
}

// ── Epoch stake (sol_get_epoch_stake, SIMD-0133) ────────────────────

/// Get the current-epoch activated stake of the vote account at `vote`.
#[inline]
pub fn get_epoch_stake(vote: &Address) -> u64 {
    #[cfg(target_os = "solana")]
    {
        // SAFETY: `vote` is a 32-byte address pointer the syscall reads.
        unsafe { crate::syscalls::sol_get_epoch_stake(vote.as_array().as_ptr()) }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = vote;
        0
    }
}

/// Get the cluster-wide total activated stake for the current epoch.
#[inline]
pub fn get_total_epoch_stake() -> u64 {
    #[cfg(target_os = "solana")]
    {
        // SAFETY: a null `vote_address` is the documented request for the
        // cluster total (SIMD-0133).
        unsafe { crate::syscalls::sol_get_epoch_stake(core::ptr::null()) }
    }
    #[cfg(not(target_os = "solana"))]
    {
        0
    }
}

// ── LastRestartSlot (SIMD-0047) ─────────────────────────────────────

/// LastRestartSlot sysvar address.
pub const LAST_RESTART_SLOT_ID: Address =
    crate::address!("SysvarLastRestartS1ot1111111111111111111111");

/// Read the slot of the last cluster restart (hard fork), or `0` if the
/// cluster has never been restarted.
///
/// This wraps the dedicated `sol_get_last_restart_slot` syscall
/// (SIMD-0047). Programs that must reason about whether state predates a
/// restart (oracle freshness, liveness windows) read it here instead of
/// passing the sysvar as an account.
#[inline]
pub fn get_last_restart_slot() -> Result<u64, ProgramError> {
    #[allow(unused_mut)]
    let mut slot: u64 = 0;
    #[cfg(target_os = "solana")]
    {
        // SAFETY: the syscall writes a single `u64` into the 8-byte buffer
        // `slot` points at; `slot` is a live stack local for the call.
        let rc =
            unsafe { crate::syscalls::sol_get_last_restart_slot(&mut slot as *mut u64 as *mut u8) };
        if rc != 0 {
            return Err(ProgramError::UnsupportedSysvar);
        }
    }
    Ok(slot)
}

// ── SlotHashes ──────────────────────────────────────────────────────

/// One `(slot, hash)` entry from the SlotHashes sysvar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotHash {
    pub slot: u64,
    pub hash: [u8; 32],
}

/// Read the most recent `(slot, hash)` from the SlotHashes sysvar.
///
/// SlotHashes is a length-prefixed list ordered most-recent-first:
/// `u64 count` then `count` entries of `slot(u64) + hash([u8;32])`. This
/// reads just the count and the first entry (48 bytes total) via
/// `sol_get_sysvar`, avoiding the cost of materializing the full 16 KiB
/// sysvar. Returns `Ok(None)` when the list is empty.
#[inline]
pub fn slot_hashes_latest() -> Result<Option<SlotHash>, ProgramError> {
    let mut count_buf = [0u8; 8];
    get_sysvar_into(&SLOT_HASHES_ID, 0, &mut count_buf)?;
    let count = u64::from_le_bytes(count_buf);
    if count == 0 {
        return Ok(None);
    }
    let mut entry = [0u8; 40];
    get_sysvar_into(&SLOT_HASHES_ID, 8, &mut entry)?;
    let slot = u64::from_le_bytes([
        entry[0], entry[1], entry[2], entry[3], entry[4], entry[5], entry[6], entry[7],
    ]);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&entry[8..40]);
    Ok(Some(SlotHash { slot, hash }))
}

// ── StakeHistory ────────────────────────────────────────────────────

/// One epoch's stake-history entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StakeHistoryEntry {
    pub epoch: u64,
    pub effective: u64,
    pub activating: u64,
    pub deactivating: u64,
}

/// Read the most recent stake-history entry.
///
/// StakeHistory is a length-prefixed list ordered most-recent-first:
/// `u64 count` then entries of `epoch(u64) + effective(u64) +
/// activating(u64) + deactivating(u64)` (32 bytes each). Returns
/// `Ok(None)` when the history is empty.
#[inline]
pub fn stake_history_latest() -> Result<Option<StakeHistoryEntry>, ProgramError> {
    let mut count_buf = [0u8; 8];
    get_sysvar_into(&STAKE_HISTORY_ID, 0, &mut count_buf)?;
    let count = u64::from_le_bytes(count_buf);
    if count == 0 {
        return Ok(None);
    }
    let mut entry = [0u8; 32];
    get_sysvar_into(&STAKE_HISTORY_ID, 8, &mut entry)?;
    let rd = |o: usize| {
        u64::from_le_bytes([
            entry[o],
            entry[o + 1],
            entry[o + 2],
            entry[o + 3],
            entry[o + 4],
            entry[o + 5],
            entry[o + 6],
            entry[o + 7],
        ])
    };
    Ok(Some(StakeHistoryEntry {
        epoch: rd(0),
        effective: rd(8),
        activating: rd(16),
        deactivating: rd(24),
    }))
}

// ── EpochRewards (SIMD-0118) ────────────────────────────────────────

/// EpochRewards sysvar data (SIMD-0118).
///
/// Surfaces the partitioned-rewards distribution state for the current
/// epoch: how many lamports are being paid out, how far distribution has
/// progressed, and whether the rewards period is still active. Staking and
/// airdrop programs read it to gate behaviour during the distribution
/// window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EpochRewards {
    /// First block height at which rewards distribution begins this epoch.
    pub distribution_starting_block_height: u64,
    /// Number of partitions the distribution is split across.
    pub num_partitions: u64,
    /// Blockhash of the parent of the epoch's first block.
    pub parent_blockhash: [u8; 32],
    /// Total rewards points calculated for the epoch.
    pub total_points: u128,
    /// Total rewards (lamports) calculated for the epoch.
    pub total_rewards: u64,
    /// Rewards (lamports) distributed so far this epoch.
    pub distributed_rewards: u64,
    /// Whether the rewards period (calculation + distribution) is active.
    pub active: bool,
}

/// Wire size of the bincode-serialized EpochRewards sysvar account data.
///
/// `8 + 8 + 32 + 16 + 8 + 8 + 1`. Unlike Clock/Rent/EpochSchedule (which
/// have dedicated syscalls that memcpy a `#[repr(C)]` struct), EpochRewards
/// is only reachable through `sol_get_sysvar`, which returns the sysvar
/// *account data* in bincode form: a flat little-endian field concatenation
/// with no padding. The decoder below mirrors that image byte-for-byte
/// rather than casting a `#[repr(C)]` struct (whose `u128` would force
/// 16-byte alignment and 96-byte size, reading past the 81-byte image).
const EPOCH_REWARDS_LEN: usize = 81;

#[inline]
fn decode_epoch_rewards(buf: &[u8; EPOCH_REWARDS_LEN]) -> EpochRewards {
    let rd8 = |o: usize| {
        u64::from_le_bytes([
            buf[o],
            buf[o + 1],
            buf[o + 2],
            buf[o + 3],
            buf[o + 4],
            buf[o + 5],
            buf[o + 6],
            buf[o + 7],
        ])
    };
    let mut parent_blockhash = [0u8; 32];
    parent_blockhash.copy_from_slice(&buf[16..48]);
    let mut points = [0u8; 16];
    points.copy_from_slice(&buf[48..64]);
    EpochRewards {
        distribution_starting_block_height: rd8(0),
        num_partitions: rd8(8),
        parent_blockhash,
        total_points: u128::from_le_bytes(points),
        total_rewards: rd8(64),
        distributed_rewards: rd8(72),
        active: buf[80] != 0,
    }
}

/// Read the EpochRewards sysvar (SIMD-0118).
///
/// Reads the bincode account-data image via `sol_get_sysvar` and decodes
/// it without alignment assumptions. Off-chain this returns the zeroed
/// default (`active = false`). Returns `Err(UnsupportedSysvar)` if the
/// syscall fails (e.g. the sysvar is unavailable on the cluster).
#[inline]
pub fn get_epoch_rewards() -> Result<EpochRewards, ProgramError> {
    let mut buf = [0u8; EPOCH_REWARDS_LEN];
    get_sysvar_into(&EPOCH_REWARDS_ID, 0, &mut buf)?;
    Ok(decode_epoch_rewards(&buf))
}

#[cfg(test)]
mod abi_tests {
    use super::*;

    /// Reproduce the byte image the runtime memcpy's for EpochSchedule and
    /// confirm every field reads from the offset the syscall writes. This
    /// is the runtime counterpart to the compile-time offset asserts: it
    /// proves the read side, not just the struct shape.
    #[test]
    fn epoch_schedule_reads_canonical_byte_image() {
        // Devnet/mainnet default: 432_000 slots/epoch, no warmup.
        let mut buf = [0u8; 40];
        buf[0..8].copy_from_slice(&432_000u64.to_le_bytes()); // slots_per_epoch
        buf[8..16].copy_from_slice(&432_000u64.to_le_bytes()); // leader_schedule_slot_offset
        buf[16] = 0; // warmup = false
                     // bytes 17..24 are padding
        buf[24..32].copy_from_slice(&0u64.to_le_bytes()); // first_normal_epoch
        buf[32..40].copy_from_slice(&0u64.to_le_bytes()); // first_normal_slot

        // SAFETY: `EpochSchedule` is repr(C), size 40, and `buf` is 40 bytes
        // matching the canonical wire image asserted above.
        let sched: EpochSchedule = unsafe { core::ptr::read(buf.as_ptr() as *const EpochSchedule) };
        assert_eq!(sched.slots_per_epoch, 432_000);
        assert_eq!(sched.leader_schedule_slot_offset, 432_000);
        assert!(!sched.warmup);
        assert_eq!(sched.first_normal_epoch, 0);
        assert_eq!(sched.first_normal_slot, 0);
    }

    #[test]
    fn clock_reads_canonical_byte_image() {
        let mut buf = [0u8; 40];
        buf[0..8].copy_from_slice(&123u64.to_le_bytes()); // slot
        buf[8..16].copy_from_slice(&1_600_000_000i64.to_le_bytes()); // epoch_start_timestamp
        buf[16..24].copy_from_slice(&7u64.to_le_bytes()); // epoch
        buf[24..32].copy_from_slice(&8u64.to_le_bytes()); // leader_schedule_epoch
        buf[32..40].copy_from_slice(&1_600_000_500i64.to_le_bytes()); // unix_timestamp

        // SAFETY: `Clock` is repr(C), size 40, matching this 40-byte image.
        let clock: Clock = unsafe { core::ptr::read(buf.as_ptr() as *const Clock) };
        assert_eq!(clock.slot, 123);
        assert_eq!(clock.epoch_start_timestamp, 1_600_000_000);
        assert_eq!(clock.epoch, 7);
        assert_eq!(clock.leader_schedule_epoch, 8);
        assert_eq!(clock.unix_timestamp, 1_600_000_500);
    }

    /// Build the canonical bincode image for EpochRewards and confirm the
    /// decoder reads every field from the byte offset the runtime writes.
    /// This is the read-side proof for the no-dedicated-syscall sysvar.
    #[test]
    fn epoch_rewards_decodes_canonical_byte_image() {
        let mut buf = [0u8; EPOCH_REWARDS_LEN];
        buf[0..8].copy_from_slice(&100u64.to_le_bytes()); // distribution_starting_block_height
        buf[8..16].copy_from_slice(&8u64.to_le_bytes()); // num_partitions
        buf[16..48].copy_from_slice(&[7u8; 32]); // parent_blockhash
        buf[48..64].copy_from_slice(&123_456_789u128.to_le_bytes()); // total_points
        buf[64..72].copy_from_slice(&5_000_000u64.to_le_bytes()); // total_rewards
        buf[72..80].copy_from_slice(&1_250_000u64.to_le_bytes()); // distributed_rewards
        buf[80] = 1; // active = true

        let er = decode_epoch_rewards(&buf);
        assert_eq!(er.distribution_starting_block_height, 100);
        assert_eq!(er.num_partitions, 8);
        assert_eq!(er.parent_blockhash, [7u8; 32]);
        assert_eq!(er.total_points, 123_456_789);
        assert_eq!(er.total_rewards, 5_000_000);
        assert_eq!(er.distributed_rewards, 1_250_000);
        assert!(er.active);
    }

    #[test]
    fn epoch_rewards_off_chain_is_zeroed_default() {
        // Off-chain `get_sysvar_into` is a no-op, so the getter yields the
        // zeroed default with `active = false`.
        let er = get_epoch_rewards().unwrap();
        assert_eq!(er, EpochRewards::default());
        assert!(!er.active);
    }
}
