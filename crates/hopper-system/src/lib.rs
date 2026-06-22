//! Hopper-owned System Program builder surface.
//!
//! Thin first-class Hopper wrappers over the canonical runtime builders.
//! This crate exists so authored Hopper programs can depend on Hopper-native
//! program helper crates instead of reaching through backend-specific packages.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub use hopper_runtime::system::{
    instructions, AdvanceNonceAccount, Allocate, AllocateWithSeed, Assign, AssignWithSeed,
    AuthorizeNonceAccount, CreateAccount, CreateAccountWithSeed, InitializeNonceAccount, NonceState,
    Transfer, TransferWithSeed, UpgradeNonceAccount, WithdrawNonceAccount, MAX_SEED_LEN,
    NONCE_ACCOUNT_LEN, NONCE_STATE_INITIALIZED, NONCE_VERSION_CURRENT, RECENT_BLOCKHASHES_ID,
    RENT_SYSVAR_ID, SYSTEM_PROGRAM_ID,
};
