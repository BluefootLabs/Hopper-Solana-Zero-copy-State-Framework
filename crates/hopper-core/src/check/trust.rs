//! Foreign-account trust profiles.
//!
//! Configurable validation policies for loading accounts owned by external
//! programs. Each profile defines which checks to enforce, allowing
//! programs to explicitly declare their trust assumptions.
//!
//! ## Trust Levels
//!
//! - **Strict**: owner + layout_id + exact size + not frozen/closed
//! - **Compatible**: owner + layout_id + minimum size (supports newer versions)
//! - **Observational**: layout_id only, best-effort (indexers/tooling)
//!
//! ```ignore
//! let profile = TrustProfile::strict(&KNOWN_PROGRAM_ID, &MyLayout::LAYOUT_ID, MyLayout::LEN);
//! let data = profile.load(account)?;
//! let overlay = MyLayout::overlay(data)?;
//! ```

use hopper_runtime::error::ProgramError;
use hopper_runtime::{AccountView, Address};

/// Trust level for foreign account validation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Full validation: owner + layout_id + exact size + not closed.
    Strict,
    /// Version-compatible: owner + layout_id + minimum size.
    Compatible,
    /// Best-effort: layout_id match only. For tooling and indexers.
    Observational,
}

/// Policy flags for additional constraints.
#[derive(Clone, Copy)]
pub struct TrustFlags {
    /// Reject accounts with the close sentinel (disc == 0xFF).
    pub reject_closed: bool,
    /// Require the account to be immutable (not writable).
    pub require_immutable: bool,
    /// Minimum version (byte 1 of header). 0 = no minimum.
    pub min_version: u8,
}

impl TrustFlags {
    /// Default flags: reject closed, no immutability requirement, no version floor.
    #[inline(always)]
    pub const fn default() -> Self {
        Self {
            reject_closed: true,
            require_immutable: false,
            min_version: 0,
        }
    }

    /// Paranoid mode: reject closed + require immutable.
    #[inline(always)]
    pub const fn paranoid() -> Self {
        Self {
            reject_closed: true,
            require_immutable: true,
            min_version: 0,
        }
    }
}

/// A foreign-account trust profile.
///
/// Encapsulates the expected owner, layout_id, size, and trust level
/// so that foreign account loading is explicit and auditable.
pub struct TrustProfile<'a> {
    /// Expected owner program.
    pub owner: &'a Address,
    /// Expected layout_id (first 8 bytes of SHA-256 hash).
    pub layout_id: &'a [u8; 8],
    /// Expected size (exact for Strict, minimum for Compatible, ignored for Observational).
    pub size: usize,
    /// Trust level.
    pub level: TrustLevel,
    /// Additional flags.
    pub flags: TrustFlags,
}

impl<'a> TrustProfile<'a> {
    /// Strict profile: full validation.
    #[inline(always)]
    pub const fn strict(owner: &'a Address, layout_id: &'a [u8; 8], size: usize) -> Self {
        Self {
            owner,
            layout_id,
            size,
            level: TrustLevel::Strict,
            flags: TrustFlags::default(),
        }
    }

    /// Compatible profile: accepts newer versions with larger accounts.
    #[inline(always)]
    pub const fn compatible(owner: &'a Address, layout_id: &'a [u8; 8], min_size: usize) -> Self {
        Self {
            owner,
            layout_id,
            size: min_size,
            level: TrustLevel::Compatible,
            flags: TrustFlags::default(),
        }
    }

    /// Observational profile: layout_id only, for tooling.
    #[inline(always)]
    pub const fn observational(layout_id: &'a [u8; 8]) -> Self {
        // Observational mode: zeroed address is intentional -- owner check
        // is skipped by load_observational(), so this value is never read.
        const ZERO_ADDR: Address = Address::new_from_array([0u8; 32]);
        Self {
            owner: &ZERO_ADDR,
            layout_id,
            size: 0,
            level: TrustLevel::Observational,
            flags: TrustFlags {
                reject_closed: false,
                require_immutable: false,
                min_version: 0,
            },
        }
    }

    /// Read-only profile: owner + layout_id + minimum size + require immutable.
    ///
    /// Like `compatible()` but additionally requires the account to not be
    /// writable. Use this when reading cross-program state that must not
    /// be mutated within the same transaction.
    #[inline(always)]
    pub const fn read_only(owner: &'a Address, layout_id: &'a [u8; 8], min_size: usize) -> Self {
        Self {
            owner,
            layout_id,
            size: min_size,
            level: TrustLevel::Compatible,
            flags: TrustFlags {
                reject_closed: true,
                require_immutable: true,
                min_version: 0,
            },
        }
    }

    /// Set the minimum version floor.
    #[inline(always)]
    pub const fn with_min_version(mut self, v: u8) -> Self {
        self.flags.min_version = v;
        self
    }

    /// Require the account to be immutable (not writable).
    #[inline(always)]
    pub const fn require_immutable(mut self) -> Self {
        self.flags.require_immutable = true;
        self
    }

    /// Validate an account against this profile and return its data.
    ///
    /// On success, returns a byte slice suitable for overlay.
    #[inline]
    pub fn load(&self, account: &'a AccountView) -> Result<&'a [u8], ProgramError> {
        // Immutability check (if required).
        if self.flags.require_immutable && account.is_writable() {
            return Err(ProgramError::InvalidAccountData);
        }

        match self.level {
            TrustLevel::Strict => self.load_strict(account),
            TrustLevel::Compatible => self.load_compatible(account),
            TrustLevel::Observational => self.load_observational(account),
        }
    }

    #[inline]
    fn load_strict(&self, account: &'a AccountView) -> Result<&'a [u8], ProgramError> {
        // Owner check.
        if !account.owned_by(self.owner) {
            return Err(ProgramError::IncorrectProgramId);
        }
        // SAFETY: Read-only borrow for validation. No conflicting mutable borrows.
        let data = unsafe { account.borrow_unchecked() };
        // Exact size check.
        if data.len() != self.size {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // Layout ID check.
        self.check_layout_id(data)?;
        // Close sentinel check.
        if self.flags.reject_closed {
            self.check_not_closed(data)?;
        }
        // Version floor check.
        if self.flags.min_version > 0 {
            self.check_min_version(data)?;
        }
        Ok(data)
    }

    #[inline]
    fn load_compatible(&self, account: &'a AccountView) -> Result<&'a [u8], ProgramError> {
        if !account.owned_by(self.owner) {
            return Err(ProgramError::IncorrectProgramId);
        }
        // SAFETY: Read-only borrow for validation.
        let data = unsafe { account.borrow_unchecked() };
        // Minimum size check (account may be larger than expected).
        if data.len() < self.size {
            return Err(ProgramError::AccountDataTooSmall);
        }
        self.check_layout_id(data)?;
        if self.flags.reject_closed {
            self.check_not_closed(data)?;
        }
        if self.flags.min_version > 0 {
            self.check_min_version(data)?;
        }
        Ok(data)
    }

    #[inline]
    fn load_observational(&self, account: &'a AccountView) -> Result<&'a [u8], ProgramError> {
        // SAFETY: Read-only borrow. No owner check for observational mode.
        let data = unsafe { account.borrow_unchecked() };
        if data.len() < crate::account::HEADER_LEN {
            return Err(ProgramError::AccountDataTooSmall);
        }
        self.check_layout_id(data)?;
        Ok(data)
    }

    /// Check the layout_id in the header matches expected.
    #[inline(always)]
    fn check_layout_id(&self, data: &[u8]) -> Result<(), ProgramError> {
        if data.len() < 12 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        if data[4..12] != *self.layout_id {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Check the account is not closed (disc != CLOSE_SENTINEL).
    #[inline(always)]
    fn check_not_closed(&self, data: &[u8]) -> Result<(), ProgramError> {
        if !data.is_empty() && data[0] == crate::account::CLOSE_SENTINEL {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Check version meets the floor.
    #[inline(always)]
    fn check_min_version(&self, data: &[u8]) -> Result<(), ProgramError> {
        if data.len() < 2 {
            return Err(ProgramError::AccountDataTooSmall);
        }
        if data[1] < self.flags.min_version {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }
}

/// Load a foreign account with a trust profile, returning a typed overlay.
///
/// Convenience function combining profile validation with Pod overlay.
#[inline]
pub fn load_foreign_with_profile<'a, T: crate::account::Pod + crate::account::FixedLayout>(
    account: &'a AccountView,
    profile: &TrustProfile<'a>,
) -> Result<crate::account::VerifiedAccount<'a, T>, ProgramError> {
    let data = profile.load(account)?;
    crate::account::VerifiedAccount::new(data)
}
