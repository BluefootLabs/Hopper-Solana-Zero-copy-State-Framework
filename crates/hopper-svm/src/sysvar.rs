//! Sysvar state, Hopper-flavored.
//!
//! Phase 1 ships shapes for the most-tested sysvars (`Clock`, `Rent`)
//! plus a `Sysvars` aggregate the harness reads through. The shapes
//! are wire-compatible with `solana_sdk::sysvar::*` so a built-in
//! that wants to lift Hopper sysvars into the upstream type can do
//! so via `Into`. Phase 2 adds the `sol_get_*_sysvar` syscall
//! handlers that copy these into a BPF program's heap.
//!
//! Default values:
//!
//! - `Clock` slot 0, epoch 0, unix_timestamp 1_700_000_000 (a
//!   stable November-2023 timestamp; deterministic and easy to
//!   recognise in test output).
//! - `Rent` standard mainnet parameters (`19.055441478_439425` SOL
//!   per byte-year, `2` exemption threshold, `50%` burn).
//!
//! Tests that need a specific clock or rent state override via
//! [`crate::HopperSvm::with_sysvars`].

/// Clock sysvar shape — wire-compatible with
/// `solana_sdk::sysvar::clock::Clock`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clock {
    /// The current slot.
    pub slot: u64,
    /// The timestamp of the first slot in this Solana epoch.
    pub epoch_start_timestamp: i64,
    /// The bank epoch.
    pub epoch: u64,
    /// The future epoch for which the leader schedule is selected.
    pub leader_schedule_epoch: u64,
    /// Originally computed from genesis creation time and the
    /// network's bank target slot duration.
    pub unix_timestamp: i64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            slot: 0,
            epoch_start_timestamp: 1_700_000_000,
            epoch: 0,
            leader_schedule_epoch: 0,
            unix_timestamp: 1_700_000_000,
        }
    }
}

/// Rent sysvar shape — wire-compatible with
/// `solana_sdk::sysvar::rent::Rent`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rent {
    /// Rental rate in lamports/byte-year.
    pub lamports_per_byte_year: u64,
    /// Amount of time (in years) a balance must include rent for to be
    /// rent-exempt.
    pub exemption_threshold: f64,
    /// The percentage of collected rent that is burned.
    pub burn_percent: u8,
}

impl Default for Rent {
    fn default() -> Self {
        // Mainnet defaults — stable values that tests can pin
        // assertions against without coupling to runtime config.
        Self {
            lamports_per_byte_year: 3_480,
            exemption_threshold: 2.0,
            burn_percent: 50,
        }
    }
}

impl Rent {
    /// Minimum lamport balance required for `data_size` bytes to be
    /// rent-exempt. Matches Solana's formula:
    /// `(data_size + 128) * lamports_per_byte_year * exemption_threshold`.
    /// The 128-byte overhead is the on-chain account-metadata size
    /// account holders are billed for.
    pub fn minimum_balance(&self, data_size: usize) -> u64 {
        let billable_bytes = (data_size as u64).saturating_add(128);
        let lamports_per_year = billable_bytes
            .saturating_mul(self.lamports_per_byte_year);
        ((lamports_per_year as f64) * self.exemption_threshold) as u64
    }
}

/// EpochSchedule sysvar shape — wire-compatible with
/// `solana_sdk::epoch_schedule::EpochSchedule`. The on-chain
/// runtime ships fixed values for these; tests can override via
/// `Sysvars::epoch_schedule`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochSchedule {
    /// Maximum number of slots per epoch.
    pub slots_per_epoch: u64,
    /// Number of slots before beginning of an epoch the leader
    /// schedule is selected.
    pub leader_schedule_slot_offset: u64,
    /// Whether epochs ramp up in size from `MINIMUM_SLOTS_PER_EPOCH`
    /// to `slots_per_epoch` (warmup mode) or are immediately full
    /// (no-warmup mode).
    pub warmup: bool,
    /// First epoch where `slots_per_epoch == max`.
    pub first_normal_epoch: u64,
    /// First slot of `first_normal_epoch`.
    pub first_normal_slot: u64,
}

impl Default for EpochSchedule {
    fn default() -> Self {
        // Mainnet defaults — `MINIMUM_SLOTS_PER_EPOCH = 32`,
        // ramp to 432_000 slots/epoch (~2 days at 400ms slots).
        // Tests that need a tight loop can override.
        Self {
            slots_per_epoch: 432_000,
            leader_schedule_slot_offset: 432_000,
            warmup: false,
            first_normal_epoch: 0,
            first_normal_slot: 0,
        }
    }
}

/// LastRestartSlot sysvar — slot of the most recent cluster
/// restart. Defaults to 0 (no restart since genesis).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastRestartSlot {
    /// Most recent restart slot, or 0 if the cluster has never
    /// been restarted.
    pub last_restart_slot: u64,
}

/// EpochRewards sysvar — wire-compatible with
/// `solana_sdk::epoch_rewards::EpochRewards`. Tracks epoch
/// reward distribution state. Programs read this to know
/// whether rewards are currently being distributed (in which
/// case stake operations are restricted) and how much per-epoch
/// reward is being paid.
#[derive(Clone, Debug, PartialEq)]
pub struct EpochRewards {
    /// Distribution started at this slot.
    pub distribution_starting_block_height: u64,
    /// Number of partitions used for partitioned reward
    /// distribution. 0 means no partitioned distribution active.
    pub num_partitions: u64,
    /// Hash of the last block. 32 bytes.
    pub parent_blockhash: [u8; 32],
    /// Total points (lamports × time) earned this epoch by all
    /// stakeholders.
    pub total_points: u128,
    /// Total rewards (in lamports) distributed this epoch.
    pub total_rewards: u64,
    /// Lamports already distributed.
    pub distributed_rewards: u64,
    /// Whether rewards are currently being distributed.
    pub active: bool,
}

impl Default for EpochRewards {
    fn default() -> Self {
        // Default = "no rewards being distributed" — most tests
        // don't care about epoch boundaries.
        Self {
            distribution_starting_block_height: 0,
            num_partitions: 0,
            parent_blockhash: [0u8; 32],
            total_points: 0,
            total_rewards: 0,
            distributed_rewards: 0,
            active: false,
        }
    }
}

/// All sysvars the Hopper SVM tracks. Cloned into each instruction
/// execution so the program sees a consistent view even if the
/// outer test code mutates the harness's sysvars between calls.
#[derive(Clone, Debug, Default)]
pub struct Sysvars {
    /// Clock sysvar (slot, epoch, timestamps).
    pub clock: Clock,
    /// Rent sysvar (lamports per byte-year).
    pub rent: Rent,
    /// Epoch schedule sysvar — slots-per-epoch, leader-schedule
    /// offset, etc.
    pub epoch_schedule: EpochSchedule,
    /// Last cluster restart slot. Defaults to 0.
    pub last_restart_slot: LastRestartSlot,
    /// Epoch rewards distribution state.
    pub epoch_rewards: EpochRewards,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Rent::minimum_balance` must include the 128-byte metadata
    /// overhead — pin against a known value to catch silent drift.
    #[test]
    fn rent_minimum_balance_includes_metadata_overhead() {
        let r = Rent::default();
        // 0-byte account: 128 * 3_480 * 2.0 = 890_880 lamports.
        assert_eq!(r.minimum_balance(0), 890_880);
        // 100-byte account: 228 * 3_480 * 2.0 = 1_586_880.
        assert_eq!(r.minimum_balance(100), 1_586_880);
    }

    /// Sysvars default to a known-stable state. Catches accidental
    /// drift in the defaults that would silently change every test
    /// snapshot.
    #[test]
    fn sysvars_have_stable_defaults() {
        let s = Sysvars::default();
        assert_eq!(s.clock.slot, 0);
        assert_eq!(s.clock.unix_timestamp, 1_700_000_000);
        assert_eq!(s.rent.burn_percent, 50);
    }
}
