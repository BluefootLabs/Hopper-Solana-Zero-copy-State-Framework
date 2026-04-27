//! Hopper-owned System Program builder surface.
//!
//! Thin first-class Hopper wrappers over the canonical runtime builders.
//! This crate exists so authored Hopper programs can depend on Hopper-native
//! program helper crates instead of reaching through backend-specific packages.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub use hopper_runtime::system::{
    instructions, Allocate, Assign, CreateAccount, Transfer, SYSTEM_PROGRAM_ID,
};
