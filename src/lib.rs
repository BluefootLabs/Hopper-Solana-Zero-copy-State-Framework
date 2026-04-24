//! # Hopper
//!
//! A zero-copy Solana program framework. One access model, one dispatch
//! path, one set of safety rules. Unsafe is available when you need it,
//! and it is spelled `unsafe` so you can find it again.
//!
//! The goals, in priority order:
//!
//! 1. **Safety by default.** Every account byte you touch has been
//!    owner-checked, signer-checked, layout-checked, and borrow-checked
//!    before you see it. Unsafe is an opt-in escape hatch, never a
//!    default.
//! 2. **Pinocchio-class performance.** Account data points directly at
//!    the runtime input region. No deserialization pass, no heap
//!    allocation, no hidden format machinery. If it costs compute, it
//!    is because you asked for it.
//! 3. **Anchor-grade ergonomics.** `#[hopper::state]`, `#[hopper::context]`,
//!    `#[hopper::program]`, and the `#[account(...)]` constraint vocabulary
//!    read the same way an Anchor program reads. Porting is a rename,
//!    not a rewrite.
//! 4. **Schema that travels.** Every layout, instruction, event, and
//!    error is emitted as inspectable compile-time metadata. Off-chain
//!    SDKs, IDLs, client generators, and diff tools consume it without
//!    parsing source.
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
//!   `hopper_manifest!`, `hopper_segment!`, `hopper_validate!`,
//!   `hopper_virtual!`, `hopper_interface!`, `hopper_assert_compatible!`,
//!   `hopper_assert_fingerprint!`.
//! - `hopper_macros_proc`: proc-macro DX layer. `#[hopper::state]`,
//!   `#[hopper::pod]`, `#[hopper::context]`, `#[hopper::program]`,
//!   `#[hopper::migrate]`, `#[hopper::args]`, `#[hopper::error]`,
//!   `#[hopper::event]`, `#[hopper::dynamic]`.
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
//! Declare a layout, declare a context, ship a handler.
//!
//! ```ignore
//! use hopper::prelude::*;
//! use hopper::hopper_layout;
//!
//! hopper_layout! {
//!     pub struct Vault, disc = 1, version = 1 {
//!         authority: [u8; 32] = 32,
//!         mint:      [u8; 32] = 32,
//!         balance:   WireU64  = 8,
//!         bump:      u8       = 1,
//!     }
//! }
//! ```
//!
//! That is it. `Vault` is now a zero-copy layout with a 16-byte Hopper
//! header, a segment map, a schema export for the manifest, and a
//! `load::<Vault>()` accessor on every `AccountView`. No derives, no
//! Borsh, no writeback pass.

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

/// Small utilities. Re-exported at the crate root so `use hopper::utils::hint::likely;`
/// just works.
pub use hopper_runtime::utils;

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
/// The bindings are plain `&AccountView` references (not owned values),
/// matching the destructuring pattern. A trailing `..` is accepted and
/// discards any extra accounts. The macro bails with
/// `ProgramError::NotEnoughAccountKeys` when the slice is too short,
/// mirroring Hopper's existing idiom.
///
/// Use this in the raw-dispatch authoring style (no `#[hopper::context]`).
/// The proc-macro context already binds accounts by name, so this is only
/// useful when you are working with `&[AccountView]` directly — typically
/// inside `fn process_instruction(_, accounts: &[AccountView], _)` before
/// routing to per-variant handlers.
///
/// ## Examples
///
/// ```ignore
/// fn process_deposit(
///     program_id: &Address,
///     accounts: &[AccountView],
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

// Ergonomic guard macros (the "winning architecture" design's
// Jiminy-replacement safety layer). All are `#[macro_export]` from
// hopper_runtime and are re-exported here so programs see them at
// the top-level `hopper::*` path without needing to reach through
// `hopper_runtime::`.
pub use hopper_runtime::{
    address, err, error, hopper_emit_cpi, hopper_log, hopper_unsafe_region, msg, require,
    require_eq, require_gt, require_gte, require_keys_eq, require_keys_neq, require_lt,
    require_lte, require_neq,
};

// Optional proc macro re-exports (enabled with `proc-macros` feature)
#[cfg(feature = "proc-macros")]
pub use hopper_macros_proc::{
    account,
    accounts,
    context,
    crank,
    declare_program,
    hopper_context,
    hopper_crank,
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

    // Crank marker type plus dynamic-CPI builder, emitted by
    // `#[hopper::crank]` and by hand-written programs that need
    // variable-length CPIs respectively. Exposed through this
    // doc-hidden module so user code never reaches into
    // `hopper_runtime::*` directly.
    pub use hopper_runtime::crank::CrankMarker;
    pub use hopper_runtime::dyn_cpi::DynCpi;

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

    // Re-export `hopper_runtime::token` so `#[hopper::context]` can
    // emit `::hopper::__runtime::token::require_token_mint(...)`
    // without dragging the user into a direct `hopper_runtime`
    // dependency. The helpers (require_token_mint / require_mint_*
    // / require_token_authority) are read-only account-byte
    // preconditions used to lower Anchor's `token::mint`,
    // `mint::authority`, etc. constraints to a single inline check.
    pub use hopper_runtime::token;
}

