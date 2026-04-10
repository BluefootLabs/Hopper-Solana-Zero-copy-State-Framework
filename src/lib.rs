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
pub mod receipts;
pub mod pda;

// Re-export crates
pub use hopper_core;
pub use hopper_solana;
pub use hopper_schema;
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

/// Prelude: re-exports the most commonly used items for ergonomic usage.
pub mod prelude {
    // Core prelude
    pub use hopper_core::prelude::*;

    // ABI wire types (explicit for discoverability)
    pub use hopper_core::abi::{
        WireU16, WireU32, WireU64, WireU128,
        WireI16, WireI32, WireI64, WireI128,
        WireBool,
    };

    // Macros
    pub use crate::{
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
        hopper_interface,
        hopper_accounts,
    };

    // New systems
    pub use hopper_core::virtual_state::{VirtualState, VirtualSlot, ShardedAccess};
    pub use hopper_core::diff::{StateSnapshot, StateDiff};
    pub use hopper_core::check::graph::{
        ValidationGraph, ValidationContext, AccountConstraint, TransactionConstraint,
    };
    pub use hopper_core::account::{
        SegmentRegistry, SegmentRegistryMut, SegmentEntry, SegmentId, segment_id,
    };
    pub use hopper_core::account::segment_role::{
        SegmentRole,
        SEG_ROLE_CORE, SEG_ROLE_EXTENSION, SEG_ROLE_JOURNAL,
        SEG_ROLE_INDEX, SEG_ROLE_CACHE, SEG_ROLE_AUDIT, SEG_ROLE_SHARD,
    };
    pub use hopper_core::receipt::{StateReceipt, RECEIPT_SIZE};
    pub use hopper_core::policy::{
        Capability, CapabilitySet, PolicyRequirement, RequirementSet,
        InstructionPolicy,
    };
    pub use hopper_core::collections::journal::{Journal, JournalReader};
    pub use hopper_core::collections::slab::Slab;

    // Runtime essentials
    pub use hopper_runtime::error::ProgramError;
    pub use hopper_runtime::program_entrypoint;
    pub use hopper_runtime::hopper_entrypoint;
    pub use hopper_runtime::{
        AccountView, Address, ProgramResult, Context, LayoutContract,
        InstructionAccount, InstructionView, Seed, Signer,
    };
    pub use hopper_runtime::layout::{
        read_disc, read_version, read_layout_id, write_header, init_header,
        HopperHeader, LayoutInfo,
    };
    pub use hopper_runtime::cpi::{
        invoke as cpi_invoke,
        invoke_signed as cpi_invoke_signed,
        set_return_data as cpi_set_return_data,
    };

    // Field maps
    pub use hopper_core::field_map::{FieldInfo, FieldMap};

    #[cfg(target_os = "solana")]
    pub use crate::pda::{
        create_program_address,
        find_program_address,
        verify_pda,
        verify_pda_with_bump,
    };

    // Hopper Lang guards
    pub use crate::guards::{
        require, require_eq, require_neq, require_gte, require_gt,
        require_signer, require_writable, require_payer, require_owner,
        require_address, require_keys_eq, require_keys_neq,
        require_disc, require_version, require_layout, require_has_data, require_data_len,
        require_unique_2, require_unique_3,
    };

    // Receipts
    pub use crate::receipts::{
        emit_receipt, emit_tagged_receipt, set_return_data,
        emit_typed_receipt, Receipt,
    };
}
