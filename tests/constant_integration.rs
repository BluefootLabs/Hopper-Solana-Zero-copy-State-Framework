//! End-to-end smoke test for `#[hopper::constant]` + Anchor IDL emission.
//!
//! Verifies:
//!   1. the original `pub const` survives unchanged and stays usable,
//!   2. a sibling `__HOPPER_CONST_<NAME>: ConstantDescriptor` is emitted,
//!   3. the descriptor captures name / type / value / docs correctly,
//!   4. `AnchorIdlWithConstants` surfaces the descriptor in the IDL JSON.

#![cfg(feature = "proc-macros")]

use hopper::hopper_schema::{anchor_idl::AnchorIdlWithConstants, ConstantDescriptor, ProgramIdl};

/// Maximum lamports per deposit.
#[hopper::constant]
pub const MAX_DEPOSIT: u64 = 1_000_000;

#[hopper::constant]
pub const FEE_BPS: u16 = 30;

const PROGRAM_CONSTANTS: &[ConstantDescriptor] =
    &[__HOPPER_CONST_MAX_DEPOSIT, __HOPPER_CONST_FEE_BPS];

#[test]
fn constant_value_preserved() {
    // The original `pub const` is still usable as a real constant.
    assert_eq!(MAX_DEPOSIT, 1_000_000);
    assert_eq!(FEE_BPS, 30);
}

#[test]
fn descriptor_captures_metadata() {
    assert_eq!(__HOPPER_CONST_MAX_DEPOSIT.name, "MAX_DEPOSIT");
    assert_eq!(__HOPPER_CONST_MAX_DEPOSIT.ty, "u64");
    assert_eq!(__HOPPER_CONST_MAX_DEPOSIT.value, "1_000_000");
    assert!(__HOPPER_CONST_MAX_DEPOSIT
        .docs
        .contains("Maximum lamports per deposit"));

    assert_eq!(__HOPPER_CONST_FEE_BPS.name, "FEE_BPS");
    assert_eq!(__HOPPER_CONST_FEE_BPS.ty, "u16");
    assert_eq!(__HOPPER_CONST_FEE_BPS.value, "30");
}

#[test]
fn idl_emitter_renders_constants_array() {
    let idl = ProgramIdl::empty();
    let json = format!(
        "{}",
        AnchorIdlWithConstants {
            idl: &idl,
            constants: PROGRAM_CONSTANTS,
        }
    );

    assert!(json.contains("\"constants\": ["));
    assert!(json.contains("\"name\": \"MAX_DEPOSIT\""));
    assert!(json.contains("\"type\": \"u64\""));
    assert!(json.contains("\"value\": \"1_000_000\""));
    assert!(json.contains("\"name\": \"FEE_BPS\""));
}

#[test]
fn idl_emitter_empty_constants_array_on_empty_slice() {
    let idl = ProgramIdl::empty();
    let json = format!(
        "{}",
        AnchorIdlWithConstants {
            idl: &idl,
            constants: &[],
        }
    );
    assert!(json.contains("\"constants\": [],"));
}
