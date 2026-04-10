//! # Hopper Core
//!
//! Core engine for Hopper, a zero-copy state framework for Solana.
//!
//! Typed account architecture, phased execution, composable validation,
//! zero-copy collections, layout evolution with deterministic fingerprints,
//! policy-aware capabilities, state receipts, and cross-program interfaces.
//! `no_std`, `no_alloc`, no proc macros required.
//!
//! ## Architecture
//!
//! - **Account memory**: Fixed, overlay, segmented, and arena layout styles
//! - **Execution**: `Frame`-based borrowed-state execution with phases
//! - **Validation**: Named rule groups, instruction-specific rule packs,
//!   post-mutation invariant checks, composable pipelines
//! - **Policy**: Declare instruction capabilities, auto-resolve validation
//!   requirements via `InstructionPolicy`
//! - **Receipts**: Structured mutation summaries combining snapshots, diffs,
//!   field masks, invariant results, and CPI tracking
//! - **Collections**: Zero-copy `FixedVec`, `RingBuffer`, `SlotMap`, `BitSet`,
//!   `Journal`, `Slab`, `PackedMap`
//! - **Segments**: Typed segment roles (Core, Extension, Journal, Index,
//!   Cache, Audit, Shard) for semantic classification
//! - **Fingerprints**: Deterministic layout_id from SHA-256, compile-time
//!   compatibility assertions, schema diffing
//! - **Evolution**: Append-only versioned layouts with migration helpers
//! - **Interfaces**: Cross-program read-only views with ABI proof
//!
//! Built on hopper-native. Compatible with jiminy account layouts.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod abi;
pub mod account;
pub mod accounts;
pub mod check;
pub mod collections;
pub mod cpi;
pub mod diff;
pub mod dispatch;
pub mod event;
pub mod field_map;
pub mod frame;
pub mod invariant;
pub mod math;
pub mod migrate;
pub mod policy;
pub mod receipt;
pub mod state;
pub mod sysvar;
pub mod time;
pub mod virtual_state;

pub use field_map::*;

// -- Internal helpers (used by macros, not public API) ------------------------

/// Hidden re-export of hopper_runtime for macro hygiene.
/// Allows `$crate::__runtime` to resolve in macro expansions without
/// requiring the caller to have a direct `hopper_runtime` dependency.
#[doc(hidden)]
pub use hopper_runtime as __runtime;

/// Const SHA-256 helper for `hopper_layout!` layout ID generation.
#[doc(hidden)]
pub const fn __sha256_const(data: &[u8]) -> [u8; 32] {
    sha2_const_stable::Sha256::new().update(data).finalize()
}

/// Const string equality helper for BUMP_OFFSET field scanning.
#[doc(hidden)]
pub const fn __str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Public const string equality (alias of `__str_eq` for internal crate use).
#[doc(hidden)]
pub const fn const_str_eq(a: &str, b: &str) -> bool {
    __str_eq(a, b)
}

/// Prelude re-exports for ergonomic usage.
pub mod prelude {
    pub use crate::abi::*;
    pub use crate::abi::{LayoutFingerprint, FingerprintTransition};
    pub use crate::abi::{
        TypedAddress, UntypedAddress,
        Authority, Mint, TokenAccount, Token, Program,
    };
    pub use crate::account::{
        AccountHeader, AccountReader, FixedLayout, Pod, VerifiedAccount, VerifiedAccountMut,
        ReallocGuard, CLOSE_SENTINEL, HEADER_LEN,
        zero_init, write_header, read_layout_id,
        pod_from_bytes, pod_from_bytes_mut, pod_read, pod_write,
        cast_unchecked, cast_unchecked_mut,
    };
    pub use crate::accounts::{
        HopperCtx, HopperAccounts, HopperAccount,
        ProgramAccount, SignerAccount, UncheckedAccount,
        MigratingAccount, SegmentedAccount, ProgramRef,
        HopperIx, hopper_entry,
        ValidateAccount, ExplainAccount,
        AccountMetaProvider,
    };
    pub use crate::check::{
        check_account, check_discriminator, check_has_one, check_keys_eq,
        check_owner, check_owner_multi,
        check_rent_exempt, check_signer, check_size, check_writable, rent_exempt_min,
        verify_pda, verify_pda_cached, find_and_verify_pda,
        require_top_level, detect_flash_loan_bracket, check_no_subsequent_invocation,
        keys_eq_fast, is_zero_address,
    };
    pub use crate::check::fast::{
        check_account_fast, check_signer_fast, check_writable_fast,
        check_authority_fast, check_executable_fast,
        HEADER_SIGNER, HEADER_WRITABLE, HEADER_SIGNER_WRITABLE, HEADER_EXECUTABLE,
    };
    pub use crate::collections::{BitSet, FixedVec, PackedMap, RingBuffer, SlotMap, SortedVec};
    pub use crate::collections::journal::{Journal, JournalReader};
    pub use crate::collections::slab::Slab;
    pub use crate::cpi::{HopperCpi, HopperCpiBuf};
    pub use crate::diff::{StateSnapshot, StateDiff};
    pub use crate::dispatch::dispatch_instruction;
    pub use crate::event::{emit_event, emit_event_tagged, emit_slices};
    pub use crate::field_map::{FieldInfo, FieldMap};
    pub use crate::frame::{Frame, FrameAccount, FrameAccountMut};
    pub use crate::frame::phase::{
        PhasedFrame, ResolvedFrame, ValidatedFrame, ExecutionContext,
    };
    pub use crate::frame::args::{InstructionArgs, ValidateArgs};
    pub use crate::invariant::{check_invariant, check_invariant_fn, InvariantSet};
    pub use crate::math::{
        checked_add, checked_div, checked_mul, checked_sub,
        checked_mul_div, checked_mul_div_ceil, checked_div_ceil,
        bps_of, bps_of_ceil, scale_bps, scale_fraction,
        scale_amount, scale_amount_ceil,
        checked_pow, to_u64, div_ceil,
    };
    pub use crate::migrate::{migrate_append, MigrationKind};
    pub use crate::state::check_state_transition;
    pub use crate::time::{check_cooldown_elapsed, check_deadline_not_passed, check_staleness};
    pub use crate::sysvar::{CachedClock, CachedRent, SysvarContext};
    pub use crate::virtual_state::{VirtualState, VirtualSlot, ShardedAccess};
    pub use crate::check::modifier::{
        Account, AccountMut, Signer, Mut,
        FromAccount, HasView, HopperLayout,
    };
    pub use crate::check::graph::{
        ValidationGraph, ValidationContext, AccountConstraint, TransactionConstraint,
        ValidationGroup, ValidationBundle, Validatable,
        PostMutationValidator, TransitionRulePack,
        require_signer_at, require_writable_at, require_owned_at, require_data_min,
        require_keys_equal, require_unique, require_lamports_gte,
    };
    pub use crate::check::guards::{
        require_payer, require_authority, require_owned_writable,
        require_all_unique, check_lamport_conservation, snapshot_lamports,
        check_writable_coherence,
    };
    pub use crate::check::trust::{
        TrustProfile, TrustLevel, TrustFlags, load_foreign_with_profile,
    };
    pub use crate::account::{
        SegmentRegistry, SegmentRegistryMut, SegmentEntry, SegmentId, segment_id,
    };
    pub use crate::account::segment_role::{
        SegmentRole,
        SEG_ROLE_CORE, SEG_ROLE_EXTENSION, SEG_ROLE_JOURNAL,
        SEG_ROLE_INDEX, SEG_ROLE_CACHE, SEG_ROLE_AUDIT, SEG_ROLE_SHARD,
    };
    pub use crate::receipt::{
        StateReceipt, DecodedReceipt, ReceiptExplain, RECEIPT_SIZE,
        Phase, CompatImpact,
    };
    pub use crate::policy::{
        Capability, CapabilitySet, PolicyRequirement, RequirementSet,
        InstructionPolicy,
        // Named policy packs
        TREASURY_WRITE_POLICY, TREASURY_WRITE_CAPS,
        JOURNAL_TOUCH_POLICY, JOURNAL_TOUCH_CAPS,
        EXTERNAL_CALL_POLICY, EXTERNAL_CALL_CAPS,
        SHARD_MUTATION_POLICY, SHARD_MUTATION_CAPS,
        MIGRATION_SENSITIVE_POLICY, MIGRATION_SENSITIVE_CAPS,
        AUTHORITY_CHANGE_POLICY, AUTHORITY_CHANGE_CAPS,
        READ_ONLY_AUDIT_POLICY, READ_ONLY_AUDIT_CAPS,
        ACCOUNT_INIT_POLICY, ACCOUNT_INIT_CAPS,
        ACCOUNT_CLOSE_POLICY, ACCOUNT_CLOSE_CAPS,
        NAMED_POLICY_PACKS, PolicyPackDescriptor,
    };
}
