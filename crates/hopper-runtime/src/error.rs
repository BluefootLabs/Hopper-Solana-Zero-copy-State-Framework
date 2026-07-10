//! Hopper-owned program error type for Solana on-chain programs.
//!
//! Each variant maps to a fixed u64 error code returned to the Solana runtime.

/// Errors that a Solana program can return.
///
/// This is part of the Hopper runtime type surface. Variant discriminants
/// match the Solana runtime ABI.
///
/// `#[repr(u64)]` with EXPLICIT sequential discriminants is load-bearing
/// for binary size, exactly as on `hopper_native::error::ProgramError`
/// (which this type mirrors variant-for-variant): fieldless variant `k`
/// encodes to the runtime code `(k + 1) << 32`, so the `u64` lowering is
/// one tag read + one shift instead of a 25-arm match inlined into every
/// entrypoint, and the native<->runtime glue converts by ROUND-TRIPPING
/// the u64 code instead of two more 25-arm identity matches. The DWARF
/// size attribution measured the match forms at 880 bytes — 13% of the
/// parity vault's `.text`. Append new variants with the next sequential
/// discriminant, mirrored in the native enum; the exhaustive tests below
/// refuse to compile otherwise.
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

/// Map a builtin error index to its runtime u64 code.
const BUILTIN_BIT_SHIFT: usize = 32;
const CUSTOM_ZERO: u64 = 1_u64 << BUILTIN_BIT_SHIFT;

const BUILTIN_LOW_MASK: u64 = (1_u64 << BUILTIN_BIT_SHIFT) - 1;

/// Reference builtin encoding, kept ONLY as the test oracle: the
/// shipped conversion reads the enum tag directly, and the golden
/// tests re-derive every variant's code through this original
/// arithmetic to pin the two forever equal.
#[cfg(test)]
#[inline(always)]
const fn to_builtin(index: u64) -> u64 {
    (index + 2) << BUILTIN_BIT_SHIFT
}

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
                // Variant k encodes to (k + 1) << 32 — the old per-arm
                // to_builtin(k - 1) table without the 25-arm match.
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

// The two enums are LAYOUT-IDENTICAL twins: both `#[repr(u64)]` with the
// same variant set, the same explicit discriminants `0..=25`, and the
// same single `u32` payload on `Custom`. That makes the glue an identity
// BY LAYOUT — a transmute, zero instructions — where a 25-arm identity
// match used to sit in every binary (measured: 880 B of the parity
// vault's debug .text) and a u64 round-trip cost +3..+8 CU on benched
// rows. The correspondence is pinned three ways: the const asserts
// below, the exhaustive `glue_roundtrips_identity_for_every_variant`
// test, and each enum's own exhaustive u64 golden test.
const _: () = assert!(
    core::mem::size_of::<ProgramError>()
        == core::mem::size_of::<hopper_native::error::ProgramError>()
);
const _: () = assert!(
    core::mem::align_of::<ProgramError>()
        == core::mem::align_of::<hopper_native::error::ProgramError>()
);

impl From<hopper_native::error::ProgramError> for ProgramError {
    #[inline(always)]
    fn from(e: hopper_native::error::ProgramError) -> Self {
        // SAFETY: both enums are `#[repr(u64)]` with identical variant
        // sets, identical explicit discriminants (0..=25) and the same
        // `Custom(u32)` payload, so every valid bit pattern of one is a
        // valid bit pattern of the other with the same meaning (RFC 2195
        // layout). Size/align equality is const-asserted above and the
        // variant-for-variant identity is exhaustively tested.
        unsafe { core::mem::transmute::<hopper_native::error::ProgramError, ProgramError>(e) }
    }
}

impl From<ProgramError> for hopper_native::error::ProgramError {
    #[inline(always)]
    fn from(e: ProgramError) -> Self {
        // SAFETY: mirror of the impl above; same layout-twin argument.
        unsafe { core::mem::transmute::<ProgramError, hopper_native::error::ProgramError>(e) }
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

#[cfg(test)]
mod tag_encoding_tests {
    use super::*;

    fn all_variants() -> [ProgramError; 29] {
        [
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
        ]
    }

    /// Every variant's u64 code, pinned against the pre-tag-read reference
    /// table (the old 25-arm match, reproduced verbatim). The reference
    /// match is EXHAUSTIVE on purpose: adding a variant without updating
    /// this test — and giving it the next sequential discriminant — must
    /// not compile.
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
        for err in all_variants() {
            let want = reference(&err);
            let got: u64 = err.clone().into();
            assert_eq!(got, want, "u64 code diverged for {err:?}");
            assert_eq!(ProgramError::from(want), err, "roundtrip for {err:?}");
        }
    }

    /// The native<->runtime glue is an IDENTITY on every variant in both
    /// directions (it round-trips through the shared u64 wire code).
    #[test]
    fn glue_roundtrips_identity_for_every_variant() {
        for err in all_variants() {
            let native: hopper_native::error::ProgramError = err.clone().into();
            assert_eq!(
                u64::from(native.clone()),
                u64::from(err.clone()),
                "wire code diverged crossing into native for {err:?}"
            );
            let back: ProgramError = native.into();
            assert_eq!(back, err, "native->runtime glue not identity for {err:?}");
        }
    }
}
