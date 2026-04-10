//! Program error type for Solana on-chain programs.
//!
//! Wire-compatible with pinocchio/solana-program ProgramError.
//! Each variant maps to a fixed u64 error code returned to the runtime.

/// Errors that a Solana program can return.
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

impl From<ProgramError> for u64 {
    fn from(err: ProgramError) -> u64 {
        match err {
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
        match code {
            c if c == to_builtin(0) => ProgramError::InvalidArgument,
            c if c == to_builtin(1) => ProgramError::InvalidInstructionData,
            c if c == to_builtin(2) => ProgramError::InvalidAccountData,
            c if c == to_builtin(3) => ProgramError::AccountDataTooSmall,
            c if c == to_builtin(4) => ProgramError::InsufficientFunds,
            c if c == to_builtin(5) => ProgramError::IncorrectProgramId,
            c if c == to_builtin(6) => ProgramError::MissingRequiredSignature,
            c if c == to_builtin(7) => ProgramError::AccountAlreadyInitialized,
            c if c == to_builtin(8) => ProgramError::UninitializedAccount,
            c if c == to_builtin(9) => ProgramError::NotEnoughAccountKeys,
            c if c == to_builtin(10) => ProgramError::AccountBorrowFailed,
            c if c == to_builtin(11) => ProgramError::MaxSeedLengthExceeded,
            c if c == to_builtin(12) => ProgramError::InvalidSeeds,
            c if c == to_builtin(13) => ProgramError::BorshIoError,
            c if c == to_builtin(14) => ProgramError::AccountNotRentExempt,
            c if c == to_builtin(15) => ProgramError::UnsupportedSysvar,
            c if c == to_builtin(16) => ProgramError::IllegalOwner,
            c if c == to_builtin(17) => ProgramError::MaxAccountsDataAllocationsExceeded,
            c if c == to_builtin(18) => ProgramError::InvalidRealloc,
            c if c == to_builtin(19) => ProgramError::MaxInstructionTraceLengthExceeded,
            c if c == to_builtin(20) => ProgramError::BuiltinProgramsMustConsumeComputeUnits,
            c if c == to_builtin(21) => ProgramError::InvalidAccountOwner,
            c if c == to_builtin(22) => ProgramError::ArithmeticOverflow,
            c if c == to_builtin(23) => ProgramError::Immutable,
            c if c == to_builtin(24) => ProgramError::IncorrectAuthority,
            other => ProgramError::Custom(other as u32),
        }
    }
}

/// Map a builtin error index to its runtime u64 code.
///
/// The Solana runtime uses a specific encoding for builtin errors:
/// `BUILTIN_BIT_OFFSET + index`. This matches the solana-program-error crate.
#[inline(always)]
const fn to_builtin(index: u64) -> u64 {
    // The Solana runtime encodes builtin errors as:
    //   0x100000000 + index  (for program errors)
    // Custom errors are in the range [0, 0xFFFFFFFF].
    0x1_0000_0000_u64 + index
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
            ProgramError::MaxAccountsDataAllocationsExceeded => write!(f, "MaxAccountsDataAllocationsExceeded"),
            ProgramError::InvalidRealloc => write!(f, "InvalidRealloc"),
            ProgramError::MaxInstructionTraceLengthExceeded => write!(f, "MaxInstructionTraceLengthExceeded"),
            ProgramError::BuiltinProgramsMustConsumeComputeUnits => write!(f, "BuiltinProgramsMustConsumeComputeUnits"),
            ProgramError::InvalidAccountOwner => write!(f, "InvalidAccountOwner"),
            ProgramError::ArithmeticOverflow => write!(f, "ArithmeticOverflow"),
            ProgramError::Immutable => write!(f, "Immutable"),
            ProgramError::IncorrectAuthority => write!(f, "IncorrectAuthority"),
        }
    }
}
