//! # Hopper
//!
//! Zero-copy state framework for Solana.
//!
//! Typed account architecture, phased execution, composable validation,
//! zero-copy collections, deterministic layout fingerprints, and cross-program
//! interfaces. Built on Hopper Native. `no_std`, `no_alloc`, no proc macros required.
//!
//! ## Crates
//!
//! - `hopper_core`: ABI types, account header, overlay, tiered loading,
//!   checks, collections, Frame, lifecycle, fingerprints, policy, receipts,
//!   segments, virtual state
//! - `hopper_macros`: `hopper_layout!`, `hopper_check!`, `hopper_error!`,
//!   `hopper_init!`, `hopper_close!`, `hopper_require!`, `hopper_manifest!`,
//!   `hopper_segment!`, `hopper_validate!`, `hopper_virtual!`,
//!   `hopper_interface!`, `hopper_assert_compatible!`,
//!   `hopper_assert_fingerprint!`
//! - `hopper_solana`: SPL Token/Mint readers, Token-2022 checks, CPI guards,
//!   Pyth oracle, TWAP, Ed25519/Merkle crypto, authority rotation
//! - `hopper_system`: Hopper-owned System Program instruction builders
//! - `hopper_token`: Hopper-owned SPL Token instruction builders
//! - `hopper_token_2022`: Hopper-owned Token-2022 instruction builders and screening helpers
//! - `hopper_associated_token`: Hopper-owned ATA derivation helpers and ATA instruction builders
//! - `hopper_schema`: Layout manifests, field-level schema diffing,
//!   compatibility checks, Codama/IDL projections, client generation
//!
//! ## Quick Start
//!
//! ```ignore
//! use hopper::prelude::*;
//! use hopper::hopper_layout;
//!
//! hopper_layout! {
//!     pub struct Vault, disc = 1, version = 1 {
//!         authority: [u8; 32]  = 32,
//!         mint:      [u8; 32]  = 32,
//!         balance:   WireU64   = 8,
//!         bump:      u8        = 1,
//!     }
//! }
//! ```

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

// ── Hopper Lang modules ──────────────────────────────────────────────

pub mod guards;
pub mod prelude;
pub mod receipts;
pub mod pda;
#[doc(hidden)]
pub mod __macro_support;

// Re-export crates
pub use hopper_core;
pub use hopper_solana;
pub use hopper_schema;
pub use hopper_system;
pub use hopper_token;
pub use hopper_token_2022;
pub use hopper_associated_token;
pub use hopper_runtime;

// Re-export macros at the crate root
pub use hopper_macros::{
    hopper_layout,
    hopper_check,
    hopper_error,
    hopper_require,
    hopper_init,
    hopper_close,
    hopper_register_discs,
    hopper_verify_pda,
    hopper_invariant,
    hopper_manifest,
    hopper_segment,
    hopper_validate,
    hopper_virtual,
    hopper_assert_compatible,
    hopper_assert_fingerprint,
    hopper_interface,
    hopper_accounts,
};
pub use hopper_core::hopper_dispatch;

// Optional proc macro re-exports (enabled with `proc-macros` feature)
#[cfg(feature = "proc-macros")]
pub use hopper_macros_proc::{
    context,
    hopper_context,
    hopper_program,
    hopper_state,
    program,
    state,
};

// Private re-export for generated code to reference runtime types
#[doc(hidden)]
pub mod __runtime {
    pub use hopper_runtime::{Context, ProgramError, Ref, RefMut};
}

