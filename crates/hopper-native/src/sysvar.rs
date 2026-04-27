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
            unsafe { crate::syscalls::sol_get_clock_sysvar(&mut clock as *mut Clock as *mut u8) };
        if rc != 0 {
            return Err(ProgramError::UnsupportedSysvar);
        }
    }

    Ok(clock)
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

/// Read the Rent sysvar.
#[inline]
pub fn get_rent() -> Result<Rent, ProgramError> {
    #[allow(unused_mut)]
    let mut rent = Rent::default();

    #[cfg(target_os = "solana")]
    {
        let rc = unsafe { crate::syscalls::sol_get_rent_sysvar(&mut rent as *mut Rent as *mut u8) };
        if rc != 0 {
            return Err(ProgramError::UnsupportedSysvar);
        }
    }

    Ok(rent)
}

impl Rent {
    /// Calculate the minimum lamports for rent exemption at the given data size.
    #[inline]
    pub fn minimum_balance(&self, data_len: usize) -> u64 {
        // Total account size = data + 128 bytes of account metadata overhead.
        let total_size = (data_len as u64).saturating_add(128);
        let lamports =
            (total_size as f64) * self.lamports_per_byte_year as f64 * self.exemption_threshold;
        lamports as u64
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

/// Read the EpochSchedule sysvar.
#[inline]
pub fn get_epoch_schedule() -> Result<EpochSchedule, ProgramError> {
    #[allow(unused_mut)]
    let mut schedule = EpochSchedule::default();

    #[cfg(target_os = "solana")]
    {
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
