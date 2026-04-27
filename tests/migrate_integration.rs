//! End-to-end integration test for the `#[hopper::migrate]` +
//! `layout_migrations!` chain.
//!
//! Closes audit innovation I4 verification: author two in-place
//! migration edges, compose them into a layout's `MIGRATIONS` slice,
//! and assert that the edges are correctly registered + the migrator
//! bodies behave deterministically.
//!
//! Full runtime-account integration with `apply_pending_migrations`
//! needs a mocked `AccountView`, which lives in the existing
//! `hopper-core::tests::trust_tests` harness. This test exists to
//! prove the **macro-side chain composition** links correctly
//! against the runtime side's `LayoutMigration` trait.

#![cfg(feature = "proc-macros")]

use hopper::__runtime::{LayoutMigration, MigrationEdge, ProgramError};

#[hopper::migrate(from = 1, to = 2)]
pub fn counter_v1_to_v2(body: &mut [u8]) -> Result<(), ProgramError> {
    if body.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&body[0..8]);
    let new = u64::from_le_bytes(bytes).wrapping_add(100);
    body[0..8].copy_from_slice(&new.to_le_bytes());
    Ok(())
}

#[hopper::migrate(from = 2, to = 3)]
pub fn counter_v2_to_v3(body: &mut [u8]) -> Result<(), ProgramError> {
    if body.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&body[0..8]);
    let new = u64::from_le_bytes(bytes) ^ 0xAAAA_AAAA_AAAA_AAAA;
    body[0..8].copy_from_slice(&new.to_le_bytes());
    Ok(())
}

/// Lightweight marker type used only to drive the
/// `layout_migrations!` invocation. we don't need a full Hopper
/// `#[state]` layout to prove the composition works, since
/// `LayoutMigration`'s only requirement is `Self: Sized` (via the
/// default blanket). The `MIGRATIONS` slice is the deliverable.
pub struct CounterMigChain;

hopper::layout_migrations! {
    CounterMigChain = [COUNTER_V1_TO_V2_EDGE, COUNTER_V2_TO_V3_EDGE],
}

#[test]
fn edge_constants_carry_correct_epochs() {
    assert_eq!(COUNTER_V1_TO_V2_EDGE.from_epoch, 1);
    assert_eq!(COUNTER_V1_TO_V2_EDGE.to_epoch, 2);
    assert_eq!(COUNTER_V2_TO_V3_EDGE.from_epoch, 2);
    assert_eq!(COUNTER_V2_TO_V3_EDGE.to_epoch, 3);
}

#[test]
fn edge_constants_are_forward_migrations() {
    assert!(COUNTER_V1_TO_V2_EDGE.is_forward());
    assert!(COUNTER_V2_TO_V3_EDGE.is_forward());
}

#[test]
fn layout_migrations_macro_registers_chain() {
    let chain: &[MigrationEdge] = <CounterMigChain as LayoutMigration>::MIGRATIONS;
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].from_epoch, 1);
    assert_eq!(chain[0].to_epoch, 2);
    assert_eq!(chain[1].from_epoch, 2);
    assert_eq!(chain[1].to_epoch, 3);
}

#[test]
fn migration_chain_has_continuity() {
    // The most important invariant for in-place migration: each
    // edge's to_epoch must equal the next edge's from_epoch, and the
    // sequence must be strictly monotonic.
    let chain: &[MigrationEdge] = <CounterMigChain as LayoutMigration>::MIGRATIONS;
    for pair in chain.windows(2) {
        assert_eq!(pair[0].to_epoch, pair[1].from_epoch);
        assert!(pair[0].from_epoch < pair[1].from_epoch);
    }
}

#[test]
fn v1_to_v2_migrator_adds_100_in_place() {
    let mut body = 50u64.to_le_bytes();
    counter_v1_to_v2(&mut body).unwrap();
    assert_eq!(u64::from_le_bytes(body), 150);
}

#[test]
fn v2_to_v3_migrator_xors_with_constant_deterministically() {
    let input = 0x1111_1111_1111_1111u64;
    let mut body = input.to_le_bytes();
    counter_v2_to_v3(&mut body).unwrap();
    assert_eq!(u64::from_le_bytes(body), input ^ 0xAAAA_AAAA_AAAA_AAAA);
}

#[test]
fn migrators_reject_too_short_body() {
    let mut short = [0u8; 4];
    assert!(counter_v1_to_v2(&mut short).is_err());
    assert!(counter_v2_to_v3(&mut short).is_err());
}

#[test]
fn applying_edges_in_order_produces_final_state() {
    // Chain both edges manually and confirm the final body matches
    // the algebraic composition: ((input + 100) ^ 0xAA..AA).
    let input = 42u64;
    let mut body = input.to_le_bytes();
    counter_v1_to_v2(&mut body).unwrap();
    counter_v2_to_v3(&mut body).unwrap();
    let expected = (input.wrapping_add(100)) ^ 0xAAAA_AAAA_AAAA_AAAA;
    assert_eq!(u64::from_le_bytes(body), expected);
}
