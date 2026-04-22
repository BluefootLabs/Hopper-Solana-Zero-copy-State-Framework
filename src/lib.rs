//! # Hopper
//!
//! Zero-copy state framework for Solana.
//!
//! One access model. Explicit unsafe escape hatches. Optional advanced
//! guarantees. Compile-time generated ergonomics. Inspectable output.
//!
//! ## Crates
//!
//! - `hopper_core`: ABI types, account header, segment maps, validation,
//!   collections, and optional advanced subsystems (frame, receipt, policy,
//!   diff, migration) behind feature gates
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
//! ## Access Model
//!
//! ```text
//! Safe full:    account.load::<T>()        / account.load_mut::<T>()
//! Safe segment: ctx.segment_ref::<T>(i,o)  / ctx.segment_mut::<T>(i,o)
//! Explicit raw: unsafe { account.raw_ref() / account.raw_mut() }
//! Cross-prog:   account.load_cross_program::<T>()
//! ```
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

// Audit I4: schema-epoch migration chain composition. `#[macro_export]`
// macros are always anchored at the defining crate's root, so the
// user-facing path `hopper::layout_migrations!` requires an explicit
// re-export here.
pub use hopper_runtime::layout_migrations;

// Ergonomic guard macros (the "winning architecture" design's
// Jiminy-replacement safety layer). All are `#[macro_export]` from
// hopper_runtime and are re-exported here so programs see them at
// the top-level `hopper::*` path without needing to reach through
// `hopper_runtime::`.
pub use hopper_runtime::{
    address, require, require_eq, require_gt, require_gte, require_keys_eq,
    require_keys_neq, require_neq,
};

// Optional proc macro re-exports (enabled with `proc-macros` feature)
#[cfg(feature = "proc-macros")]
pub use hopper_macros_proc::{
    context,
    hopper_context,
    hopper_migrate,
    hopper_pod,
    hopper_program,
    hopper_state,
    migrate,
    pod,
    program,
    state,
};

// Private re-export for generated code to reference runtime types
#[doc(hidden)]
pub mod __runtime {
    pub use hopper_runtime::{
        apply_pending_migrations, read_tail, read_tail_len, tail_payload, write_tail,
        Account, AccountLayout, AccountView, Address, Context, HopperInstructionPolicy,
        HopperProgramPolicy, HopperSigner, InitAccount, LayoutMigration, MigrationEdge, Pod,
        Program, ProgramError, ProgramId, Ref, RefMut, SegRef, SegRefMut, SegmentLease,
        SystemId, TailCodec,
    };

    // `#[hopper::state]` and `#[hopper::pod]` emit bytemuck derives
    // through this path so user code never needs a direct bytemuck
    // dependency. Gated on the native backend because that's where
    // the bytemuck re-export lives.
    #[cfg(feature = "hopper-native-backend")]
    pub use hopper_runtime::__hopper_native;

    // Audit final-API Step 5 seal. Doc-hidden re-export of the
    // sealed-trait module so macros can emit
    // `unsafe impl ::hopper::__runtime::__sealed::HopperZeroCopySealed for Foo {}`
    // without the user ever naming the private module.
    pub use hopper_runtime::__sealed;
}

