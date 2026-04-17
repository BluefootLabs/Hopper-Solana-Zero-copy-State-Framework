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
//!
//! ## Feature flags
//!
//! `hopper-core` ships one hot-path core plus opt-in advanced subsystems.
//! The default feature set is `programs`, `hopper-native-backend`, `cpi`,
//! `collections`, and the `advanced` umbrella (`frame`, `receipt`, `policy`,
//! `graph`, `migrate`, `virtual-state`, `diff`, `explain`). Programs that
//! only touch raw fields and segments can disable every optional surface:
//!
//! ```toml
//! hopper-core = { version = "0.1", default-features = false,
//!                 features = ["programs", "hopper-native-backend", "cpi"] }
//! ```
//!
//! That lean configuration drops `frame`, `receipt`, `policy`, `graph`,
//! `migrate`, `virtual-state`, `diff`, `explain`, and `collections` from the
//! compile surface and leaves only the hot-path access model, validation
//! primitives, ABI, layout metadata, and CPI. Re-enable features individually
//! as the program grows.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(test)]
extern crate std;

pub mod abi;
pub mod account;
pub mod accounts;
pub mod check;
#[cfg(feature = "collections")]
pub mod collections;
pub mod cpi;
pub mod dispatch;
pub mod event;
pub mod field_map;
pub mod invariant;
pub mod math;
pub mod segment_map;
pub mod state;
pub mod sysvar;
pub mod time;

// ── Advanced subsystems (feature-gated) ──────────────────────────
// These modules are real differentiators but sit outside the hot-path
// access model. Gating them lets lean programs compile only what they
// use, and communicates one clear core identity.
#[cfg(feature = "diff")]
pub mod diff;
#[cfg(feature = "frame")]
pub mod frame;
#[cfg(feature = "migrate")]
pub mod migrate;
#[cfg(feature = "policy")]
pub mod policy;
#[cfg(feature = "receipt")]
pub mod receipt;
#[cfg(feature = "virtual-state")]
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

/// Compute an Anchor-compatible 8-byte discriminator at compile time.
///
/// Anchor discriminators are `sha256("global:{instruction_name}")[0..8]`.
/// This function enables Hopper programs to interoperate with Anchor IDLs
/// and Quasar programs that use the same discriminator scheme.
///
/// ```ignore
/// const INIT_DISC: [u8; 8] = hopper_core::anchor_discriminator("initialize");
/// ```
pub const fn anchor_discriminator(instruction_name: &str) -> [u8; 8] {
    let hash = sha2_const_stable::Sha256::new()
        .update(b"global:")
        .update(instruction_name.as_bytes())
        .finalize();
    [
        hash[0], hash[1], hash[2], hash[3],
        hash[4], hash[5], hash[6], hash[7],
    ]
}

/// Compute an Anchor-compatible 8-byte account discriminator at compile time.
///
/// Account discriminators are `sha256("account:{TypeName}")[0..8]`.
pub const fn anchor_account_discriminator(type_name: &str) -> [u8; 8] {
    let hash = sha2_const_stable::Sha256::new()
        .update(b"account:")
        .update(type_name.as_bytes())
        .finalize();
    [
        hash[0], hash[1], hash[2], hash[3],
        hash[4], hash[5], hash[6], hash[7],
    ]
}

/// Narrow, hot-path-only prelude.
///
/// The finish-line audit demanded that Hopper's "core identity" stay
/// tight: **memory + access + layout**. Everything else — frame-based
/// execution, receipts, policies, validation graphs, migrations, virtual
/// state, diffing, explain — is opt-in power, not launch identity.
///
/// This prelude ships only the types and helpers a Hopper program needs
/// to declare state, bind accounts, check invariants, and make CPIs.
/// For the full surface (historical compatibility), use
/// [`prelude`](crate::prelude) which glob-imports this and then adds the
/// advanced subsystems on top.
pub mod prelude_core {
    // ── ABI primitives: typed addresses, role tags ──────────────────
    pub use crate::abi::{
        Authority, LayoutFingerprint, Mint, Program, Token, TokenAccount,
        TypedAddress, UntypedAddress,
    };

    // ── Account memory: headers, overlays, pod casts ────────────────
    pub use crate::account::{
        cast_unchecked, cast_unchecked_mut, pod_from_bytes, pod_from_bytes_mut, pod_read,
        pod_write, read_layout_id, write_header, zero_init, AccountHeader, AccountReader,
        FixedLayout, Pod, ReallocGuard, VerifiedAccount, VerifiedAccountMut, CLOSE_SENTINEL,
        HEADER_LEN,
    };

    // ── Account wrappers: typed instruction parameters ──────────────
    pub use crate::accounts::{
        hopper_entry, HopperAccount, HopperAccounts, HopperCtx, HopperIx, ProgramAccount,
        ProgramRef, SegmentedAccount, SignerAccount, UncheckedAccount, ValidateAccount,
        AccountMetaProvider,
    };

    // ── Checks: signer, owner, PDA, discriminator ───────────────────
    pub use crate::check::{
        check_account, check_discriminator, check_has_one, check_keys_eq, check_owner,
        check_owner_multi, check_rent_exempt, check_signer, check_size, check_writable,
        find_and_verify_pda, is_zero_address, keys_eq_fast, rent_exempt_min, verify_pda,
        verify_pda_cached,
    };
    pub use crate::check::fast::{
        check_account_fast, check_authority_fast, check_executable_fast, check_signer_fast,
        check_writable_fast, HEADER_EXECUTABLE, HEADER_SIGNER, HEADER_SIGNER_WRITABLE,
        HEADER_WRITABLE,
    };
    pub use crate::check::modifier::{
        Account, AccountMut, FromAccount, HasView, HopperLayout, Mut, Signer,
    };

    // ── Dispatch and events: program plumbing ───────────────────────
    pub use crate::dispatch::{
        dispatch_instruction, dispatch_instruction_8, dispatch_instruction_u16,
        EVENT_CPI_PREFIX,
    };
    pub use crate::event::{emit_event, emit_event_tagged, emit_slices};
    #[cfg(feature = "cpi")]
    pub use crate::event::emit_event_cpi;

    // ── Field + segment metadata (compile-time layout truth) ────────
    pub use crate::field_map::{FieldInfo, FieldMap};
    pub use crate::segment_map::{assert_segment_field_alignment, SegmentMap, StaticSegment};
    pub use hopper_runtime::Segment;
    pub use hopper_runtime::segment_borrow::{
        AccessKind, SegmentBorrow, SegmentBorrowRegistry,
    };

    // ── CPI plumbing ────────────────────────────────────────────────
    pub use crate::cpi::{HopperCpi, HopperCpiBuf};

    // ── Math + time + sysvar: everyday program helpers ──────────────
    pub use crate::math::{
        bps_of, bps_of_ceil, checked_add, checked_div, checked_div_ceil, checked_mul,
        checked_mul_div, checked_mul_div_ceil, checked_pow, checked_sub, div_ceil,
        scale_amount, scale_amount_ceil, scale_bps, scale_fraction, to_u64,
    };
    pub use crate::sysvar::{CachedClock, CachedRent, SysvarContext};
    pub use crate::time::{check_cooldown_elapsed, check_deadline_not_passed, check_staleness};
    pub use crate::state::check_state_transition;
    pub use crate::invariant::{check_invariant, check_invariant_fn, InvariantSet};

    // ── On-chain segment metadata (for segmented accounts) ──────────
    pub use crate::account::{
        segment_id, SegmentEntry, SegmentId, SegmentRegistry, SegmentRegistryMut,
    };
    pub use crate::account::segment_role::{
        SegmentRole, SEG_ROLE_AUDIT, SEG_ROLE_CACHE, SEG_ROLE_CORE, SEG_ROLE_EXTENSION,
        SEG_ROLE_INDEX, SEG_ROLE_JOURNAL, SEG_ROLE_SHARD,
    };

    // ── Anchor-compatible discriminators ────────────────────────────
    pub use crate::{anchor_account_discriminator, anchor_discriminator};

    // ── Collections: zero-copy containers (default-on feature) ──────
    #[cfg(feature = "collections")]
    pub use crate::collections::{BitSet, FixedVec, PackedMap, RingBuffer, SlotMap, SortedVec};
    #[cfg(feature = "collections")]
    pub use crate::collections::journal::{Journal, JournalReader};
    #[cfg(feature = "collections")]
    pub use crate::collections::slab::Slab;
}

/// Advanced subsystem prelude: everything outside the core identity.
///
/// Re-exports the feature-gated surfaces — frame, receipts, policies,
/// validation graphs, migrations, virtual state, diffs, explain,
/// additional check helpers, trust profiles. Each item respects the
/// feature flag that controls its module; disable the feature and the
/// item silently disappears from this prelude, keeping lean programs
/// compiling against [`prelude_core`] alone.
pub mod prelude_advanced {
    // Composite check guards (payer, authority, lamport conservation, …)
    pub use crate::check::guards::{
        check_lamport_conservation, check_writable_coherence, require_all_unique,
        require_authority, require_owned_writable, require_payer, require_unique_signers,
        require_unique_writable, snapshot_lamports,
    };
    pub use crate::check::trust::{
        load_foreign_with_profile, TrustFlags, TrustLevel, TrustProfile,
    };
    pub use crate::check::{
        check_no_subsequent_invocation, detect_flash_loan_bracket, require_top_level,
    };

    #[cfg(feature = "diff")]
    pub use crate::diff::{StateDiff, StateSnapshot};

    #[cfg(feature = "explain")]
    pub use crate::accounts::{AccountExplain, ContextExplain, ExplainAccount};

    #[cfg(feature = "migrate")]
    pub use crate::accounts::MigratingAccount;

    #[cfg(feature = "frame")]
    pub use crate::frame::{Frame, FrameAccount, FrameAccountMut};
    #[cfg(feature = "frame")]
    pub use crate::frame::args::{InstructionArgs, ValidateArgs};
    #[cfg(feature = "frame")]
    pub use crate::frame::phase::{
        ExecutionContext, PhasedFrame, ResolvedFrame, ValidatedFrame,
    };

    #[cfg(feature = "graph")]
    pub use crate::check::graph::{
        require_all_unique_accounts, require_data_min, require_keys_equal, require_lamports_gte,
        require_owned_at, require_signer_at, require_unique, require_unique_signer_accounts,
        require_unique_writable_accounts, require_writable_at, AccountConstraint,
        PostMutationValidator, TransactionConstraint, TransitionRulePack, Validatable,
        ValidationBundle, ValidationContext, ValidationGraph, ValidationGroup,
    };

    #[cfg(feature = "migrate")]
    pub use crate::migrate::{migrate_append, MigrationKind};

    #[cfg(feature = "policy")]
    pub use crate::policy::{
        ACCOUNT_CLOSE_CAPS, ACCOUNT_CLOSE_POLICY, ACCOUNT_INIT_CAPS, ACCOUNT_INIT_POLICY,
        AUTHORITY_CHANGE_CAPS, AUTHORITY_CHANGE_POLICY, Capability, CapabilitySet,
        EXTERNAL_CALL_CAPS, EXTERNAL_CALL_POLICY, InstructionPolicy, JOURNAL_TOUCH_CAPS,
        JOURNAL_TOUCH_POLICY, MIGRATION_SENSITIVE_CAPS, MIGRATION_SENSITIVE_POLICY,
        NAMED_POLICY_PACKS, PolicyPackDescriptor, PolicyRequirement, READ_ONLY_AUDIT_CAPS,
        READ_ONLY_AUDIT_POLICY, RequirementSet, SHARD_MUTATION_CAPS, SHARD_MUTATION_POLICY,
        TREASURY_WRITE_CAPS, TREASURY_WRITE_POLICY,
    };

    #[cfg(feature = "receipt")]
    pub use crate::receipt::{
        CompatImpact, DecodedReceipt, Phase, ReceiptExplain, StateReceipt, RECEIPT_SIZE,
    };

    #[cfg(feature = "virtual-state")]
    pub use crate::virtual_state::{ShardedAccess, VirtualSlot, VirtualState};
}

/// Prelude re-exports for ergonomic usage.
///
/// Backwards-compatible: re-exports both [`prelude_core`] and
/// [`prelude_advanced`] so existing `use hopper::prelude::*;` code keeps
/// compiling. New code that wants the lean surface should reach for
/// [`prelude_core`] directly; feature-gated builds can rely on it
/// alone once the advanced subsystems are turned off.
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
        SegmentedAccount, ProgramRef,
        HopperIx, hopper_entry,
        ValidateAccount,
        AccountMetaProvider,
    };
    #[cfg(feature = "migrate")]
    pub use crate::accounts::MigratingAccount;
    #[cfg(feature = "explain")]
    pub use crate::accounts::{ExplainAccount, ContextExplain, AccountExplain};
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
    #[cfg(feature = "collections")]
    pub use crate::collections::{BitSet, FixedVec, PackedMap, RingBuffer, SlotMap, SortedVec};
    #[cfg(feature = "collections")]
    pub use crate::collections::journal::{Journal, JournalReader};
    #[cfg(feature = "collections")]
    pub use crate::collections::slab::Slab;
    pub use crate::cpi::{HopperCpi, HopperCpiBuf};
    #[cfg(feature = "diff")]
    pub use crate::diff::{StateSnapshot, StateDiff};
    pub use crate::dispatch::{dispatch_instruction, dispatch_instruction_u16, dispatch_instruction_8, EVENT_CPI_PREFIX};
    pub use crate::event::{emit_event, emit_event_tagged, emit_slices};
    #[cfg(feature = "cpi")]
    pub use crate::event::emit_event_cpi;
    pub use crate::field_map::{FieldInfo, FieldMap};
    pub use crate::anchor_discriminator;
    pub use crate::anchor_account_discriminator;
    #[cfg(feature = "frame")]
    pub use crate::frame::{Frame, FrameAccount, FrameAccountMut};
    pub use crate::segment_map::{SegmentMap, StaticSegment, assert_segment_field_alignment};
    pub use hopper_runtime::segment_borrow::{
        AccessKind, SegmentBorrow, SegmentBorrowRegistry,
    };
    #[cfg(feature = "frame")]
    pub use crate::frame::phase::{
        PhasedFrame, ResolvedFrame, ValidatedFrame, ExecutionContext,
    };
    #[cfg(feature = "frame")]
    pub use crate::frame::args::{InstructionArgs, ValidateArgs};
    pub use crate::invariant::{check_invariant, check_invariant_fn, InvariantSet};
    pub use crate::math::{
        checked_add, checked_div, checked_mul, checked_sub,
        checked_mul_div, checked_mul_div_ceil, checked_div_ceil,
        bps_of, bps_of_ceil, scale_bps, scale_fraction,
        scale_amount, scale_amount_ceil,
        checked_pow, to_u64, div_ceil,
    };
    #[cfg(feature = "migrate")]
    pub use crate::migrate::{migrate_append, MigrationKind};
    pub use crate::state::check_state_transition;
    pub use crate::time::{check_cooldown_elapsed, check_deadline_not_passed, check_staleness};
    pub use crate::sysvar::{CachedClock, CachedRent, SysvarContext};
    #[cfg(feature = "virtual-state")]
    pub use crate::virtual_state::{VirtualState, VirtualSlot, ShardedAccess};
    pub use crate::check::modifier::{
        Account, AccountMut, Signer, Mut,
        FromAccount, HasView, HopperLayout,
    };
    #[cfg(feature = "graph")]
    pub use crate::check::graph::{
        ValidationGraph, ValidationContext, AccountConstraint, TransactionConstraint,
        ValidationGroup, ValidationBundle, Validatable,
        PostMutationValidator, TransitionRulePack,
        require_signer_at, require_writable_at, require_owned_at, require_data_min,
        require_keys_equal, require_unique, require_all_unique_accounts,
        require_unique_writable_accounts, require_unique_signer_accounts,
        require_lamports_gte,
    };
    pub use crate::check::guards::{
        require_payer, require_authority, require_owned_writable,
        require_all_unique, require_unique_writable, require_unique_signers,
        check_lamport_conservation, snapshot_lamports,
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
    #[cfg(feature = "receipt")]
    pub use crate::receipt::{
        StateReceipt, DecodedReceipt, ReceiptExplain, RECEIPT_SIZE,
        Phase, CompatImpact,
    };
    #[cfg(feature = "policy")]
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
