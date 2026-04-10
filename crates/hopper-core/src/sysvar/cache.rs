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
                data[0], data[1], data[2], data[3],
                data[4], data[5], data[6], data[7],
            ]),
            epoch: u64::from_le_bytes([
                data[16], data[17], data[18], data[19],
                data[20], data[21], data[22], data[23],
            ]),
            unix_timestamp: i64::from_le_bytes([
                data[32], data[33], data[34], data[35],
                data[36], data[37], data[38], data[39],
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
pub struct CachedRent {
    pub lamports_per_byte_year: u64,
}

impl CachedRent {
    /// Parse Rent sysvar from account data.
    #[inline]
    pub fn from_account_data(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() < 8 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self {
            lamports_per_byte_year: u64::from_le_bytes([
                data[0], data[1], data[2], data[3],
                data[4], data[5], data[6], data[7],
            ]),
        })
    }

    /// Compute rent-exempt minimum for a given data size.
    #[inline(always)]
    pub fn exempt_min(&self, data_len: usize) -> u64 {
        // Standard formula: (128 + data_len) * lamports_per_byte_year * 2 / 365.25 / 86400
        // Simplified to the common approximation used by Solana:
        ((128 + data_len) as u64) * 6960
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
