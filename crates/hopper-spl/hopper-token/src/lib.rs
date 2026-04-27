//! Hopper-owned SPL Token builder surface.
//!
//! Thin first-class Hopper wrappers over the canonical runtime builders.
//! This crate gives Hopper a native token CPI surface instead of forcing
//! authored programs to depend on external helper crates.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[allow(deprecated)]
pub use hopper_runtime::token::{
    instructions,
    Approve,
    ApproveChecked,
    Burn,
    BurnChecked,
    CloseAccount,
    InitializeAccount,
    MintTo,
    MintToChecked,
    Revoke,
    Transfer,
    TransferChecked,
    TOKEN_PROGRAM_ID,
};