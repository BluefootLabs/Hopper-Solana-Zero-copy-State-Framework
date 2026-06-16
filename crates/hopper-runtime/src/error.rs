//! Hopper-owned program error type for Solana on-chain programs.
//!
//! Each variant maps to a fixed u64 error code returned to the Solana runtime.

/// Errors that a Solana program can return.
///
/// This is part of the Hopper runtime type surface. Variant discriminants
/// match the Solana runtime ABI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProgramError {
    /// Custom program error with a u32 code.
    Custom(u32),
    InvalidArgument,
    InvalidInstructionData,
    InvalidAccountData,
    AccountDataTooSmall,
    InsufficientFunds,
    IncorrectProgramId,
    MissingRequiredSignature,
    AccountAlreadyInitialized,
    UninitializedAccount,
    NotEnoughAccountKeys,
    AccountBorrowFailed,
    MaxSeedLengthExceeded,
    InvalidSeeds,
    BorshIoError,
    AccountNotRentExempt,
    UnsupportedSysvar,
    IllegalOwner,
    MaxAccountsDataAllocationsExceeded,
    InvalidRealloc,
    MaxInstructionTraceLengthExceeded,
    BuiltinProgramsMustConsumeComputeUnits,
    InvalidAccountOwner,
    ArithmeticOverflow,
    Immutable,
    IncorrectAuthority,
}

// ── u64 conversion (Solana runtime ABI) ──────────────────────────────

/// Map a builtin error index to its runtime u64 code.
const BUILTIN_BIT_SHIFT: usize = 32;
const CUSTOM_ZERO: u64 = 1_u64 << BUILTIN_BIT_SHIFT;

/// Hopper builtin errors follow Solana's ABI:
/// - `Custom(0)` occupies `1 << 32`
/// - builtin errors start at `2 << 32`
#[inline(always)]
const fn to_builtin(index: u64) -> u64 {
    (index + 2) << BUILTIN_BIT_SHIFT
}

const BUILTIN_LOW_MASK: u64 = (1_u64 << BUILTIN_BIT_SHIFT) - 1;

impl From<ProgramError> for u64 {
    fn from(err: ProgramError) -> u64 {
        match err {
            ProgramError::Custom(0) => CUSTOM_ZERO,
            ProgramError::Custom(code) => code as u64,
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

// ── Backend conversions ──────────────────────────────────────────────

/// Convert between Hopper and backend ProgramError via u64 error codes.
///
/// Both types encode errors as the same u64 values, so we round-trip
/// through the integer representation. This is zero-cost in the happy
/// path (errors are exceptional).
impl From<hopper_native::error::ProgramError> for ProgramError {
    #[inline]
    fn from(e: hopper_native::error::ProgramError) -> Self {
        match e {
            hopper_native::error::ProgramError::Custom(code) => ProgramError::Custom(code),
            hopper_native::error::ProgramError::InvalidArgument => ProgramError::InvalidArgument,
            hopper_native::error::ProgramError::InvalidInstructionData => {
                ProgramError::InvalidInstructionData
            }
            hopper_native::error::ProgramError::InvalidAccountData => {
                ProgramError::InvalidAccountData
            }
            hopper_native::error::ProgramError::AccountDataTooSmall => {
                ProgramError::AccountDataTooSmall
            }
            hopper_native::error::ProgramError::InsufficientFunds => {
                ProgramError::InsufficientFunds
            }
            hopper_native::error::ProgramError::IncorrectProgramId => {
                ProgramError::IncorrectProgramId
            }
            hopper_native::error::ProgramError::MissingRequiredSignature => {
                ProgramError::MissingRequiredSignature
            }
            hopper_native::error::ProgramError::AccountAlreadyInitialized => {
                ProgramError::AccountAlreadyInitialized
            }
            hopper_native::error::ProgramError::UninitializedAccount => {
                ProgramError::UninitializedAccount
            }
            hopper_native::error::ProgramError::NotEnoughAccountKeys => {
                ProgramError::NotEnoughAccountKeys
            }
            hopper_native::error::ProgramError::AccountBorrowFailed => {
                ProgramError::AccountBorrowFailed
            }
            hopper_native::error::ProgramError::MaxSeedLengthExceeded => {
                ProgramError::MaxSeedLengthExceeded
            }
            hopper_native::error::ProgramError::InvalidSeeds => ProgramError::InvalidSeeds,
            hopper_native::error::ProgramError::BorshIoError => ProgramError::BorshIoError,
            hopper_native::error::ProgramError::AccountNotRentExempt => {
                ProgramError::AccountNotRentExempt
            }
            hopper_native::error::ProgramError::UnsupportedSysvar => {
                ProgramError::UnsupportedSysvar
            }
            hopper_native::error::ProgramError::IllegalOwner => ProgramError::IllegalOwner,
            hopper_native::error::ProgramError::MaxAccountsDataAllocationsExceeded => {
                ProgramError::MaxAccountsDataAllocationsExceeded
            }
            hopper_native::error::ProgramError::InvalidRealloc => ProgramError::InvalidRealloc,
            hopper_native::error::ProgramError::MaxInstructionTraceLengthExceeded => {
                ProgramError::MaxInstructionTraceLengthExceeded
            }
            hopper_native::error::ProgramError::BuiltinProgramsMustConsumeComputeUnits => {
                ProgramError::BuiltinProgramsMustConsumeComputeUnits
            }
            hopper_native::error::ProgramError::InvalidAccountOwner => {
                ProgramError::InvalidAccountOwner
            }
            hopper_native::error::ProgramError::ArithmeticOverflow => {
                ProgramError::ArithmeticOverflow
            }
            hopper_native::error::ProgramError::Immutable => ProgramError::Immutable,
            hopper_native::error::ProgramError::IncorrectAuthority => {
                ProgramError::IncorrectAuthority
            }
        }
    }
}

impl From<ProgramError> for hopper_native::error::ProgramError {
    #[inline]
    fn from(e: ProgramError) -> Self {
        match e {
            ProgramError::Custom(code) => hopper_native::error::ProgramError::Custom(code),
            ProgramError::InvalidArgument => hopper_native::error::ProgramError::InvalidArgument,
            ProgramError::InvalidInstructionData => {
                hopper_native::error::ProgramError::InvalidInstructionData
            }
            ProgramError::InvalidAccountData => {
                hopper_native::error::ProgramError::InvalidAccountData
            }
            ProgramError::AccountDataTooSmall => {
                hopper_native::error::ProgramError::AccountDataTooSmall
            }
            ProgramError::InsufficientFunds => {
                hopper_native::error::ProgramError::InsufficientFunds
            }
            ProgramError::IncorrectProgramId => {
                hopper_native::error::ProgramError::IncorrectProgramId
            }
            ProgramError::MissingRequiredSignature => {
                hopper_native::error::ProgramError::MissingRequiredSignature
            }
            ProgramError::AccountAlreadyInitialized => {
                hopper_native::error::ProgramError::AccountAlreadyInitialized
            }
            ProgramError::UninitializedAccount => {
                hopper_native::error::ProgramError::UninitializedAccount
            }
            ProgramError::NotEnoughAccountKeys => {
                hopper_native::error::ProgramError::NotEnoughAccountKeys
            }
            ProgramError::AccountBorrowFailed => {
                hopper_native::error::ProgramError::AccountBorrowFailed
            }
            ProgramError::MaxSeedLengthExceeded => {
                hopper_native::error::ProgramError::MaxSeedLengthExceeded
            }
            ProgramError::InvalidSeeds => hopper_native::error::ProgramError::InvalidSeeds,
            ProgramError::BorshIoError => hopper_native::error::ProgramError::BorshIoError,
            ProgramError::AccountNotRentExempt => {
                hopper_native::error::ProgramError::AccountNotRentExempt
            }
            ProgramError::UnsupportedSysvar => {
                hopper_native::error::ProgramError::UnsupportedSysvar
            }
            ProgramError::IllegalOwner => hopper_native::error::ProgramError::IllegalOwner,
            ProgramError::MaxAccountsDataAllocationsExceeded => {
                hopper_native::error::ProgramError::MaxAccountsDataAllocationsExceeded
            }
            ProgramError::InvalidRealloc => hopper_native::error::ProgramError::InvalidRealloc,
            ProgramError::MaxInstructionTraceLengthExceeded => {
                hopper_native::error::ProgramError::MaxInstructionTraceLengthExceeded
            }
            ProgramError::BuiltinProgramsMustConsumeComputeUnits => {
                hopper_native::error::ProgramError::BuiltinProgramsMustConsumeComputeUnits
            }
            ProgramError::InvalidAccountOwner => {
                hopper_native::error::ProgramError::InvalidAccountOwner
            }
            ProgramError::ArithmeticOverflow => {
                hopper_native::error::ProgramError::ArithmeticOverflow
            }
            ProgramError::Immutable => hopper_native::error::ProgramError::Immutable,
            ProgramError::IncorrectAuthority => {
                hopper_native::error::ProgramError::IncorrectAuthority
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Cold error constructors
// ══════════════════════════════════════════════════════════════════════
//
// `#[cold]` + `#[inline(never)]` on error return helpers keeps the error path
// out of the hot-path instruction cache.
// Call sites become a single branch + call, keeping the inlined fast path tiny.

impl ProgramError {
    #[inline(always)]
    pub fn err_data_too_small<T>() -> Result<T, Self> {
        Err(ProgramError::AccountDataTooSmall)
    }

    #[cold]
    #[inline(never)]
    pub fn err_invalid_data<T>() -> Result<T, Self> {
        Err(ProgramError::InvalidAccountData)
    }

    #[cold]
    #[inline(never)]
    pub fn err_missing_signer<T>() -> Result<T, Self> {
        Err(ProgramError::MissingRequiredSignature)
    }

    #[inline(always)]
    pub fn err_immutable<T>() -> Result<T, Self> {
        Err(ProgramError::Immutable)
    }

    #[cold]
    #[inline(never)]
    pub fn err_not_enough_keys<T>() -> Result<T, Self> {
        Err(ProgramError::NotEnoughAccountKeys)
    }

    #[cold]
    #[inline(never)]
    pub fn err_borrow_failed<T>() -> Result<T, Self> {
        Err(ProgramError::AccountBorrowFailed)
    }

    #[cold]
    #[inline(never)]
    pub fn err_overflow<T>() -> Result<T, Self> {
        Err(ProgramError::ArithmeticOverflow)
    }

    #[cold]
    #[inline(never)]
    pub fn err_invalid_argument<T>() -> Result<T, Self> {
        Err(ProgramError::InvalidArgument)
    }

    #[inline(always)]
    pub fn err_incorrect_program<T>() -> Result<T, Self> {
        Err(ProgramError::IncorrectProgramId)
    }
}
