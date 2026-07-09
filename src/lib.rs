//! # Hopper
//!
//! Fast zero-copy Solana framework. Start with the familiar account/context
//! facade, then opt into layout, segment, receipt, policy, migration, and
//! schema modules only when the program needs them. Unsafe is available when
//! you need it, and it is spelled `unsafe` so you can find it again.
//!
//! The goals, in priority order:
//!
//! 1. **Safety by default.** Every account byte you touch has been
//!    owner-checked, signer-checked, layout-checked, and borrow-checked
//!    before you see it. Unsafe is an opt-in escape hatch, never a
//!    default.
//! 2. **Low-overhead performance.** Account data points directly at
//!    the runtime input region. No deserialization pass, no heap
//!    allocation, no hidden format machinery. If it costs compute, it
//!    is because you asked for it.
//! 3. **Framework-grade ergonomics.** `#[hopper::account]`,
//!    `#[hopper::accounts]`, `#[hopper::program]`, typed wrappers such as
//!    `Account<'info, T>` and `Signer<'info>`, and the facade modules under
//!    `hopper::{account, cpi, token, system}` keep the beginner surface small.
//! 4. **Schema that travels.** Every layout, instruction, event, and
//!    error is emitted as inspectable compile-time metadata. Off-chain
//!    SDKs, IDLs, client generators, and diff tools consume it without
//!    parsing source.
//!
//! ## Layers
//!
//! - `hopper::prelude`: beginner and migration path. Same runtime, clean import
//!   story.
//! - `hopper::{account, context, cpi, system, token, associated_token, memo}`:
//!   everyday program modules.
//! - `hopper::{layout, segment, receipt, migration, interface, schema, policy}`:
//!   systems-mode modules for layout evolution, field leasing, manifests, and
//!   audited escape hatches.
//! - `hopper::substrate`: raw Hopper Runtime and Hopper Native building blocks
//!   for programs that want Pinocchio-class control with Hopper's account and
//!   wire contracts still visible.
//! - `hopper::internal`: explicit lower-crate escape hatch.
//!
//! ## Crate map
//!
//! - `hopper_core`: wire types, account header, segment maps, validation,
//!   collections, and opt-in advanced subsystems (frame, receipt, policy,
//!   diff, migration) behind feature gates.
//! - `hopper_runtime`: the runtime surface a program actually calls.
//!   Context, Account, Pod, guards, CPI, migrations, log macros,
//!   entrypoint bridges.
//! - `hopper_macros`: declarative macros. `hopper_layout!`, `hopper_check!`,
//!   `hopper_error!`, `hopper_init!`, `hopper_close!`, `hopper_require!`,
//!   `hopper_register_discs!`, `hopper_verify_pda!`, `hopper_invariant!`,
//!   `hopper_manifest!`, `hopper_segment!`, `hopper_validate!`,
//!   `hopper_virtual!`, `hopper_interface!`, `hopper_accounts!`,
//!   `hopper_assert_compatible!`, `hopper_assert_fingerprint!`, and
//!   `const_assert_pod!`.
//! - `hopper_derive` (`hopper-derive` package): proc-macro DX layer.
//!   `#[hopper::state]`, `#[hopper::pod]`, `#[hopper::context]`,
//!   `#[derive(Accounts)]`, `#[hopper::program]`, `#[hopper::migrate]`,
//!   `#[hopper::args]`, `#[hopper::error_code]`, `#[hopper::event]`,
//!   `#[hopper::constant]`, `#[hopper::crank]`, `hopper::declare_program!`,
//!   `#[hopper::dynamic]`, and `#[hopper::dynamic_account]`.
//! - `hopper_solana`: SPL Token/Mint readers, Token-2022 checks, CPI
//!   guards, Pyth oracle, TWAP, Ed25519/Merkle crypto, authority rotation.
//! - `hopper_system`: System Program instruction builders.
//! - `hopper_token`: SPL Token instruction builders.
//! - `hopper_token_2022`: Token-2022 instruction builders plus extension
//!   screening helpers.
//! - `hopper_associated_token`: ATA derivation and instruction builders.
//! - `hopper_schema`: layout manifests, fingerprinting, field-level
//!   diffing, compatibility verdicts, Codama/IDL projections, client
//!   generation.
//!
//! ## Access model
//!
//! Four reads, one rule: safety first, unsafe by name.
//!
//! ```text
//! Safe full:    account.load::<T>()        / account.load_mut::<T>()
//! Safe segment: ctx.segment_ref::<T>(i,o)  / ctx.segment_mut::<T>(i,o)
//! Explicit raw: unsafe { account.raw_ref() / account.raw_mut() }
//! Cross-prog:   account.load_cross_program::<T>()
//! ```
//!
//! `load` and `segment_ref` are the defaults. `raw_ref` is the escape
//! hatch. `load_cross_program` is the Hopper-specific verb for reading
//! an account owned by another program (foreign-ownership checked at
//! the type level).
//!
//! ## Quick start
//!
//! Declare an account layout, bind it in a context, ship a handler.
//!
//! ```ignore
//! use hopper::prelude::*;
//!
//! #[account(disc = 1, version = 1)]
//! #[repr(C)]
//! #[derive(Clone, Copy)]
//! pub struct Vault {
//!     pub authority: Address,
//!     pub balance: WireU64,
//!     pub bump: u8,
//! }
//! ```
//!
//! `Vault` is now a zero-copy account layout with a Hopper header, layout
//! fingerprint, schema metadata, and typed `Account<'info, Vault>` access. No
//! Borsh pass, no hidden writeback pass.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

// ---------------------------------------------------------------------

#[doc(hidden)]
pub mod __macro_support;
pub mod guards;
pub mod pda;
pub mod prelude;
pub mod receipts;

// Re-export crates
pub use hopper_associated_token;
pub use hopper_core;
pub use hopper_runtime;
pub use hopper_schema;
pub use hopper_solana;
pub use hopper_system;
pub use hopper_token;
pub use hopper_token_2022;

// Program-wide tuned memory intrinsics (opt-in, I18). The anonymous
// `as _` import forces the hopper-builtins rlib onto the linker command
// line even though no item is named, so its `#[no_mangle]`
// memcmp/bcmp/memcpy/memset definitions are pulled from our archive
// before the platform-tools compiler-builtins archive is searched.
// SBF-only: on host targets the crate exports nothing to link.
#[cfg(all(target_os = "solana", feature = "builtins"))]
use hopper_builtins as _;

/// Beginner-facing account surface.
///
/// This module is the framework-first import path for account roles and typed
/// account access. The lower-level crates remain available, but normal program
/// authors should be able to stay inside `hopper::prelude::*` and these facade
/// modules.
pub mod account {
    pub use hopper_core::abi::{
        Authority, Mint, Token, TokenAccount, TypedAddress, UntypedAddress,
    };
    pub use hopper_core::accounts::{
        HopperAccount, HopperAccounts, HopperCtx, HopperIx, ProgramAccount, ProgramRef,
        SegmentedAccount, SignerAccount, ValidateAccount,
    };
    pub use hopper_runtime::{
        Account, AccountView, CompactDynamicLayout, CompactLayout, ExplainExternal,
        ExternalAccount, ExternalBytes, ExternalChecked, ExternalExplainSink, ExternalLens,
        ExternalLensValue, ExternalProof, ExternalResolve, ExternalZeroCopy,
        HopperSigner as Signer, InitAccount, Interface, InterfaceAccount, InterfaceAccountLayout,
        InterfaceAccountResolve, InterfaceSpec, Program, ProgramId, SystemAccount, SystemId,
        UncheckedAccount, COMPACT_BODY_OFFSET,
    };

    /// Anchor-style spelling for the System Program marker.
    pub type System = SystemId;
}

/// Tier 2 of the three-tier metadata model: the on-chain program
/// registry (see `docs/THREE_TIER_METADATA.md`).
pub mod manifest {
    pub use hopper_core::manifest::*;
}

/// Zero-copy collections for on-chain account data, including the `Tail*`
/// views (`TailVec`, `TailRing`, `TailSlab`, `TailBitSet`) and the
/// [`CompactTail`](hopper_core::collections::CompactTail) bridge that overlays
/// them on a compact dynamic account's tail.
///
/// Enable with `--features collections` (the collections crate surface is
/// opt-in to keep the minimal build small).
#[cfg(feature = "collections")]
pub mod collections {
    pub use hopper_core::collections::*;
}

/// Typed instruction context and account-binding helpers.
pub mod context {
    pub use hopper_core::accounts::{hopper_entry, HopperAccounts, HopperCtx, HopperIx};
    pub use hopper_runtime::{Context, ScopedContext};
}

/// Cross-program invocation helpers and generated instruction parts.
pub mod cpi {
    pub use hopper_core::cpi::*;
    pub use hopper_runtime::cpi::{
        invoke, invoke_checked, invoke_signed, invoke_signed_checked, MAX_CPI_ACCOUNTS,
        MAX_STATIC_CPI_ACCOUNTS,
    };
    pub use hopper_runtime::cpi::{
        invoke_signed_unchecked, invoke_signed_with_bounds, invoke_unchecked, invoke_with_bounds,
    };
    pub use hopper_runtime::return_data::{
        get_return_data, set_return_data, try_set_return_data, ReturnData, MAX_RETURN_DATA,
    };
    pub use hopper_runtime::CpiAccount;
    pub use hopper_runtime::{
        InstructionAccount, InstructionView, Seed, Signer, StoredAccountMeta, StoredInstruction,
    };
}

/// On-chain cryptography and precompile-verification helpers.
pub mod crypto {
    pub use hopper_runtime::crypto::*;
    pub use hopper_solana::crypto::{ed25519::*, merkle::*, secp256k1::*};
}

/// Compute-budget helpers.
pub mod compute {
    pub use hopper_runtime::compute::*;
}

/// SVM memory helpers with safe slice wrappers.
pub mod memory {
    pub use hopper_runtime::memory::*;
}

/// CPI return-data helpers.
pub mod return_data {
    pub use hopper_runtime::return_data::*;
}

/// Sysvar access (Clock, Rent, EpochSchedule, SlotHashes, StakeHistory,
/// LastRestartSlot, epoch stake) via direct syscalls — no account passing,
/// no deserialization.
///
/// Framework-mode programs that need on-chain time, rent-exempt minimums, or
/// epoch/slot context import these here (`hopper::sysvar::Clock::get()`,
/// `hopper::sysvar::get_rent()`), instead of descending into the low-level
/// `hopper::substrate::sysvar` substrate path.
pub mod sysvar {
    pub use hopper_runtime::__hopper_native::sysvar::*;

    // Instructions-sysvar introspection lives here too, so a single
    // `hopper::sysvar` import covers both the direct-syscall sysvars
    // (Clock/Rent/EpochRewards/…) and reading the Instructions sysvar
    // account (parity with Pinocchio `Instructions<T>` / Quasar
    // introspection). The richer guard helpers stay in `hopper::systems`.
    pub use hopper_core::check::{
        current_instruction_index, instruction_count, read_program_id_at, InstructionAccountMeta,
        InstructionsSysvar, IntrospectedInstruction,
    };
}

/// System Program helpers.
#[allow(unused_imports)]
pub mod system {
    pub use hopper_system::instructions::*;
    pub use hopper_system::SYSTEM_PROGRAM_ID;
    pub use hopper_system::*;
}

/// SPL Token helpers.
#[allow(ambiguous_glob_reexports, unused_imports)]
pub mod token {
    pub use hopper_runtime::token::*;
    pub use hopper_solana::interface::{
        interface_transfer_checked, interface_transfer_checked_signed, InterfaceMint,
        InterfaceTokenAccount, TokenProgramKind,
    };
    pub use hopper_token::instructions::*;
    pub use hopper_token::TOKEN_PROGRAM_ID;
    pub use hopper_token::*;
}

/// SPL Token-2022 helpers.
#[allow(ambiguous_glob_reexports, unused_imports)]
pub mod token_2022 {
    pub use hopper_runtime::token_2022_ext::*;
    pub use hopper_token_2022::instructions::*;
    pub use hopper_token_2022::TOKEN_2022_PROGRAM_ID;
    pub use hopper_token_2022::*;
}

/// Associated Token Account helpers.
#[allow(unused_imports)]
pub mod associated_token {
    pub use hopper_associated_token::instructions::*;
    pub use hopper_associated_token::ATA_PROGRAM_ID;
    pub use hopper_associated_token::*;
}

/// SPL Memo helper surface.
pub mod memo {
    pub use hopper_memo::*;
}

/// Event and log helpers.
pub mod events {
    pub use crate::receipts::{emit_receipt, emit_tagged_receipt, emit_typed_receipt, Receipt};
    #[cfg(feature = "cpi")]
    pub use hopper_core::event::emit_event_cpi;
    pub use hopper_core::event::{emit_event, emit_event_tagged, emit_slices};
    pub use hopper_runtime::{hopper_emit_cpi, hopper_log, msg};
}

/// Advanced layout and header surface.
pub mod layout {
    pub use hopper_core::abi::{FingerprintTransition, LayoutFingerprint, WireType};
    pub use hopper_core::account::{
        check_header, read_discriminator, read_header_flags, read_layout_id, read_version,
        write_header, AccountHeader, AccountReader, FixedLayout, HEADER_FORMAT, HEADER_LEN,
    };
    pub use hopper_core::field_map::{FieldInfo, FieldMap};
    pub use hopper_runtime::layout::{init_header, HopperHeader, LayoutContract, LayoutInfo};
    pub use hopper_runtime::{AccountLayout, Pod, WireLayout, ZeroCopy};
}

/// Advanced segment and field-lease surface.
pub mod segment {
    pub use hopper_core::account::segment_role::{
        SegmentRole, SEG_ROLE_AUDIT, SEG_ROLE_CACHE, SEG_ROLE_CORE, SEG_ROLE_EXTENSION,
        SEG_ROLE_INDEX, SEG_ROLE_JOURNAL, SEG_ROLE_SHARD,
    };
    pub use hopper_core::account::{
        segment_id, SegmentDescriptor, SegmentEntry, SegmentId, SegmentRegistry,
        SegmentRegistryMut, SegmentSlice, SegmentSliceMut, SegmentTable, SegmentTableMut,
        MAX_REGISTRY_SEGMENTS, MAX_SEGMENTS, REGISTRY_HEADER_SIZE, REGISTRY_OFFSET,
        SEGMENT_DESC_SIZE, SEGMENT_ENTRY_SIZE, SEG_FLAG_DYNAMIC, SEG_FLAG_FROZEN, SEG_FLAG_LOCKED,
    };
    pub use hopper_core::segment_map::{assert_segment_field_alignment, SegmentMap, StaticSegment};
    pub use hopper_runtime::{
        AccessKind, FieldCapability, Ref, RefMut, SegRef, SegRefMut, Segment, SegmentBorrow,
        SegmentBorrowGuard, SegmentBorrowRegistry, SegmentLease, TypedSegment,
        FIELD_POLICY_AUTHORITY_GATED, FIELD_POLICY_CHECKED_MATH, FIELD_POLICY_IMMUTABLE_AFTER_INIT,
        FIELD_ROLE_AUTHORITY, FIELD_ROLE_BALANCE, FIELD_ROLE_DATA, FIELD_ROLE_VERSION,
    };
}

/// Receipt helpers and receipt wire types.
pub mod receipt {
    pub use crate::receipts::{emit_receipt, emit_tagged_receipt, emit_typed_receipt, Receipt};
    #[cfg(feature = "receipt")]
    pub use hopper_core::receipt::*;
}

/// Layout migration helpers.
pub mod migration {
    #[cfg(feature = "migrate")]
    pub use hopper_core::migrate::*;
    pub use hopper_runtime::{apply_pending_migrations, LayoutMigration, MigrationEdge};
}

/// Cross-program layout/interface pinning helpers.
pub mod interface {
    pub use hopper_runtime::{
        ExplainExternal, ExternalAccount, ExternalBytes, ExternalChecked, ExternalExplainSink,
        ExternalLens, ExternalLensValue, ExternalProof, ExternalResolve, ExternalZeroCopy,
        ForeignLens, ForeignManifest, Interface, InterfaceAccount, InterfaceAccountLayout,
        InterfaceAccountResolve, InterfaceSpec, TransparentAddress,
    };
    pub use hopper_solana::interface::*;
}

/// Checked arithmetic and protocol-grade unit wrappers.
pub mod math {
    pub use hopper_core::math::*;
}

/// Schema, manifest, and IDL projection surface.
pub mod schema {
    pub use hopper_schema::*;
}

/// Policy engine surface for systems-mode programs.
pub mod policy {
    #[cfg(feature = "policy")]
    pub use hopper_core::policy::*;
    pub use hopper_runtime::{HopperInstructionPolicy, HopperProgramPolicy, HopperProgramProfile};
}

/// Protocol-grade Hopper surface.
///
/// Import this when a program deliberately reaches beyond framework mode into
/// layout contracts, segment registries, receipts, policies, migrations,
/// manifests, overlays, or cross-program interface pinning.
#[allow(ambiguous_glob_reexports, unused_imports)]
pub mod systems {
    pub use crate::compute::*;
    pub use crate::interface::*;
    pub use crate::layout::*;
    pub use crate::math::*;
    pub use crate::memory::*;
    pub use crate::migration::*;
    pub use crate::policy::*;
    pub use crate::receipt::*;
    pub use crate::return_data::*;
    pub use crate::schema::*;
    pub use crate::segment::*;
    pub use crate::{
        compute, crypto, interface, layout, math, memory, migration, policy, receipt, return_data,
        schema, segment,
    };

    pub use hopper_core::account::{
        overlay, overlay_mut, read_dynamic_u16, read_dynamic_u32, read_dynamic_u8, safe_close,
        safe_close_with_sentinel, safe_realloc, write_dynamic_u16, write_dynamic_u32,
        write_dynamic_u8, DynamicView, DynamicViewMut, ReallocGuard, VerifiedAccount,
        VerifiedAccountMut, CLOSE_SENTINEL,
    };
    pub use hopper_core::check::{find_and_verify_pda, rent_exempt_min};
    pub use hopper_core::prelude_advanced::*;
    pub use hopper_core::prelude_core::*;
    pub use hopper_runtime::CpiAccount;
    pub use hopper_runtime::{
        default_allocator, fast_entrypoint, hopper_entrypoint, hopper_fast_entrypoint,
        hopper_lazy_entrypoint, lazy_entrypoint, no_allocator, nostd_panic_handler,
        program_entrypoint, AccountProof, BoundedString, BoundedVec, ExecutableChecked,
        ExplainExternal, ExternalBytes, ExternalChecked, ExternalExplainSink, ExternalLens,
        ExternalLensValue, ExternalProof, ExternalResolve, HasOneChecked, HopperString, HopperVec,
        InstructionAccount, InstructionView, LayoutChecked, OwnerChecked,
        RemainingExternalAccounts, RemainingGroup, RemainingLazy, RemainingLazySlot, Seed,
        SeedsChecked, SignerChecked, StoredAccountMeta, StoredInstruction, TailCodec, TailElement,
        TokenExtensionsChecked, Unchecked, WritableChecked,
    };

    pub use crate::{
        const_assert_pod, hopper_accounts, hopper_assert_compatible, hopper_assert_fingerprint,
        hopper_check, hopper_close, hopper_dynamic_fields, hopper_dynamic_tail, hopper_error,
        hopper_init, hopper_interface, hopper_invariant, hopper_layout, hopper_load,
        hopper_manifest, hopper_register_discs, hopper_require, hopper_segment, hopper_validate,
        hopper_verify_pda, hopper_virtual, layout_migrations,
    };

    #[cfg(feature = "proc-macros")]
    pub use crate::{args, declare_program, dynamic, migrate, pod, state};
}

/// Sovereign substrate surface.
///
/// This is the lowest public layer for programs that want direct account,
/// syscall, PDA, hashing, memory, budget, and raw-entrypoint tools while still
/// staying inside Hopper's crate graph. Framework-mode code should prefer
/// `hopper::prelude::*`; systems-mode code should prefer `hopper::systems::*`.
#[allow(ambiguous_glob_reexports, unused_imports)]
pub mod substrate {
    pub use crate::return_data as hopper_return_data;
    pub use crate::{compute, crypto, memory};
    pub use hopper_runtime::CpiAccount;
    pub use hopper_runtime::{
        AccountView, Address, InstructionAccount, InstructionView, ProgramError, ProgramResult,
        Ref, RefMut, RemainingLazy, RemainingLazySlot, Seed, Signer, StoredAccountMeta,
        StoredInstruction, SUCCESS,
    };

    pub use hopper_runtime::__hopper_native::{
        account_view, address, batch, budget, capability, entrypoint, error, hash, introspect,
        lazy, lens, log, mem, pda, pod, raw, raw_account, raw_input, return_data, safe, syscalls,
        sysvar, verify, wire, AccountView as NativeAccountView, Address as NativeAddress, CuBudget,
        DataFingerprint, LamportSnapshot, ReturnData,
    };

    pub use hopper_runtime::__hopper_native::{
        find_bump_for_address, read_bump_from_account, verify_pda_from_stored_bump,
        verify_pda_strict, RuntimeAccount,
    };
}

/// First-party DeFi math helpers. Enable with `features = ["finance"]`.
#[cfg(feature = "finance")]
pub mod finance {
    pub use hopper_finance::*;
}

/// First-party lending helpers. Enable with `features = ["lending"]`.
#[cfg(feature = "lending")]
pub mod lending {
    pub use hopper_lending::*;
}

/// First-party staking helpers. Enable with `features = ["staking"]`.
#[cfg(feature = "staking")]
pub mod staking {
    pub use hopper_staking::*;
}

/// First-party vesting helpers. Enable with `features = ["vesting"]`.
#[cfg(feature = "vesting")]
pub mod vesting {
    pub use hopper_vesting::*;
}

/// First-party distribution helpers. Enable with `features = ["distribute"]`.
#[cfg(feature = "distribute")]
pub mod distribute {
    pub use hopper_distribute::*;
}

/// First-party multisig helpers. Enable with `features = ["multisig"]`.
#[cfg(feature = "multisig")]
pub mod multisig {
    pub use hopper_multisig::*;
}

/// Anchor interop helpers. Enable with `features = ["anchor-interop"]`.
#[cfg(feature = "anchor-interop")]
pub mod anchor {
    pub use hopper_anchor::*;
}

/// Explicit escape hatch for lower layers. Normal programs should not need it.
#[doc(hidden)]
pub mod internal {
    pub use crate::{hopper_core, hopper_runtime, hopper_schema, hopper_solana};
}

/// Optional Metaplex Token Metadata builders. Behind `--features metaplex`.
/// Reach for this via `hopper::hopper_metaplex::CreateMetadataAccountV3`,
/// `CreateMasterEditionV3`, `UpdateMetadataAccountV2`, plus the
/// `metadata_pda` / `master_edition_pda` PDA helpers.
#[cfg(feature = "metaplex")]
pub use hopper_metaplex;

/// Small utilities. Re-exported at the crate root so `use hopper::utils::hint::likely;`
/// just works.
pub use hopper_runtime::utils;

// Re-export macros at the crate root
pub use hopper_core::hopper_dispatch;
pub use hopper_macros::{
    const_assert_pod, hopper_accounts, hopper_assert_compatible, hopper_assert_fingerprint,
    hopper_check, hopper_close, hopper_error, hopper_init, hopper_interface, hopper_invariant,
    hopper_layout, hopper_manifest, hopper_register_discs, hopper_require, hopper_segment,
    hopper_validate, hopper_verify_pda, hopper_virtual,
};

// Audit I4: schema-epoch migration chain composition. `#[macro_export]`
// macros are always anchored at the defining crate's root, so the
// user-facing path `hopper::layout_migrations!` requires an explicit
// re-export here.
pub use hopper_runtime::layout_migrations;

/// Destructuring sugar for raw-dispatch handlers.
///
/// Replaces the common pattern:
///
/// ```ignore
/// let [user, vault, system_program, ..] = accounts else {
///     return Err(ProgramError::NotEnoughAccountKeys);
/// };
/// ```
///
/// with the tighter form:
///
/// ```ignore
/// hopper_load!(accounts => [user, vault, system_program]);
/// ```
///
/// The bindings are plain `&AccountView<'_>` references (not owned values),
/// matching the destructuring pattern. A trailing `..` is accepted and
/// discards any extra accounts. The macro bails with
/// `ProgramError::NotEnoughAccountKeys` when the slice is too short,
/// mirroring Hopper's existing idiom.
///
/// Use this in the raw-dispatch authoring style (no `#[hopper::context]`).
/// The proc-macro context already binds accounts by name, so this is only
/// useful when you are working with `&[AccountView<'_>]` directly - typically
/// inside `fn process_instruction(_, accounts: &[AccountView<'_>], _)` before
/// routing to per-variant handlers.
///
/// ## Examples
///
/// ```ignore
/// fn process_deposit(
///     program_id: &Address,
///     accounts: &[AccountView<'_>],
///     data: &[u8],
/// ) -> ProgramResult {
///     hopper_load!(accounts => [user, vault, system_program]);
///     user.require_signer()?;
///     vault.require_writable()?;
///     // ... rest of handler ...
///     Ok(())
/// }
/// ```
///
/// With a trailing rest pattern (accept more accounts, ignore them):
///
/// ```ignore
/// hopper_load!(accounts => [user, vault, ..]);
/// ```
///
/// The trailing `..` is redundant with the default behaviour (the macro
/// always accepts more accounts than declared) but is supported for
/// stylistic parity with the native Rust slice pattern.
#[macro_export]
macro_rules! hopper_load {
    ( $slice:expr => [ $($binding:ident),+ $(, ..)? $(,)? ] ) => {
        let [ $($binding,)+ .. ] = $slice else {
            return ::core::result::Result::Err(
                $crate::hopper_runtime::error::ProgramError::NotEnoughAccountKeys,
            );
        };
    };
}

/// Declare a dynamic-tail payload with bounded dynamic fields.
///
/// The generated type implements `TailCodec`, so it can be used directly with
/// `#[hopper::state(dynamic_tail = MyTail)]`. Fields are encoded in declaration
/// order. Use `HopperString<N>` for bounded UTF-8 strings and
/// `HopperVec<T, N>` for bounded element lists. The longer
/// `BoundedString<N>` and `BoundedVec<T, N>` names remain available when a
/// protocol wants the storage tradeoff to be explicit in source. For
/// Quasar-shaped account syntax, prefer `#[hopper::dynamic_account]`; use this
/// macro when you want an explicit custom `TailCodec` payload.
///
/// ```ignore
/// hopper::hopper_dynamic_tail! {
///     pub struct MultisigTail {
///         label: HopperString<32>,
///         signers: HopperVec<Address, 10>,
///     }
/// }
/// ```
#[macro_export]
macro_rules! hopper_dynamic_tail {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $( $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Default)]
        $vis struct $name {
            $( pub $field: $ty, )*
        }

        impl $crate::__runtime::TailCodec for $name {
            const MAX_ENCODED_LEN: usize = 0 $(+ <$ty as $crate::__runtime::TailCodec>::MAX_ENCODED_LEN)*;

            #[inline]
            fn encode(
                &self,
                out: &mut [u8],
            ) -> ::core::result::Result<usize, $crate::__runtime::ProgramError> {
                let mut cursor = 0usize;
                $(
                    let written = <$ty as $crate::__runtime::TailCodec>::encode(
                        &self.$field,
                        &mut out[cursor..],
                    )?;
                    cursor = cursor
                        .checked_add(written)
                        .ok_or($crate::__runtime::ProgramError::AccountDataTooSmall)?;
                )*
                Ok(cursor)
            }

            #[inline]
            fn decode(
                input: &[u8],
            ) -> ::core::result::Result<(Self, usize), $crate::__runtime::ProgramError> {
                let mut cursor = 0usize;
                $(
                    let ($field, consumed) = <$ty as $crate::__runtime::TailCodec>::decode(
                        &input[cursor..],
                    )?;
                    cursor = cursor
                        .checked_add(consumed)
                        .ok_or($crate::__runtime::ProgramError::InvalidAccountData)?;
                )*
                Ok((Self { $( $field, )* }, cursor))
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __hopper_dynamic_fields_tail {
    (@emit [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]) => {
        $crate::hopper_dynamic_tail! {
            $($meta)*
            $vis struct $name {
                $($fields)*
            }
        }
    };

    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]) => {
        $crate::__hopper_dynamic_fields_tail!(@emit [$($meta)*] [$vis] [$name] [$($fields)*]);
    };
    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*] ,) => {
        $crate::__hopper_dynamic_fields_tail!(@emit [$($meta)*] [$vis] [$name] [$($fields)*]);
    };

    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]
        $field:ident : string < $cap:literal >, $($rest:tt)+
    ) => {
        $crate::__hopper_dynamic_fields_tail!(@parse [$($meta)*] [$vis] [$name]
            [$($fields)* $field: $crate::__runtime::HopperString<$cap>,]
            $($rest)+
        );
    };
    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]
        $field:ident : string < $cap:literal > $(,)?
    ) => {
        $crate::__hopper_dynamic_fields_tail!(@emit [$($meta)*] [$vis] [$name]
            [$($fields)* $field: $crate::__runtime::HopperString<$cap>,]
        );
    };

    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]
        $field:ident : vec < $ty:ty, $cap:literal >, $($rest:tt)+
    ) => {
        $crate::__hopper_dynamic_fields_tail!(@parse [$($meta)*] [$vis] [$name]
            [$($fields)* $field: $crate::__runtime::HopperVec<$ty, $cap>,]
            $($rest)+
        );
    };
    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]
        $field:ident : vec < $ty:ty, $cap:literal > $(,)?
    ) => {
        $crate::__hopper_dynamic_fields_tail!(@emit [$($meta)*] [$vis] [$name]
            [$($fields)* $field: $crate::__runtime::HopperVec<$ty, $cap>,]
        );
    };

    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]
        $field:ident : $ty:ty, $($rest:tt)+
    ) => {
        $crate::__hopper_dynamic_fields_tail!(@parse [$($meta)*] [$vis] [$name]
            [$($fields)* $field: $ty,]
            $($rest)+
        );
    };
    (@parse [$($meta:tt)*] [$vis:vis] [$name:ident] [$($fields:tt)*]
        $field:ident : $ty:ty $(,)?
    ) => {
        $crate::__hopper_dynamic_fields_tail!(@emit [$($meta)*] [$vis] [$name]
            [$($fields)* $field: $ty,]
        );
    };
}

/// Explicit sugar for declaring bounded dynamic-tail fields.
///
/// This is a small front end over `hopper_dynamic_tail!`. It keeps Hopper's
/// explicit fixed-body plus encoded-tail model, while letting ported programs
/// spell custom tail fields as `string<N>` and `vec<T, N>`. For a full account
/// declaration with inline tail fields, use `#[hopper::dynamic_account]`.
///
/// ```ignore
/// hopper::hopper_dynamic_fields! {
///     pub struct MultisigTail {
///         label: string<32>,
///         signers: vec<Address, 10>,
///         nonce: u32,
///     }
/// }
/// ```
#[macro_export]
macro_rules! hopper_dynamic_fields {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($body:tt)*
        }
    ) => {
        $crate::__hopper_dynamic_fields_tail!(@parse [$(#[$meta])*] [$vis] [$name] [] $($body)*);
    };
}

// Ergonomic guard macros. All are `#[macro_export]` from
// hopper_runtime and are re-exported here so programs see them at
// the top-level `hopper::*` path without needing to reach through
// `hopper_runtime::`.
pub use hopper_runtime::{
    address, declare_id, default_allocator, err, error, fast_entrypoint, hopper_emit_cpi,
    hopper_entrypoint, hopper_fast_entrypoint, hopper_lazy_entrypoint, hopper_log,
    hopper_unsafe_region, lazy_entrypoint, msg, no_allocator, nostd_panic_handler,
    program_entrypoint, require, require_eq, require_gt, require_gte, require_keys_eq,
    require_keys_neq, require_lt, require_lte, require_neq,
};

/// Declare a bounded multi-layout interface account resolver.
///
/// Use this when one account slot may hold one of several Hopper-header layouts
/// owned by the same interface program set, such as a migration reader that
/// accepts `VaultV1` or `VaultV2`.
///
/// ```ignore
/// hopper::interface_account_set! {
///     pub struct AnyVaultAccount: VaultPrograms;
///     pub enum AnyVault {
///         V1(VaultV1),
///         V2(VaultV2),
///     }
/// }
/// ```
#[macro_export]
macro_rules! interface_account_set {
    (
        $(#[$marker_meta:meta])*
        $vis:vis struct $marker:ident : $interface:ty;
        $(#[$resolved_meta:meta])*
        $enum_vis:vis enum $resolved:ident {
            $($variant:ident($layout:ty)),+ $(,)?
        }
    ) => {
        $(#[$marker_meta])*
        #[derive(Clone, Copy, Debug, Default)]
        $vis struct $marker;

        impl $crate::__runtime::FieldMap for $marker {
            const FIELDS: &'static [$crate::__runtime::FieldInfo] = &[];
        }

        impl $crate::__runtime::LayoutContract for $marker {
            const DISC: u8 = 0;
            const VERSION: u8 = 0;
            const LAYOUT_ID: [u8; 8] = [0; 8];
            const SIZE: usize = $crate::__runtime::HopperHeader::SIZE;
        }

        impl $crate::__runtime::InterfaceAccountLayout for $marker {
            type Interface = $interface;

            #[inline]
            fn validate_interface_account(
                view: &$crate::__runtime::AccountView<'_>,
            ) -> ::core::result::Result<(), $crate::__runtime::ProgramError> {
                let data = view.try_borrow()?;
                let info = $crate::__runtime::LayoutInfo::from_data(&data)
                    .ok_or($crate::__runtime::ProgramError::AccountDataTooSmall)?;
                if false $(|| info.matches::<$layout>())+ {
                    Ok(())
                } else {
                    Err($crate::__runtime::ProgramError::InvalidAccountData)
                }
            }
        }

        $(#[$resolved_meta])*
        $enum_vis enum $resolved<'a> {
            $($variant($crate::__runtime::Ref<'a, $layout>)),+
        }

        impl $crate::__runtime::InterfaceAccountResolve for $marker {
            type Resolved<'a> = $resolved<'a>;

            #[inline]
            fn resolve<'a>(
                view: &'a $crate::__runtime::AccountView<'a>,
            ) -> ::core::result::Result<Self::Resolved<'a>, $crate::__runtime::ProgramError> {
                let info = {
                    let data = view.try_borrow()?;
                    $crate::__runtime::LayoutInfo::from_data(&data)
                        .ok_or($crate::__runtime::ProgramError::AccountDataTooSmall)?
                };
                $(
                    if info.matches::<$layout>() {
                        return Ok($resolved::$variant(view.load_cross_program::<$layout>()?));
                    }
                )+
                Err($crate::__runtime::ProgramError::InvalidAccountData)
            }
        }
    };
}

/// Generate the small runtime bridge for a `#[program(entrypoint = false)]`
/// module.
///
/// New programs do not need this macro: `#[program]` emits the same audited
/// Hopper entrypoint bridge automatically. Keep `program_dispatch!` for
/// compatibility and for unusual crates that intentionally disable the
/// automatic bridge.
///
/// ```ignore
/// #[program(entrypoint = false)]
/// mod counter_program {
///     // handlers
/// }
///
/// hopper::program_dispatch!(counter_program);
/// ```
#[macro_export]
macro_rules! program_dispatch {
    ($program_mod:ident) => {
        #[cfg(target_os = "solana")]
        $crate::program_entrypoint!(__hopper_process_instruction);

        #[doc(hidden)]
        fn __hopper_process_instruction(
            program_id: &$crate::__runtime::Address,
            accounts: &[$crate::__runtime::AccountView<'_>],
            instruction_data: &[u8],
        ) -> ::core::result::Result<(), $crate::__runtime::ProgramError> {
            let mut ctx = $crate::prelude::Context::new(program_id, accounts, instruction_data);
            $program_mod::process_instruction(&mut ctx)
        }
    };
}

// Optional proc macro re-exports (enabled with `proc-macros` feature)
#[cfg(feature = "proc-macros")]
pub use hopper_macros_proc::{
    account, accounts, args, constant, context, crank, declare_program, dynamic, dynamic_account,
    error as error_code, event, hopper_args, hopper_constant, hopper_context, hopper_crank,
    hopper_dynamic, hopper_dynamic_account, hopper_event, hopper_migrate, hopper_pod,
    hopper_program, hopper_state, migrate, pod, program, state, Accounts, HopperInitSpace,
};

// Private re-export for generated code to reference runtime types
#[doc(hidden)]
pub mod __runtime {
    pub use hopper_runtime::token_2022_ext;
    pub use hopper_runtime::{
        apply_pending_migrations, borrow_address_slice, borrow_bounded_str, read_tail,
        read_tail_len, tail_capacity, tail_payload, transfer_lamports, write_tail,
        write_tail_payload, Account, AccountLayout, AccountView, Address, BoundedString,
        BoundedVec, Context, FieldInfo, FieldMap, HopperHeader, HopperInstructionPolicy,
        HopperProgramPolicy, HopperProgramProfile, HopperSigner, HopperString, HopperVec,
        InitAccount, InstructionAccount, InstructionView, Interface, InterfaceAccount,
        InterfaceAccountLayout, InterfaceAccountResolve, InterfaceSpec, LayoutContract, LayoutInfo,
        LayoutMigration, MigrationEdge, Pod, Program, ProgramError, ProgramId, Ref, RefMut, SegRef,
        SegRefMut, SegmentLease, SystemAccount, SystemId, TailBytes, TailCodec, TailElement,
        TailStr, UncheckedAccount, Zeroable,
    };

    // Crank marker type plus dynamic-CPI builder, emitted by
    // `#[hopper::crank]` and by hand-written programs that need
    // variable-length CPIs respectively. Exposed through this
    // doc-hidden module so user code never reaches into
    // `hopper_runtime::*` directly.
    pub use hopper_runtime::crank::CrankMarker;
    pub use hopper_runtime::dyn_cpi::DynCpi;

    // `#[hopper::state]` and `#[hopper::pod]` emit Hopper-owned
    // zero-copy proofs through this path so user code never needs a
    // direct runtime-internals dependency.
    pub use hopper_runtime::__hopper_native;

    // Audit final-API Step 5 seal. Doc-hidden re-export of the
    // sealed-trait module so macros can emit
    // `unsafe impl ::hopper::__runtime::__sealed::HopperZeroCopySealed for Foo {}`
    // without the user ever naming the private module.
    pub use hopper_runtime::__sealed;

    // Re-export `hopper_runtime::token` so `#[hopper::context]` can
    // emit `::hopper::__runtime::token::require_token_mint(...)`
    // without dragging the user into a direct `hopper_runtime`
    // dependency. The helpers (require_token_mint / require_mint_*
    // / require_token_authority) are read-only account-byte
    // preconditions used to lower Anchor's `token::mint`,
    // `mint::authority`, etc. constraints to a single inline check.
    pub use hopper_runtime::token;

    // Innovation I12: `#[hopper::context(strict_writes)]` emits a
    // `static ::hopper::__runtime::write_policy::WritePolicy` compiled
    // from the context's `mut` / `mut(seg, ...)` declarations and
    // installs it on the raw context during `bind()`.
    pub use hopper_runtime::write_policy;
}
