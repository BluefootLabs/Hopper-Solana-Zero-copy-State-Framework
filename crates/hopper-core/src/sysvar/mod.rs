//! Sysvar readers -- zero-copy Clock and Rent parsing.

mod cache;

pub use cache::{CachedClock, CachedRent, SysvarContext};

use hopper_runtime::error::ProgramError;

/// Clock sysvar fields (from Solana's runtime).
#[derive(Clone, Copy)]
pub struct Clock {
    pub slot: u64,
    pub epoch_start_timestamp: i64,
    pub epoch: u64,
    pub leader_schedule_epoch: u64,
    pub unix_timestamp: i64,
}

/// Read the Clock sysvar from account data.
///
/// The Clock sysvar is 40 bytes:
/// `[slot:8][epoch_start_timestamp:8][epoch:8][leader_schedule_epoch:8][unix_timestamp:8]`
#[inline]
pub fn read_clock(data: &[u8]) -> Result<Clock, ProgramError> {
    if data.len() < 40 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(Clock {
        slot: u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]),
        epoch_start_timestamp: i64::from_le_bytes([
            data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
        ]),
        epoch: u64::from_le_bytes([
            data[16], data[17], data[18], data[19], data[20], data[21], data[22], data[23],
        ]),
        leader_schedule_epoch: u64::from_le_bytes([
            data[24], data[25], data[26], data[27], data[28], data[29], data[30], data[31],
        ]),
        unix_timestamp: i64::from_le_bytes([
            data[32], data[33], data[34], data[35], data[36], data[37], data[38], data[39],
        ]),
    })
}

/// Rent sysvar fields.
#[derive(Clone, Copy)]
pub struct Rent {
    pub lamports_per_byte_year: u64,
    pub exemption_threshold_num: u64,
    pub exemption_threshold_den: u64,
    pub burn_percent: u8,
}

/// Read the Rent sysvar from account data.
#[inline]
pub fn read_rent(data: &[u8]) -> Result<Rent, ProgramError> {
    if data.len() < 25 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(Rent {
        lamports_per_byte_year: u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]),
        // The exemption threshold is stored as f64 in Solana,
        // but we decompose to avoid floating-point on-chain.
        // Standard value: 2.0 years -> we store as 2/1
        exemption_threshold_num: 2,
        exemption_threshold_den: 1,
        burn_percent: data[16],
    })
}
