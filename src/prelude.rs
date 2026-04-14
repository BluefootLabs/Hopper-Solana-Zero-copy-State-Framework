//! One-import path for authored Hopper programs.

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

#[cfg(feature = "proc-macros")]
pub use crate::{context, hopper_context, hopper_program, hopper_state, program, state};

// New systems
#[cfg(feature = "virtual-state")]
pub use hopper_core::virtual_state::{VirtualState, VirtualSlot, ShardedAccess};
#[cfg(feature = "diff")]
pub use hopper_core::diff::{StateSnapshot, StateDiff};
#[cfg(feature = "graph")]
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
#[cfg(feature = "receipt")]
pub use hopper_core::receipt::{StateReceipt, RECEIPT_SIZE};
#[cfg(feature = "policy")]
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
pub use hopper_runtime::hopper_fast_entrypoint;
pub use hopper_runtime::fast_entrypoint;
pub use hopper_runtime::hopper_lazy_entrypoint;
pub use hopper_runtime::lazy_entrypoint;
pub use hopper_runtime::{no_allocator, nostd_panic_handler};
pub use hopper_runtime::{
    AccountView, Address, ProgramResult, Context, LayoutContract,
    InstructionAccount, InstructionView, Seed, Signer,
    TransparentAddress,
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
pub use hopper_system;
pub use hopper_system::instructions as system_instructions;
pub use hopper_system::SYSTEM_PROGRAM_ID;
pub use hopper_token;
pub use hopper_token::instructions as token_instructions;
pub use hopper_token::TOKEN_PROGRAM_ID;
pub use hopper_token_2022;
pub use hopper_token_2022::instructions as token_2022_instructions;
pub use hopper_token_2022::TOKEN_2022_PROGRAM_ID;
pub use hopper_associated_token;
pub use hopper_associated_token::instructions as associated_token_instructions;
pub use hopper_associated_token::ATA_PROGRAM_ID;

// Field maps
pub use hopper_core::field_map::{FieldInfo, FieldMap};
pub use hopper_core::account::HEADER_LEN;
pub use hopper_core::segment_map::{SegmentMap, StaticSegment};
pub use hopper_core::invariant::InvariantSet;

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
