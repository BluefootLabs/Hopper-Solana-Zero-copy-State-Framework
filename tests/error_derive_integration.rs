//! End-to-end coverage for `#[hopper::error]`. Audit parity item: the error
//! model must lower into the program-return channel the same way Anchor's
//! `#[error_code]` does, so a handler can `return Err(MyError::Foo.into())`.

#![cfg(feature = "proc-macros")]

use hopper::__runtime::ProgramError;
use hopper::prelude::*;

#[hopper::error_code]
#[repr(u32)]
pub enum VaultError {
    #[invariant = "balance_nonzero"]
    InsufficientBalance = 0x1001,
    #[invariant = "authority_match"]
    Unauthorized = 0x1002,
    // No explicit discriminant: code is SHA-256 derived and stable.
    MigrationRequired,
}

#[test]
fn explicit_codes_round_trip_through_u32() {
    assert_eq!(u32::from(VaultError::InsufficientBalance), 0x1001);
    assert_eq!(u32::from(VaultError::Unauthorized), 0x1002);
    assert_eq!(VaultError::InsufficientBalance.code(), 0x1001);
}

#[test]
fn derived_code_is_stable_and_nonzero() {
    let code = VaultError::MigrationRequired.code();
    assert_ne!(code, 0);
    // Derived codes stay in the low 31 bits, leaving the high bit for
    // user-explicit codes.
    assert_eq!(code & 0x8000_0000, 0);
    assert_eq!(u32::from(VaultError::MigrationRequired), code);
}

#[test]
fn rust_discriminant_agrees_with_wire_code() {
    // Batch 4 audit fix: auto-derived codes are written back as enum
    // discriminants, so the natural `as u32` cast can never disagree
    // with `code()` / `CODE_TABLE` / `ProgramError::Custom`.
    assert_eq!(
        VaultError::MigrationRequired as u32,
        VaultError::MigrationRequired.code()
    );
    assert_eq!(VaultError::InsufficientBalance as u32, 0x1001);
    assert_eq!(VaultError::Unauthorized as u32, 0x1002);
}

#[test]
fn lowers_into_program_error_custom() {
    let err: ProgramError = VaultError::Unauthorized.into();
    assert_eq!(err, ProgramError::Custom(0x1002));
}

#[test]
fn into_works_in_program_result_position() {
    fn handler(unauthorized: bool) -> Result<(), ProgramError> {
        if unauthorized {
            return Err(VaultError::Unauthorized.into());
        }
        Ok(())
    }
    assert_eq!(handler(true), Err(ProgramError::Custom(0x1002)));
    assert_eq!(handler(false), Ok(()));
}

#[test]
fn invariant_metadata_is_recoverable() {
    assert_eq!(
        VaultError::InsufficientBalance.invariant(),
        "balance_nonzero"
    );
    assert_eq!(VaultError::Unauthorized.invariant(), "authority_match");
    assert_eq!(VaultError::MigrationRequired.invariant(), "");
    assert_eq!(
        VaultError::InsufficientBalance.variant_name(),
        "InsufficientBalance"
    );
}

#[test]
fn registry_tables_track_every_variant() {
    assert_eq!(VaultError::CODE_TABLE.len(), 3);
    assert_eq!(VaultError::INVARIANT_TABLE.len(), 3);
    assert_eq!(VaultError::CODE_TABLE[0], ("InsufficientBalance", 0x1001));
    // `invariant_idx` indexes both tables.
    let idx = VaultError::Unauthorized.invariant_idx() as usize;
    assert_eq!(VaultError::CODE_TABLE[idx].0, "Unauthorized");
    assert_eq!(VaultError::INVARIANT_TABLE[idx].1, "authority_match");
}
