//! Sysvar context cache -- avoid repeated syscall reads.
//!
//! Inspired by Star Frame's context caching pattern. When multiple
//! checks in one instruction need the same sysvar (e.g., clock for
//! deadline + staleness), reading it once and caching saves ~100+ CU
//! per duplicate read.
//!
//! All caching is stack-local -- no global state, no heap.

use hopper_runtime::error::ProgramError;

/// Cached Clock sysvar fields.
///
/// Created once per instruction, used by multiple checks.
/// Each field is `Option` -- populated lazily on first access
/// from account data.
pub struct CachedClock {
    pub slot: u64,
    pub epoch: u64,
    pub unix_timestamp: i64,
}

impl CachedClock {
    /// Parse Clock sysvar from account data (40 bytes).
    ///
    /// Call once at the start of your instruction, then pass
    /// the cached value to all checks that need clock data.
    #[inline]
    pub fn from_account_data(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() < 40 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self {
            slot: u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]),
            epoch: u64::from_le_bytes([
                data[16], data[17], data[18], data[19], data[20], data[21], data[22], data[23],
            ]),
            unix_timestamp: i64::from_le_bytes([
                data[32], data[33], data[34], data[35], data[36], data[37], data[38], data[39],
            ]),
        })
    }

    /// Check that a deadline has not passed.
    #[inline(always)]
    pub fn check_not_expired(&self, deadline: i64) -> Result<(), ProgramError> {
        if self.unix_timestamp > deadline {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Check that a deadline HAS passed (for claiming, unlocking, etc.).
    #[inline(always)]
    pub fn check_expired(&self, deadline: i64) -> Result<(), ProgramError> {
        if self.unix_timestamp <= deadline {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Check that now is within a time window [start, end].
    #[inline(always)]
    pub fn check_within_window(&self, start: i64, end: i64) -> Result<(), ProgramError> {
        if self.unix_timestamp < start || self.unix_timestamp > end {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Check cooldown: enough time has passed since last action.
    #[inline(always)]
    pub fn check_cooldown(&self, last_action: i64, cooldown_secs: i64) -> Result<(), ProgramError> {
        if self.unix_timestamp < last_action + cooldown_secs {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Check slot staleness: last_update_slot is within max_age of current slot.
    #[inline(always)]
    pub fn check_slot_staleness(
        &self,
        last_update_slot: u64,
        max_age: u64,
    ) -> Result<(), ProgramError> {
        if self.slot.saturating_sub(last_update_slot) > max_age {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }
}

/// Cached Rent sysvar fields.
///
/// Carries the two inputs to the rent-exempt minimum straight from the live
/// sysvar: `lamports_per_byte_year` and the `exemption_threshold` `f64`. The
/// earlier form stored only `lamports_per_byte_year` and then *ignored* it in
/// `exempt_min`, hardcoding `6960` (= 3480 * 2) — which baked in BOTH today's
/// per-byte cost and a 2.0 threshold. After any rent reprice that
/// under/over-funds accounts; now both values are read from the bytes.
pub struct CachedRent {
    pub lamports_per_byte_year: u64,
    pub exemption_threshold: f64,
}

impl CachedRent {
    /// Parse Rent sysvar from account data.
    ///
    /// Reads `lamports_per_byte_year` (bytes `[0..8]`) and
    /// `exemption_threshold` (bytes `[8..16]`, a little-endian `f64`). Needs
    /// at least 16 bytes; a real Rent sysvar is 17 (the trailing byte is
    /// `burn_percent`, which the exempt-minimum calc does not use).
    #[inline]
    pub fn from_account_data(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() < 16 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self {
            lamports_per_byte_year: u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]),
            exemption_threshold: f64::from_le_bytes([
                data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
            ]),
        })
    }

    /// Compute the rent-exempt minimum for a given data size.
    ///
    /// Byte-matches Solana's `solana_rent::Rent::minimum_balance` (and
    /// [`super::Rent::minimum_balance`]): the
    /// `(ACCOUNT_STORAGE_OVERHEAD + data_len) * lamports_per_byte_year`
    /// product in **integer**, then the single `exemption_threshold` f64
    /// multiply, then truncate. It is NOT divided by seconds/year — the real
    /// runtime formula has no such division; the old `/365.25/86400` comment
    /// described a formula the code never actually ran (it hardcoded `6960`).
    ///
    /// The integer product is saturating purely as an overflow guard; for
    /// every loader-permitted `data_len` (`<= 10 MiB`) and realistic
    /// `lamports_per_byte_year` it never saturates, so the byte-match with the
    /// runtime is exact. On today's cluster (lpby=3480, threshold=2.0) this
    /// equals the prior `(128 + data_len) * 6960`, so the on-cluster result is
    /// unchanged — only a repriced cluster now gets the correct (larger/
    /// smaller) value instead of a stale one.
    #[inline(always)]
    pub fn exempt_min(&self, data_len: usize) -> u64 {
        let integer_part = super::ACCOUNT_STORAGE_OVERHEAD
            .saturating_add(data_len as u64)
            .saturating_mul(self.lamports_per_byte_year);
        (integer_part as f64 * self.exemption_threshold) as u64
    }
}

/// Combined sysvar context for a single instruction.
///
/// Parse all needed sysvars once at the top of your instruction handler,
/// then pass this context to all validation functions.
///
/// ```ignore
/// let ctx = SysvarContext::new()
///     .with_clock(&clock_account)?
///     .with_rent(&rent_account)?;
///
/// ctx.clock()?.check_not_expired(deadline)?;
/// ctx.clock()?.check_slot_staleness(oracle_slot, 50)?;
/// ```
pub struct SysvarContext {
    clock: Option<CachedClock>,
    rent: Option<CachedRent>,
}

impl SysvarContext {
    /// Create an empty context.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            clock: None,
            rent: None,
        }
    }

    /// Parse and cache the Clock sysvar.
    #[inline]
    pub fn with_clock(mut self, clock_data: &[u8]) -> Result<Self, ProgramError> {
        self.clock = Some(CachedClock::from_account_data(clock_data)?);
        Ok(self)
    }

    /// Parse and cache the Rent sysvar.
    #[inline]
    pub fn with_rent(mut self, rent_data: &[u8]) -> Result<Self, ProgramError> {
        self.rent = Some(CachedRent::from_account_data(rent_data)?);
        Ok(self)
    }

    /// Get the cached Clock. Returns error if not initialized.
    #[inline(always)]
    pub fn clock(&self) -> Result<&CachedClock, ProgramError> {
        match &self.clock {
            Some(c) => Ok(c),
            None => Err(ProgramError::InvalidArgument),
        }
    }

    /// Get the cached Rent. Returns error if not initialized.
    #[inline(always)]
    pub fn rent(&self) -> Result<&CachedRent, ProgramError> {
        match &self.rent {
            Some(r) => Ok(r),
            None => Err(ProgramError::InvalidArgument),
        }
    }

    /// Check if clock is available.
    #[inline(always)]
    pub fn has_clock(&self) -> bool {
        self.clock.is_some()
    }

    /// Check if rent is available.
    #[inline(always)]
    pub fn has_rent(&self) -> bool {
        self.rent.is_some()
    }
}

impl Default for SysvarContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOADER_MAX_DATA_LEN: usize = 10_485_760;

    /// Solana's reference `Rent::minimum_balance`.
    fn solana_reference_exempt_min(data_len: usize, lpby: u64, threshold: f64) -> u64 {
        (((super::super::ACCOUNT_STORAGE_OVERHEAD + data_len as u64) * lpby) as f64 * threshold)
            as u64
    }

    fn rent_bytes(lpby: u64, threshold: f64) -> [u8; 17] {
        let mut buf = [0u8; 17];
        buf[0..8].copy_from_slice(&lpby.to_le_bytes());
        buf[8..16].copy_from_slice(&threshold.to_le_bytes());
        buf
    }

    /// `CachedRent` must read the stored threshold, not a hardcoded 2.0.
    #[test]
    fn cached_rent_reads_stored_threshold() {
        let cr = CachedRent::from_account_data(&rent_bytes(3_480, 2.5)).unwrap();
        assert_eq!(cr.lamports_per_byte_year, 3_480);
        assert_eq!(cr.exemption_threshold, 2.5);
    }

    /// Needs 16 bytes; the old 8-byte inputs no longer parse (they lack the
    /// threshold), a real 17-byte sysvar does.
    #[test]
    fn cached_rent_length_check() {
        assert!(CachedRent::from_account_data(&[0u8; 15]).is_err());
        assert!(CachedRent::from_account_data(&rent_bytes(3_480, 2.0)).is_ok());
    }

    /// `exempt_min` byte-matches Solana across sizes/reprices, and still
    /// equals the old `(128 + data_len) * 6960` on today's config.
    #[test]
    fn exempt_min_byte_matches_solana_reference() {
        let cases: &[(usize, u64, f64)] = &[
            (0, 3_480, 2.0),
            (56, 3_480, 2.0),
            (165, 3_480, 2.0),
            (10_240, 3_480, 2.0),
            (1_000_000, 6_960, 2.0),
            (500_000, 3_480, 2.5),
            (10_485_760, 3_480, 2.0),
        ];
        for &(dl, lpby, threshold) in cases {
            let cr = CachedRent::from_account_data(&rent_bytes(lpby, threshold)).unwrap();
            assert_eq!(
                cr.exempt_min(dl),
                solana_reference_exempt_min(dl, lpby, threshold),
                "exempt_min != Solana reference at dl={dl}, lpby={lpby}, threshold={threshold}"
            );
        }
    }

    /// On today's cluster config `exempt_min` reproduces the prior
    /// `(128 + data_len) * 6960` exactly — the on-cluster path is unchanged.
    #[test]
    fn exempt_min_backward_compatible_on_current_config() {
        let cr = CachedRent::from_account_data(&rent_bytes(3_480, 2.0)).unwrap();
        for &dl in &[0usize, 56, 128, 1024, 10_240, 1_000_000] {
            assert_eq!(cr.exempt_min(dl), ((128 + dl) as u64) * 6960);
        }
    }

    /// No overflow/panic at a wildly repriced per-byte cost and max size.
    #[test]
    fn exempt_min_no_overflow_at_extremes() {
        let cr = CachedRent::from_account_data(&rent_bytes(1u64 << 40, 2.0)).unwrap();
        let _ = cr.exempt_min(LOADER_MAX_DATA_LEN);
        assert_eq!(
            cr.exempt_min(0),
            solana_reference_exempt_min(0, 1u64 << 40, 2.0)
        );
    }

    /// The cache path threads the live threshold through `with_rent`.
    #[test]
    fn context_with_rent_exposes_repriced_threshold() {
        let ctx = SysvarContext::new()
            .with_rent(&rent_bytes(3_480, 2.5))
            .unwrap();
        let cr = ctx.rent().unwrap();
        assert_eq!(cr.exemption_threshold, 2.5);
        assert_eq!(cr.exempt_min(0), solana_reference_exempt_min(0, 3_480, 2.5));
    }
}
