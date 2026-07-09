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

/// Fixed per-account storage overhead the cluster charges on top of user
/// data (128 bytes: header + metadata). One of the three inputs to the
/// rent-exempt minimum, shared with [`cache::CachedRent::exempt_min`].
pub const ACCOUNT_STORAGE_OVERHEAD: u64 = 128;

/// Rent sysvar fields.
///
/// Mirrors Solana's `solana_rent::Rent` (and `hopper-native`'s `Rent`): the
/// on-wire sysvar stores `exemption_threshold` as an `f64`, so we carry it
/// as an `f64` here too rather than hardcoding a rational. If the cluster
/// ever reprices the exemption threshold (a future SIMD), [`read_rent`]
/// surfaces the live value and [`Rent::minimum_balance`] charges the correct
/// amount — the earlier `exemption_threshold_num:2 / _den:1` form silently
/// ignored the stored bytes and would under/over-fund accounts after a
/// reprice, risking reaping and data loss.
#[derive(Clone, Copy)]
pub struct Rent {
    pub lamports_per_byte_year: u64,
    pub exemption_threshold: f64,
    pub burn_percent: u8,
}

/// Read the Rent sysvar from account data.
///
/// The Rent sysvar is bincode-serialized in field order with no padding:
/// `[lamports_per_byte_year:u64][exemption_threshold:f64][burn_percent:u8]`
/// = 17 bytes. The `exemption_threshold` at bytes `[8..16]` is a little-
/// endian `f64` (`2.0` on today's cluster); we read it verbatim via
/// `f64::from_le_bytes` rather than assuming a value, so a reprice is
/// honored. (The prior reader demanded 25 bytes and then discarded the
/// stored threshold entirely, hardcoding 2/1 — both are fixed here.)
#[inline]
pub fn read_rent(data: &[u8]) -> Result<Rent, ProgramError> {
    if data.len() < 17 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(Rent {
        lamports_per_byte_year: u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]),
        exemption_threshold: f64::from_le_bytes([
            data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
        ]),
        burn_percent: data[16],
    })
}

impl Rent {
    /// Minimum lamports for rent exemption at `data_len`, computed from the
    /// **live sysvar** values — the correct source for reaping-relevant
    /// decisions after a rent reprice.
    ///
    /// Byte-matches Solana's own `solana_rent::Rent::minimum_balance`:
    ///
    /// ```text
    /// (((ACCOUNT_STORAGE_OVERHEAD + data_len) * lamports_per_byte_year) as f64
    ///     * exemption_threshold) as u64
    /// ```
    ///
    /// The runtime does the `(overhead + bytes) * lamports_per_byte_year`
    /// product in **integer** and applies the fractional `exemption_threshold`
    /// (a `2.0` float today) as the *only* f64 step, then truncates. We
    /// replicate that exact sequence so our result equals the runtime's for
    /// every input it accepts. The integer product uses saturating ops purely
    /// as an overflow guard; for every loader-permitted `data_len`
    /// (`<= 10_485_760`) and realistic `lamports_per_byte_year` it never
    /// saturates, so the byte-match is exact. This mirrors
    /// `hopper_native::sysvar::Rent::minimum_balance` field-for-field.
    #[inline]
    pub fn minimum_balance(&self, data_len: usize) -> u64 {
        let bytes = data_len as u64;
        let integer_part = ACCOUNT_STORAGE_OVERHEAD
            .saturating_add(bytes)
            .saturating_mul(self.lamports_per_byte_year);
        (integer_part as f64 * self.exemption_threshold) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loader bound on serialized account data (10 MiB) — the largest
    /// `data_len` any rent calculation ever sees on-chain.
    const LOADER_MAX_DATA_LEN: usize = 10_485_760;

    /// Byte-for-byte transcription of Solana's `Rent::minimum_balance`:
    /// integer `(overhead + bytes) * lpby`, then the single f64
    /// `exemption_threshold` multiply, then truncate.
    fn solana_reference_minimum_balance(data_len: usize, lpby: u64, threshold: f64) -> u64 {
        let bytes = data_len as u64;
        (((ACCOUNT_STORAGE_OVERHEAD + bytes) * lpby) as f64 * threshold) as u64
    }

    /// Build a canonical 17-byte Rent sysvar image.
    fn rent_bytes(lpby: u64, threshold: f64, burn_percent: u8) -> [u8; 17] {
        let mut buf = [0u8; 17];
        buf[0..8].copy_from_slice(&lpby.to_le_bytes());
        buf[8..16].copy_from_slice(&threshold.to_le_bytes());
        buf[16] = burn_percent;
        buf
    }

    /// The core fix: `read_rent` must return the threshold stored in the
    /// bytes, NOT a hardcoded 2/1. A non-2.0 threshold (2.5) round-trips.
    #[test]
    fn read_rent_honors_stored_non_two_threshold() {
        let buf = rent_bytes(3_480, 2.5, 5);
        let rent = read_rent(&buf).unwrap();
        assert_eq!(rent.lamports_per_byte_year, 3_480);
        assert_eq!(rent.exemption_threshold, 2.5);
        assert_eq!(rent.burn_percent, 5);
    }

    /// A repriced per-byte cost and threshold both survive the read.
    #[test]
    fn read_rent_honors_repriced_fields() {
        let buf = rent_bytes(6_960, 3.0, 50);
        let rent = read_rent(&buf).unwrap();
        assert_eq!(rent.lamports_per_byte_year, 6_960);
        assert_eq!(rent.exemption_threshold, 3.0);
        assert_eq!(rent.burn_percent, 50);
    }

    /// Data shorter than the 17-byte sysvar image is rejected; exactly 17
    /// bytes (a real Rent sysvar) is accepted.
    #[test]
    fn read_rent_rejects_short_data_accepts_17() {
        assert!(read_rent(&[0u8; 16]).is_err());
        assert!(read_rent(&rent_bytes(3_480, 2.0, 0)).is_ok());
    }

    /// `minimum_balance` must byte-match Solana's runtime formula across a
    /// range of sizes, repriced per-byte costs, and fractional thresholds —
    /// including a `lamports_per_byte_year` past f64's 53-bit exact-integer
    /// range, where an all-f64 formula would drift.
    #[test]
    fn minimum_balance_byte_matches_solana_reference() {
        let cases: &[(usize, u64, f64)] = &[
            (0, 3_480, 2.0),
            (165, 3_480, 2.0),
            (10_240, 3_480, 2.0),
            (1_000_000, 6_960, 2.0),  // hypothetical 2x reprice
            (500_000, 3_480, 2.5),    // fractional threshold reprice
            (4_096, 3_480, 3.0),      // upward threshold reprice
            (10_485_760, 3_480, 2.0), // max size
            // `lpby` just past f64's 2^53 exact-integer range with a small
            // `data_len` so the integer product stays inside u64.
            (1_024, 9_007_199_254_740_993, 2.0),
        ];
        for &(dl, lpby, threshold) in cases {
            let rent = Rent {
                lamports_per_byte_year: lpby,
                exemption_threshold: threshold,
                burn_percent: 0,
            };
            assert_eq!(
                rent.minimum_balance(dl),
                solana_reference_minimum_balance(dl, lpby, threshold),
                "minimum_balance != Solana reference at dl={dl}, lpby={lpby}, threshold={threshold}"
            );
        }
    }

    /// On today's cluster config (lpby=3480, threshold=2.0) the empty-account
    /// minimum is the well-known 890_880 lamports.
    #[test]
    fn minimum_balance_matches_mainnet_empty_account() {
        let rent = Rent {
            lamports_per_byte_year: 3_480,
            exemption_threshold: 2.0,
            burn_percent: 0,
        };
        assert_eq!(rent.minimum_balance(0), 890_880);
    }

    /// The saturating integer product must not overflow or panic even with a
    /// wildly repriced `lamports_per_byte_year` at the maximum data length.
    #[test]
    fn minimum_balance_no_overflow_at_extremes() {
        let rent = Rent {
            lamports_per_byte_year: 1u64 << 40, // ~300x today's value
            exemption_threshold: 2.0,
            burn_percent: 0,
        };
        let _ = rent.minimum_balance(LOADER_MAX_DATA_LEN);
        assert_eq!(
            rent.minimum_balance(0),
            solana_reference_minimum_balance(0, 1u64 << 40, 2.0)
        );
    }

    /// After an UPWARD threshold reprice the live sysvar demands strictly
    /// more than the old hardcoded-2.0 path — the correctness gap this fix
    /// closes. A 2.0 reader would under-fund and leave the account reapable.
    #[test]
    fn upward_threshold_reprice_demands_more_than_hardcoded_two() {
        let dl = 4_096;
        let hardcoded_two = Rent {
            lamports_per_byte_year: 3_480,
            exemption_threshold: 2.0,
            burn_percent: 0,
        }
        .minimum_balance(dl);
        let repriced = Rent {
            lamports_per_byte_year: 3_480,
            exemption_threshold: 2.5,
            burn_percent: 0,
        }
        .minimum_balance(dl);
        assert!(
            repriced > hardcoded_two,
            "repriced ({repriced}) must exceed hardcoded-2.0 ({hardcoded_two})"
        );
    }
}
