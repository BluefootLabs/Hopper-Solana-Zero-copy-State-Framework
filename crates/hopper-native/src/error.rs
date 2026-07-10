//! Program error type for Solana on-chain programs.
//!
//! Wire-compatible with pinocchio/solana-program ProgramError.
//! Each variant maps to a fixed u64 error code returned to the runtime.

/// Errors that a Solana program can return.
///
/// `#[repr(u64)]` with EXPLICIT discriminants is load-bearing for binary
/// size: variant `k` (for the fieldless builtins, `k = 1..=24`) encodes to
/// the runtime code `(k + 1) << 32`, so [`From<ProgramError> for u64`] is
/// one tag read + one shift instead of a 25-arm match. The DWARF size
/// attribution measured that match at 880 bytes — 13% of the parity
/// vault's `.text` — because the error lowering inlines into every
/// entrypoint. Keep new variants in this scheme: append with the next
/// sequential discriminant and the conversion stays arm-free (the
/// exhaustive `u64_conversion_matches_reference_for_every_variant` test
/// enforces the correspondence).
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum ProgramError {
    /// Custom program error with a u32 code.
    Custom(u32) = 0,
    InvalidArgument = 1,
    InvalidInstructionData = 2,
    InvalidAccountData = 3,
    AccountDataTooSmall = 4,
    InsufficientFunds = 5,
    IncorrectProgramId = 6,
    MissingRequiredSignature = 7,
    AccountAlreadyInitialized = 8,
    UninitializedAccount = 9,
    NotEnoughAccountKeys = 10,
    AccountBorrowFailed = 11,
    MaxSeedLengthExceeded = 12,
    InvalidSeeds = 13,
    BorshIoError = 14,
    AccountNotRentExempt = 15,
    UnsupportedSysvar = 16,
    IllegalOwner = 17,
    MaxAccountsDataAllocationsExceeded = 18,
    InvalidRealloc = 19,
    MaxInstructionTraceLengthExceeded = 20,
    BuiltinProgramsMustConsumeComputeUnits = 21,
    InvalidAccountOwner = 22,
    ArithmeticOverflow = 23,
    Immutable = 24,
    IncorrectAuthority = 25,
}

// ── u64 conversion (Solana runtime ABI) ──────────────────────────────

impl From<ProgramError> for u64 {
    #[inline]
    fn from(err: ProgramError) -> u64 {
        match err {
            ProgramError::Custom(0) => CUSTOM_ZERO,
            ProgramError::Custom(code) => code as u64,
            builtin => {
                // SAFETY: `ProgramError` is `#[repr(u64)]`, which
                // guarantees the discriminant is stored as a leading
                // `u64` tag readable through a pointer cast (RFC 2195
                // primitive-representation layout). `builtin` is one of
                // the fieldless variants (Custom was matched above), so
                // its tag is the explicit discriminant `1..=25`.
                let tag = unsafe { *(&builtin as *const ProgramError as *const u64) };
                // Variant k encodes to (k + 1) << 32: InvalidArgument
                // (tag 1) -> 2 << 32, ..., IncorrectAuthority (tag 25)
                // -> 26 << 32 — exactly the old per-arm to_builtin(k-1)
                // table, without the 25-arm match in every entrypoint.
                (tag + 1) << BUILTIN_BIT_SHIFT
            }
        }
    }
}

impl From<u64> for ProgramError {
    fn from(code: u64) -> Self {
        if code == CUSTOM_ZERO {
            return ProgramError::Custom(0);
        }
        let builtin = code >> BUILTIN_BIT_SHIFT;
        if code & BUILTIN_LOW_MASK == 0 && builtin >= 2 {
            match builtin - 2 {
                0 => return ProgramError::InvalidArgument,
                1 => return ProgramError::InvalidInstructionData,
                2 => return ProgramError::InvalidAccountData,
                3 => return ProgramError::AccountDataTooSmall,
                4 => return ProgramError::InsufficientFunds,
                5 => return ProgramError::IncorrectProgramId,
                6 => return ProgramError::MissingRequiredSignature,
                7 => return ProgramError::AccountAlreadyInitialized,
                8 => return ProgramError::UninitializedAccount,
                9 => return ProgramError::NotEnoughAccountKeys,
                10 => return ProgramError::AccountBorrowFailed,
                11 => return ProgramError::MaxSeedLengthExceeded,
                12 => return ProgramError::InvalidSeeds,
                13 => return ProgramError::BorshIoError,
                14 => return ProgramError::AccountNotRentExempt,
                15 => return ProgramError::UnsupportedSysvar,
                16 => return ProgramError::IllegalOwner,
                17 => return ProgramError::MaxAccountsDataAllocationsExceeded,
                18 => return ProgramError::InvalidRealloc,
                19 => return ProgramError::MaxInstructionTraceLengthExceeded,
                20 => return ProgramError::BuiltinProgramsMustConsumeComputeUnits,
                21 => return ProgramError::InvalidAccountOwner,
                22 => return ProgramError::ArithmeticOverflow,
                23 => return ProgramError::Immutable,
                24 => return ProgramError::IncorrectAuthority,
                _ => {}
            }
        }
        ProgramError::Custom(code as u32)
    }
}

/// Map a builtin error index to its runtime u64 code.
///
/// The Solana runtime uses a specific encoding for builtin errors:
/// - `Custom(0)` occupies `1 << 32`
/// - builtin errors start at `2 << 32`
const BUILTIN_BIT_SHIFT: usize = 32;
const CUSTOM_ZERO: u64 = 1_u64 << BUILTIN_BIT_SHIFT;
const BUILTIN_LOW_MASK: u64 = (1_u64 << BUILTIN_BIT_SHIFT) - 1;

/// Reference builtin encoding, kept ONLY as the test oracle: the
/// shipped conversion reads the enum tag directly, and the golden
/// tests below re-derive every variant's code through this original
/// arithmetic to pin the two forever equal.
#[cfg(test)]
#[inline(always)]
const fn to_builtin(index: u64) -> u64 {
    (index + 2) << BUILTIN_BIT_SHIFT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant's u64 code, pinned against the pre-tag-read
    /// reference table (the old 25-arm match, reproduced here verbatim).
    /// The match is EXHAUSTIVE on purpose: adding a variant without
    /// updating this test — and without giving it the next sequential
    /// discriminant — must not compile.
    #[test]
    fn u64_conversion_matches_reference_for_every_variant() {
        fn reference(err: &ProgramError) -> u64 {
            match err {
                ProgramError::Custom(0) => CUSTOM_ZERO,
                ProgramError::Custom(code) => *code as u64,
                ProgramError::InvalidArgument => to_builtin(0),
                ProgramError::InvalidInstructionData => to_builtin(1),
                ProgramError::InvalidAccountData => to_builtin(2),
                ProgramError::AccountDataTooSmall => to_builtin(3),
                ProgramError::InsufficientFunds => to_builtin(4),
                ProgramError::IncorrectProgramId => to_builtin(5),
                ProgramError::MissingRequiredSignature => to_builtin(6),
                ProgramError::AccountAlreadyInitialized => to_builtin(7),
                ProgramError::UninitializedAccount => to_builtin(8),
                ProgramError::NotEnoughAccountKeys => to_builtin(9),
                ProgramError::AccountBorrowFailed => to_builtin(10),
                ProgramError::MaxSeedLengthExceeded => to_builtin(11),
                ProgramError::InvalidSeeds => to_builtin(12),
                ProgramError::BorshIoError => to_builtin(13),
                ProgramError::AccountNotRentExempt => to_builtin(14),
                ProgramError::UnsupportedSysvar => to_builtin(15),
                ProgramError::IllegalOwner => to_builtin(16),
                ProgramError::MaxAccountsDataAllocationsExceeded => to_builtin(17),
                ProgramError::InvalidRealloc => to_builtin(18),
                ProgramError::MaxInstructionTraceLengthExceeded => to_builtin(19),
                ProgramError::BuiltinProgramsMustConsumeComputeUnits => to_builtin(20),
                ProgramError::InvalidAccountOwner => to_builtin(21),
                ProgramError::ArithmeticOverflow => to_builtin(22),
                ProgramError::Immutable => to_builtin(23),
                ProgramError::IncorrectAuthority => to_builtin(24),
            }
        }

        let all = [
            ProgramError::Custom(0),
            ProgramError::Custom(1),
            ProgramError::Custom(0xD003),
            ProgramError::Custom(u32::MAX),
            ProgramError::InvalidArgument,
            ProgramError::InvalidInstructionData,
            ProgramError::InvalidAccountData,
            ProgramError::AccountDataTooSmall,
            ProgramError::InsufficientFunds,
            ProgramError::IncorrectProgramId,
            ProgramError::MissingRequiredSignature,
            ProgramError::AccountAlreadyInitialized,
            ProgramError::UninitializedAccount,
            ProgramError::NotEnoughAccountKeys,
            ProgramError::AccountBorrowFailed,
            ProgramError::MaxSeedLengthExceeded,
            ProgramError::InvalidSeeds,
            ProgramError::BorshIoError,
            ProgramError::AccountNotRentExempt,
            ProgramError::UnsupportedSysvar,
            ProgramError::IllegalOwner,
            ProgramError::MaxAccountsDataAllocationsExceeded,
            ProgramError::InvalidRealloc,
            ProgramError::MaxInstructionTraceLengthExceeded,
            ProgramError::BuiltinProgramsMustConsumeComputeUnits,
            ProgramError::InvalidAccountOwner,
            ProgramError::ArithmeticOverflow,
            ProgramError::Immutable,
            ProgramError::IncorrectAuthority,
        ];
        for err in all {
            let want = reference(&err);
            let got: u64 = err.clone().into();
            assert_eq!(got, want, "u64 code diverged for {err:?}");
            // And the reverse mapping still round-trips builtins and
            // the custom sentinel exactly as before.
            assert_eq!(ProgramError::from(want), err, "roundtrip for {err:?}");
        }
    }
}

impl core::fmt::Display for ProgramError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProgramError::Custom(code) => write!(f, "Custom({code})"),
            ProgramError::InvalidArgument => write!(f, "InvalidArgument"),
            ProgramError::InvalidInstructionData => write!(f, "InvalidInstructionData"),
            ProgramError::InvalidAccountData => write!(f, "InvalidAccountData"),
            ProgramError::AccountDataTooSmall => write!(f, "AccountDataTooSmall"),
            ProgramError::InsufficientFunds => write!(f, "InsufficientFunds"),
            ProgramError::IncorrectProgramId => write!(f, "IncorrectProgramId"),
            ProgramError::MissingRequiredSignature => write!(f, "MissingRequiredSignature"),
            ProgramError::AccountAlreadyInitialized => write!(f, "AccountAlreadyInitialized"),
            ProgramError::UninitializedAccount => write!(f, "UninitializedAccount"),
            ProgramError::NotEnoughAccountKeys => write!(f, "NotEnoughAccountKeys"),
            ProgramError::AccountBorrowFailed => write!(f, "AccountBorrowFailed"),
            ProgramError::MaxSeedLengthExceeded => write!(f, "MaxSeedLengthExceeded"),
            ProgramError::InvalidSeeds => write!(f, "InvalidSeeds"),
            ProgramError::BorshIoError => write!(f, "BorshIoError"),
            ProgramError::AccountNotRentExempt => write!(f, "AccountNotRentExempt"),
            ProgramError::UnsupportedSysvar => write!(f, "UnsupportedSysvar"),
            ProgramError::IllegalOwner => write!(f, "IllegalOwner"),
            ProgramError::MaxAccountsDataAllocationsExceeded => {
                write!(f, "MaxAccountsDataAllocationsExceeded")
            }
            ProgramError::InvalidRealloc => write!(f, "InvalidRealloc"),
            ProgramError::MaxInstructionTraceLengthExceeded => {
                write!(f, "MaxInstructionTraceLengthExceeded")
            }
            ProgramError::BuiltinProgramsMustConsumeComputeUnits => {
                write!(f, "BuiltinProgramsMustConsumeComputeUnits")
            }
            ProgramError::InvalidAccountOwner => write!(f, "InvalidAccountOwner"),
            ProgramError::ArithmeticOverflow => write!(f, "ArithmeticOverflow"),
            ProgramError::Immutable => write!(f, "Immutable"),
            ProgramError::IncorrectAuthority => write!(f, "IncorrectAuthority"),
        }
    }
}
