//! Regression tests for the Jiminy-style guard macros
//! (`require!`, `require_eq!`, `require_neq!`, `require_keys_eq!`,
//! `require_keys_neq!`, `require_gte!`, `require_gt!`).
//!
//! These macros are the ergonomic safety surface the "winning
//! architecture" design calls for. They compile to a single branch
//! with no heap, no formatting, no panic path.

use hopper::__runtime::{Address, ProgramError};

fn check_signer_like(is_signer: bool) -> Result<(), ProgramError> {
    hopper::require!(is_signer, ProgramError::MissingRequiredSignature);
    Ok(())
}

fn check_amount_positive(amount: u64) -> Result<(), ProgramError> {
    hopper::require!(amount > 0, ProgramError::InvalidArgument);
    Ok(())
}

fn check_short_form(cond: bool) -> Result<(), ProgramError> {
    hopper::require!(cond);
    Ok(())
}

fn check_eq<T: PartialEq>(a: T, b: T) -> Result<(), ProgramError> {
    hopper::require_eq!(a, b, ProgramError::InvalidAccountData);
    Ok(())
}

fn check_neq<T: PartialEq>(a: T, b: T) -> Result<(), ProgramError> {
    hopper::require_neq!(a, b, ProgramError::InvalidAccountData);
    Ok(())
}

fn check_keys_eq(a: Address, b: Address) -> Result<(), ProgramError> {
    hopper::require_keys_eq!(a, b, ProgramError::InvalidAccountData);
    Ok(())
}

fn check_keys_neq(a: Address, b: Address) -> Result<(), ProgramError> {
    hopper::require_keys_neq!(a, b, ProgramError::InvalidAccountData);
    Ok(())
}

fn check_gte(have: u64, need: u64) -> Result<(), ProgramError> {
    hopper::require_gte!(have, need, ProgramError::InsufficientFunds);
    Ok(())
}

fn check_gt(have: u64, need: u64) -> Result<(), ProgramError> {
    hopper::require_gt!(have, need, ProgramError::InsufficientFunds);
    Ok(())
}

#[test]
fn require_passes_when_cond_true() {
    assert!(check_signer_like(true).is_ok());
}

#[test]
fn require_errors_when_cond_false() {
    let err = check_signer_like(false).unwrap_err();
    assert!(matches!(err, ProgramError::MissingRequiredSignature));
}

#[test]
fn require_amount_positive_rejects_zero() {
    assert!(check_amount_positive(0).is_err());
    assert!(check_amount_positive(1).is_ok());
}

#[test]
fn require_short_form_returns_invalid_argument() {
    let err = check_short_form(false).unwrap_err();
    assert!(matches!(err, ProgramError::InvalidArgument));
}

#[test]
fn require_eq_passes_when_equal() {
    assert!(check_eq(7u64, 7u64).is_ok());
}

#[test]
fn require_eq_errors_when_not_equal() {
    let err = check_eq(7u64, 8u64).unwrap_err();
    assert!(matches!(err, ProgramError::InvalidAccountData));
}

#[test]
fn require_neq_errors_when_equal() {
    let err = check_neq(7u64, 7u64).unwrap_err();
    assert!(matches!(err, ProgramError::InvalidAccountData));
}

#[test]
fn require_neq_passes_when_different() {
    assert!(check_neq(1u64, 2u64).is_ok());
}

#[test]
fn require_keys_eq_passes_when_same_bytes() {
    let a = Address::new_from_array([1u8; 32]);
    let b = Address::new_from_array([1u8; 32]);
    assert!(check_keys_eq(a, b).is_ok());
}

#[test]
fn require_keys_eq_errors_when_different_bytes() {
    let a = Address::new_from_array([1u8; 32]);
    let b = Address::new_from_array([2u8; 32]);
    let err = check_keys_eq(a, b).unwrap_err();
    assert!(matches!(err, ProgramError::InvalidAccountData));
}

#[test]
fn require_keys_neq_errors_when_same_bytes() {
    let a = Address::new_from_array([5u8; 32]);
    let b = Address::new_from_array([5u8; 32]);
    let err = check_keys_neq(a, b).unwrap_err();
    assert!(matches!(err, ProgramError::InvalidAccountData));
}

#[test]
fn require_keys_neq_passes_when_different_bytes() {
    let a = Address::new_from_array([5u8; 32]);
    let b = Address::new_from_array([6u8; 32]);
    assert!(check_keys_neq(a, b).is_ok());
}

#[test]
fn require_gte_passes_when_greater_or_equal() {
    assert!(check_gte(100u64, 50u64).is_ok());
    assert!(check_gte(50u64, 50u64).is_ok());
}

#[test]
fn require_gte_errors_when_less() {
    let err = check_gte(10u64, 50u64).unwrap_err();
    assert!(matches!(err, ProgramError::InsufficientFunds));
}

#[test]
fn require_gt_rejects_equal_values() {
    let err = check_gt(50u64, 50u64).unwrap_err();
    assert!(matches!(err, ProgramError::InsufficientFunds));
}

#[test]
fn require_gt_passes_when_strictly_greater() {
    assert!(check_gt(51u64, 50u64).is_ok());
}
