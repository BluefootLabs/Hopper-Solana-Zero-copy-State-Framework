//! Hopper Runtime -- canonical semantic runtime surface.
//!
//! Hopper Runtime owns the public rules, validation, typed loading, CPI
//! semantics, and execution context that authored Hopper code targets.
//! Hopper Native owns the raw execution boundary.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
// The backend `AccountView` is `Copy` only under the `copy` feature. The
// `.clone()` in our manual `Clone` impl is required in the default lane, so we
// silence `clone_on_copy` only in the lane where the type is actually `Copy`.
#![cfg_attr(feature = "copy", allow(clippy::clone_on_copy))]

#[cfg(any(test, feature = "thread-local-registry"))]
extern crate std;

#[doc(hidden)]
pub mod native_boundary;

pub mod account;
pub mod account_wrappers;
pub mod address;
pub mod audit;
pub mod behavior;
pub mod borrow;
pub(crate) mod borrow_registry;
pub mod compact;
pub mod compute;
pub mod cpi;
pub mod cpi_event;
pub mod crank;
pub mod crypto;
pub mod dyn_cpi;
pub mod error;
pub mod field_map;
pub mod foreign;
pub mod interop;
pub mod lamports;
pub mod log;
pub mod memory;
pub mod migrate;
pub mod pod;
pub mod policy;
pub mod proof;
pub mod ref_only;
pub mod result;
pub mod segment;
pub mod tail;
pub mod utils;
pub mod zerocopy;
// Re-export the sealed marker module at the crate root so macro
// codegen can address it as `::hopper_runtime::__sealed::...`. It's
// doc-hidden because it's the audit's Step 5 enforcement surface,
// not a normal-user-facing API.
#[doc(hidden)]
pub use zerocopy::__sealed;
pub mod context;
pub mod instruction;
pub mod layout;
pub mod option_byte;
pub mod pda;
pub mod remaining;
pub mod rent;
pub mod return_data;
pub mod segment_borrow;
pub mod segment_lease;
pub mod syscall;
pub mod syscalls;
pub mod system;
pub mod token;
pub mod token_2022_ext;
pub mod write_policy;

pub use account::AccountView;
pub use account_wrappers::{
    Account, InitAccount, Interface, InterfaceAccount, InterfaceAccountLayout,
    InterfaceAccountResolve, InterfaceSpec, Program, ProgramId, Signer as HopperSigner,
    SystemAccount, SystemId, UncheckedAccount,
};
pub use address::Address;
pub use audit::{AccountAudit, DuplicateAccount};
pub use behavior::{BehaviorChecked, BehaviorWrite, HopperBehavior};
pub use borrow::{Ref, RefMut};
pub use compact::{CompactDynamicLayout, CompactLayout, COMPACT_BODY_OFFSET};
pub use compute::{check_compute_units, remaining_compute_units, require_compute_units};
pub use context::{Context, ScopedContext};
pub use cpi::{invoke, invoke_checked, invoke_signed, invoke_signed_checked};
#[cfg(feature = "crypto-big-mod-exp")]
pub use crypto::big_mod_exp;
#[cfg(feature = "crypto-bn254")]
pub use crypto::{
    alt_bn128_add, alt_bn128_g1_addition_be, alt_bn128_g1_compress_be, alt_bn128_g1_decompress_be,
    alt_bn128_g1_multiplication_be, alt_bn128_g2_compress_be, alt_bn128_g2_decompress_be,
    alt_bn128_mul, alt_bn128_pairing, alt_bn128_pairing_be,
};
pub use crypto::{
    blake3, blake3_single, keccak256, keccak256_single, recover_ethereum_address,
    secp256k1_recover, sha256, sha256_single,
};
#[cfg(feature = "crypto-curve")]
pub use crypto::{curve_group_add, curve_group_mul, curve_group_sub, curve_multiscalar_mul};
#[cfg(feature = "crypto-poseidon")]
pub use crypto::{poseidon_bn254_x5, poseidon_hash, poseidon_hashv};
pub use error::ProgramError;
pub use field_map::{FieldInfo, FieldMap};
pub use foreign::{
    ExplainExternal, ExternalAccount, ExternalBytes, ExternalChecked, ExternalExplainSink,
    ExternalLens, ExternalLensValue, ExternalProof, ExternalResolve, ExternalZeroCopy, ForeignLens,
    ForeignManifest,
};
pub use interop::TransparentAddress;
pub use lamports::transfer_lamports;
pub use migrate::{apply_pending_migrations, migrate_layout, LayoutMigration, MigrationEdge};
pub use policy::{HopperInstructionPolicy, HopperProgramPolicy, HopperProgramProfile};
pub use proof::{
    AccountProof, ExecutableChecked, HasOneChecked, LayoutChecked, OwnerChecked, SeedsChecked,
    SignerChecked, TokenExtensionsChecked, Unchecked, WritableChecked,
};
pub use ref_only::HopperRefOnly;
pub use remaining::{
    RemainingAccountViews, RemainingAccounts, RemainingError, RemainingExternalAccounts,
    RemainingGroup, RemainingLazy, RemainingLazySlot, RemainingMode, RemainingSigners,
    RemainingTyped, MAX_REMAINING_ACCOUNTS,
};
pub use return_data::{get_return_data, set_return_data, try_set_return_data, ReturnData};
pub use tail::{
    borrow_address_slice, borrow_bounded_str, read_tail, read_tail_len, tail_capacity,
    tail_payload, write_tail, write_tail_payload, BoundedString, BoundedVec, HopperString,
    HopperVec, TailBytes, TailCodec, TailElement, TailStr,
};

/// Compose a layout's `LayoutMigration::MIGRATIONS` chain from a list
/// of `#[hopper::migrate]`-emitted edge constants.
///
/// ```ignore
/// #[hopper::migrate(from = 1, to = 2)]
/// pub fn vault_v1_to_v2(body: &mut [u8]) -> ProgramResult { Ok(()) }
///
/// hopper::layout_migrations! {
///     Vault = [VAULT_V1_TO_V2_EDGE, VAULT_V2_TO_V3_EDGE],
/// }
/// ```
///
/// Emits `impl LayoutMigration for Vault { const MIGRATIONS = .. }`.
/// Each list entry must evaluate to a
/// [`MigrationEdge`](crate::migrate::MigrationEdge). typically the
/// `<UPPER_SNAKE_FN_NAME>_EDGE` constant that
/// `#[hopper::migrate]` emits alongside each migration function.
/// Chain continuity (every adjacent pair must satisfy
/// `a.to_epoch == b.from_epoch`) is enforced at runtime by
/// [`apply_pending_migrations`].
#[macro_export]
macro_rules! layout_migrations {
    ( $layout:ty = [ $( $edge:expr ),+ $(,)? ] $(,)? ) => {
        impl $crate::migrate::LayoutMigration for $layout {
            const MIGRATIONS: &'static [$crate::migrate::MigrationEdge] = &[
                $( $edge ),+
            ];
        }
    };
}
pub use instruction::CpiAccount;
pub use instruction::{
    InstructionAccount, InstructionView, Seed, Signer, StoredAccountMeta, StoredInstruction,
};
pub use layout::{HopperHeader, LayoutContract, LayoutInfo};
pub use pod::{read_unaligned_value, Pod, ValuePod, Zeroable};
pub use result::ProgramResult;
pub use segment::{
    FieldCapability, Segment, TypedSegment, FIELD_POLICY_AUTHORITY_GATED,
    FIELD_POLICY_CHECKED_MATH, FIELD_POLICY_IMMUTABLE_AFTER_INIT, FIELD_ROLE_AUTHORITY,
    FIELD_ROLE_BALANCE, FIELD_ROLE_DATA, FIELD_ROLE_VERSION,
};
pub use segment_borrow::{AccessKind, SegmentBorrow, SegmentBorrowGuard, SegmentBorrowRegistry};
pub use segment_lease::{SegRef, SegRefMut, SegmentLease, SegmentsMut};
pub use write_policy::{WritePolicy, WriteRange, WRITE_POLICY_VIOLATION_PAGE};
pub use zerocopy::{AccountLayout, WireLayout, ZeroCopy};

pub const MAX_TX_ACCOUNTS: usize = native_boundary::BACKEND_MAX_TX_ACCOUNTS;
pub const SUCCESS: u64 = native_boundary::BACKEND_SUCCESS;

#[doc(hidden)]
pub use hopper_native as __hopper_native;
pub use hopper_native::sha256;

#[doc(hidden)]
pub use hopper_native::address::decode_base58_32 as __decode_base58_32;

/// Compile-time base58 address literal.
#[macro_export]
macro_rules! address {
    ( $literal:expr ) => {
        $crate::Address::new_from_array($crate::__decode_base58_32($literal))
    };
}

/// Declare a program's on-chain id, mirroring the `declare_id!` convention
/// every other Solana framework ships (Anchor, Pinocchio, Quasar).
///
/// Expands to a `pub const ID: Address` decoded at compile time, a
/// `pub const fn id() -> Address` accessor, and a `pub fn check_id(&Address)
/// -> bool` guard. Programs that pin their own id for self-PDA derivation,
/// CPI-guard checks, or `require_keys_eq!(program.key(), &crate::ID)` use this
/// instead of hand-rolling an [`address!`] constant.
///
/// ```ignore
/// hopper::declare_id!("D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F");
/// assert!(crate::check_id(&crate::ID));
/// ```
#[macro_export]
macro_rules! declare_id {
    ( $literal:expr ) => {
        /// The program's on-chain address, decoded at compile time.
        pub const ID: $crate::Address = $crate::address!($literal);

        /// Return the program's declared id.
        #[inline]
        pub const fn id() -> $crate::Address {
            ID
        }

        /// Return true if `other` equals the declared program id.
        #[inline]
        pub fn check_id(other: &$crate::Address) -> bool {
            *other == ID
        }
    };
}

/// Early-return with an error if the condition is false.
#[macro_export]
macro_rules! require {
    ( $cond:expr, $err:expr ) => {
        if !($cond) {
            return Err($err);
        }
    };
    ( $cond:expr ) => {
        if !($cond) {
            return Err($crate::ProgramError::InvalidArgument);
        }
    };
}

/// Assert two values are equal, returning an error on mismatch.
#[macro_export]
macro_rules! require_eq {
    ( $left:expr, $right:expr, $err:expr ) => {
        if ($left) != ($right) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if ($left) != ($right) {
            return Err($crate::ProgramError::InvalidArgument);
        }
    };
}

/// Assert two values are not equal. Early-returns with the supplied
/// error on match (or `ProgramError::InvalidArgument` in the short
/// form). Symmetric with [`require_eq!`].
#[macro_export]
macro_rules! require_neq {
    ( $left:expr, $right:expr, $err:expr ) => {
        if ($left) == ($right) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if ($left) == ($right) {
            return Err($crate::ProgramError::InvalidArgument);
        }
    };
}

/// Assert two public keys (or any byte slices convertible via
/// [`AsRef<[u8; 32]>`]) are equal. Narrower than [`require_eq!`] but
/// matches the ergonomic spelling ecosystem migrators coming from
/// Anchor / Jiminy are familiar with.
///
/// ```ignore
/// hopper::require_keys_eq!(
///     vault.authority,
///     ctx.signer.address(),
///     ProgramError::InvalidAccountData,
/// );
/// ```
#[macro_export]
macro_rules! require_keys_eq {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !$crate::address::keys_eq(
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left),
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right),
        ) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if !$crate::address::keys_eq(
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left),
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right),
        ) {
            return Err($crate::ProgramError::InvalidAccountData);
        }
    };
}

/// Assert two public keys are *not* equal. Used for pinning distinct
/// accounts (authority != user, source != destination). Same coercion
/// and error semantics as [`require_keys_eq!`].
#[macro_export]
macro_rules! require_keys_neq {
    ( $left:expr, $right:expr, $err:expr ) => {
        if $crate::address::keys_eq(
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left),
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right),
        ) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if $crate::address::keys_eq(
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left),
            ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right),
        ) {
            return Err($crate::ProgramError::InvalidAccountData);
        }
    };
}

/// Assert `left >= right`, returning the supplied error on underrun.
/// Useful for lamport / balance checks.
#[macro_export]
macro_rules! require_gte {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left >= $right) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if !($left >= $right) {
            return Err($crate::ProgramError::InsufficientFunds);
        }
    };
}

/// Assert `left > right` strictly.
#[macro_export]
macro_rules! require_gt {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left > $right) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if !($left > $right) {
            return Err($crate::ProgramError::InvalidArgument);
        }
    };
}

/// Assert `left < right` strictly. Anchor-parity sibling of
/// [`require_gt!`]. Default error is `ProgramError::InvalidArgument`
/// because a failed ordering check most often flags a bad user input.
#[macro_export]
macro_rules! require_lt {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left < $right) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if !($left < $right) {
            return Err($crate::ProgramError::InvalidArgument);
        }
    };
}

/// Assert `left <= right`. Anchor-parity sibling of [`require_gte!`].
#[macro_export]
macro_rules! require_lte {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left <= $right) {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if !($left <= $right) {
            return Err($crate::ProgramError::InvalidArgument);
        }
    };
}

/// Return an error immediately. Parallel to Anchor's `err!`.
///
/// The macro expands to a bare `return Err(...)`, so the call site
/// reads like a control-flow keyword rather than an expression. The
/// argument is evaluated as an expression so either a Hopper-generated
/// error code or a raw `ProgramError` works.
///
/// ```ignore
/// if amount == 0 {
///     return err!(VaultError::ZeroDeposit);
/// }
/// ```
#[macro_export]
macro_rules! err {
    ( $e:expr ) => {
        return ::core::result::Result::Err($crate::ProgramError::from($e))
    };
}

/// Alias for [`err!`]. Anchor-style spelling for ported code. Functionally
/// identical.
#[macro_export]
macro_rules! error {
    ( $e:expr ) => {
        return ::core::result::Result::Err($crate::ProgramError::from($e))
    };
}

/// Auditable raw-pointer boundary.
///
/// Wraps a block that needs `unsafe` in a named Hopper macro so an
/// auditor can grep `hopper_unsafe_region!` and find every raw
/// reinterpretation in the tree with one command. The macro expands
/// to a plain `unsafe { ... }` block: zero runtime cost, identical
/// codegen, but the invocation site is nameable and documented.
///
/// Usage:
///
/// ```ignore
/// let cleared = hopper::hopper_unsafe_region!("clear rewards via raw ptr", {
///     let ptr = ctx.as_mut_ptr(0)?;
///     (ptr.add(24) as *mut u64).write_unaligned(0);
///     0u64
/// });
/// ```
///
/// The label is a compile-time string literal. It is discarded by
/// the expansion but serves as inline documentation an auditor
/// reads alongside the `unsafe` body.
#[macro_export]
macro_rules! hopper_unsafe_region {
    ( $label:literal, $body:block ) => {{
        // The label is a compile-time string literal, captured so it
        // surfaces in `cargo expand` output and can be grep'd out of
        // the expanded tree the same way as the macro name.
        const _HOPPER_UNSAFE_REGION_LABEL: &str = $label;
        #[allow(unused_unsafe)]
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { $body }
    }};
}

/// Backend-neutral logging macro.
#[macro_export]
macro_rules! msg {
    ( $literal:expr ) => {{
        $crate::log::log($literal);
    }};
    ( $fmt:expr, $($arg:tt)* ) => {{
        #[cfg(target_os = "solana")]
        {
            use core::fmt::Write;
            let mut buf = [0u8; 256];
            let mut wrapper = $crate::log::StackWriter::new(&mut buf);
            let _ = write!(wrapper, $fmt, $($arg)*);
            let len = wrapper.pos();
            $crate::log::log(
                // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
                unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
            );
        }
        #[cfg(not(target_os = "solana"))]
        {
            let _ = ($fmt, $($arg)*);
        }
    }};
}

/// Emit a Hopper event via self-CPI for reliable indexing — the
/// manual-wiring form.
///
/// Most programs should not call this directly: `#[hopper::context(event_cpi)]`
/// plus `ctx.emit_event_cpi(&event)` wires the same emission (accounts,
/// bump, sink) with zero hand-written plumbing. Reach for this macro
/// only when the context macro is out of the picture (raw handlers,
/// hand-rolled account handling, a custom sink).
///
/// Wraps [`cpi_event::encode_event_cpi`] and a call into the active
/// backend's `invoke_signed` so indexers see the event as an inner
/// instruction in the transaction metadata. Log output is size-capped; inner
/// instructions are retained. Anchor's `emit_cpi!` solves the same problem
/// with the same trick; Hopper's lives in pure Rust so it works under
/// `no_std` and any of the three backends — and its wire format is
/// 3 bytes of overhead (2-byte marker + 1-byte tag) against Anchor's 16
/// (8-byte instruction tag + 8-byte event discriminator).
///
/// ## Required program plumbing (manual form only)
///
/// The caller must declare a sentinel handler so the dispatcher routes
/// the self-CPI somewhere — and should authenticate it rather than
/// no-op, or forged events become possible:
///
/// ```ignore
/// #[instruction(discriminator = [0xE0, 0x1E])]
/// fn __hopper_event_sink(ctx: &mut Context<'_>) -> ProgramResult {
///     hopper_runtime::cpi_event::handle_event_sink(ctx, ctx.instruction_data())
/// }
/// ```
///
/// And a PDA account seeded with [`cpi_event::EVENT_AUTHORITY_SEED`]
/// (`b"__hopper_event_authority"`) so the CPI has a signer, plus the
/// program's own account in the instruction so the runtime can resolve
/// the self-CPI target.
///
/// ## Usage
///
/// ```ignore
/// hopper_emit_cpi!(
///     ctx.program_id(),
///     event_authority: &AccountView,
///     event_authority_bump: u8,
///     Deposited { amount, depositor }
/// );
/// ```
///
/// `$event` must be a `#[hopper::event]` type (anything implementing
/// [`cpi_event::CpiEvent`]). Expands to: build instruction bytes,
/// invoke_signed with the event_authority PDA as the signer. One CPI,
/// bounded stack allocation, zero heap.
#[macro_export]
macro_rules! hopper_emit_cpi {
    ( $program_id:expr, $event_authority:expr, $bump:expr, $event:expr ) => {{
        // Build the wire format into a stack buffer. MAX_EVENT_PAYLOAD
        // (512) bytes fits every sensibly-sized event; callers with
        // larger events should grow the buffer at the call site or use
        // `emit!` with the log-based path.
        let __ev = $event;
        let __tag: u8 = $crate::cpi_event::CpiEvent::tag(&__ev);
        let __payload: &[u8] = $crate::cpi_event::CpiEvent::payload_bytes(&__ev);
        let mut __buf = [0u8; 2 + 1 + $crate::cpi_event::MAX_EVENT_PAYLOAD];
        let __n = $crate::cpi_event::encode_event_cpi(__tag, __payload, &mut __buf[..])
            .ok_or($crate::ProgramError::InvalidInstructionData)?;
        // Signer seeds for the event-authority PDA. The caller
        // derived and cached `$bump` so this is a stored-bump CPI.
        let __bump_byte: [u8; 1] = [$bump];
        let __seed_slices: [&[u8]; 2] = [$crate::cpi_event::EVENT_AUTHORITY_SEED, &__bump_byte[..]];
        $crate::cpi_event::invoke_event_cpi(
            $program_id,
            $event_authority,
            &__buf[..__n],
            &__seed_slices[..],
        )?;
    }};
}

/// Cheap structured logging for hot handlers.
///
/// `hopper_log!` is the compute-unit-aware sibling of [`msg!`]. It
/// dispatches to the backend's native log syscall with no format
/// machinery, no stack buffer, and no UTF-8 formatting pass. The
/// tradeoff: fewer ergonomics, predictable CU.
///
/// Forms:
///
/// - `hopper_log!("static message")` - one `sol_log_` syscall.
/// - `hopper_log!(my_str_slice)` - same, but for runtime `&str` values.
/// - `hopper_log!("label:", u64_value)` - one `sol_log_` plus one
///   `sol_log_64_`. Five `u64` slots (the `sol_log_64_` ABI) are
///   populated left-to-right and the rest zero.
/// - `hopper_log!("label:", a, b)` through `hopper_log!("label:", a, b, c, d, e)` -
///   same pattern; up to five integer values per call.
///
/// Reach for `msg!` when you need `{}`-style formatting. Reach for
/// `hopper_log!` when you are paying for every CU and you already
/// know the shape of the data.
#[macro_export]
macro_rules! hopper_log {
    // One label + 1..=5 integer values. Each integer is cast to `u64`
    // at the call site so callers do not need to sprinkle `as u64`.
    ($label:expr, $a:expr) => {{
        $crate::log::log($label);
        $crate::log::log_64($a as u64, 0, 0, 0, 0);
    }};
    ($label:expr, $a:expr, $b:expr) => {{
        $crate::log::log($label);
        $crate::log::log_64($a as u64, $b as u64, 0, 0, 0);
    }};
    ($label:expr, $a:expr, $b:expr, $c:expr) => {{
        $crate::log::log($label);
        $crate::log::log_64($a as u64, $b as u64, $c as u64, 0, 0);
    }};
    ($label:expr, $a:expr, $b:expr, $c:expr, $d:expr) => {{
        $crate::log::log($label);
        $crate::log::log_64($a as u64, $b as u64, $c as u64, $d as u64, 0);
    }};
    ($label:expr, $a:expr, $b:expr, $c:expr, $d:expr, $e:expr) => {{
        $crate::log::log($label);
        $crate::log::log_64($a as u64, $b as u64, $c as u64, $d as u64, $e as u64);
    }};
    // Bare message. Uses the one-argument `log::log` syscall.
    ($msg:expr) => {{
        $crate::log::log($msg);
    }};
}

/// Declare the explicit Hopper runtime entrypoint bridge.
///
/// This is Hopper's direct runtime entrypoint over Solana account memory.
#[macro_export]
macro_rules! hopper_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_entrypoint!($process_instruction, { $crate::MAX_TX_ACCOUNTS });
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8) -> u64 {
            const UNINIT: core::mem::MaybeUninit<$crate::__hopper_native::AccountView<'static>> =
                core::mem::MaybeUninit::<$crate::__hopper_native::AccountView<'static>>::uninit();
            let mut accounts = [UNINIT; $maximum];

            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            let (program_id, count, instruction_data) = unsafe {
                $crate::__hopper_native::raw_input::deserialize_accounts::<$maximum>(
                    input,
                    &mut accounts,
                )
            };

            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            let hopper_program_id = unsafe {
                &*(&program_id as *const $crate::__hopper_native::Address as *const $crate::Address)
            };
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            let hopper_accounts = unsafe {
                core::slice::from_raw_parts(
                    accounts.as_ptr() as *const $crate::AccountView<'_>,
                    count,
                )
            };

            match $process_instruction(hopper_program_id, hopper_accounts, instruction_data) {
                Ok(()) => $crate::__hopper_native::SUCCESS,
                Err(error) => error.into(),
            }
        }
    };
}

/// Declare the canonical Hopper program entrypoint.
#[macro_export]
macro_rules! program_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_entrypoint!($process_instruction);
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        $crate::hopper_entrypoint!($process_instruction, $maximum);
    };
}

/// Declare the fast two-argument Hopper entrypoint.
///
/// Uses the SVM's second register (`r2`) to receive instruction data
/// directly, eliminating the full account-scanning pass (~30-40 CU per
/// instruction). The `r2` instruction-data pointer is [SIMD-0321], whose
/// feature gate is **not yet activated** on public clusters.
///
/// Without the `simd-0321` cargo feature this macro expands to the
/// standard scanning entrypoint — identical semantics, sound everywhere
/// today. With the feature it expands to the two-argument form, which
/// null-checks `r2` and falls back to the scanning parse as defense in
/// depth.
///
/// [SIMD-0321]: https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0321-vm-r2-instruction-data-pointer.md
#[cfg(feature = "simd-0321")]
#[macro_export]
macro_rules! hopper_fast_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_fast_entrypoint!($process_instruction, { $crate::MAX_TX_ACCOUNTS });
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer.
        /// When SIMD-0321 is active, `ix_data` points to the instruction data
        /// with its u64 length stored at offset -8; when it is not active the
        /// register is zero and the scanning fallback is taken.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8, ix_data: *const u8) -> u64 {
            const UNINIT: core::mem::MaybeUninit<$crate::__hopper_native::AccountView> =
                core::mem::MaybeUninit::<$crate::__hopper_native::AccountView>::uninit();
            let mut accounts = [UNINIT; $maximum];

            let (program_id, count, instruction_data) = if ix_data.is_null() {
                // SIMD-0321 not active on this cluster: r2 is zero. Fall back
                // to the full scanning parse so the program stays correct.
                // SAFETY: `input` is the loader-provided input buffer; the
                // scanning parser owns all bounds/duplicate-marker checks.
                unsafe {
                    $crate::__hopper_native::raw_input::deserialize_accounts::<$maximum>(
                        input,
                        &mut accounts,
                    )
                }
            } else {
                // SAFETY: SIMD-0321 guarantees `ix_data` points at the
                // instruction-data bytes, with the u64 length prefix at
                // `ix_data - 8` and the 32-byte program id after the data.
                let ix_len =
                    unsafe { core::ptr::read_unaligned(ix_data.sub(8) as *const u64) as usize };
                let instruction_data: &'static [u8] =
                    unsafe { core::slice::from_raw_parts(ix_data, ix_len) };
                // SAFETY: program id trails the instruction data per the
                // loader serialization layout; reading 32 bytes by value.
                let program_id = unsafe {
                    core::ptr::read(ix_data.add(ix_len) as *const $crate::__hopper_native::Address)
                };

                // SAFETY: `input` is the loader input buffer; account-slot
                // framing is validated by `deserialize_accounts_fast`.
                unsafe {
                    $crate::__hopper_native::raw_input::deserialize_accounts_fast::<$maximum>(
                        input,
                        &mut accounts,
                        instruction_data,
                        program_id,
                    )
                }
            };

            // SAFETY: `Address` is a transparent 32-byte wrapper shared by the
            // native and runtime layers; the reinterpret is layout-identical.
            let hopper_program_id = unsafe {
                &*(&program_id as *const $crate::__hopper_native::Address as *const $crate::Address)
            };
            // SAFETY: the first `count` slots were initialized by the parser;
            // runtime `AccountView` is repr(transparent) over the native view.
            let hopper_accounts = unsafe {
                core::slice::from_raw_parts(accounts.as_ptr() as *const $crate::AccountView, count)
            };

            match $process_instruction(hopper_program_id, hopper_accounts, instruction_data) {
                Ok(()) => $crate::__hopper_native::SUCCESS,
                Err(error) => error.into(),
            }
        }
    };
}

/// Without the `simd-0321` feature the "fast" entrypoint is an alias for
/// the standard scanning entrypoint: SIMD-0321 has not been activated, so
/// the two-argument form would read an uninitialized register on today's
/// clusters. Rebuild with `--features simd-0321` once the gate activates.
#[cfg(not(feature = "simd-0321"))]
#[macro_export]
macro_rules! hopper_fast_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_entrypoint!($process_instruction);
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        $crate::hopper_entrypoint!($process_instruction, $maximum);
    };
}

/// Backward-compatible alias for the fast Hopper entrypoint macro.
#[macro_export]
macro_rules! fast_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_fast_entrypoint!($process_instruction);
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        $crate::hopper_fast_entrypoint!($process_instruction, $maximum);
    };
}

/// Declare the Hopper lazy entrypoint.
#[macro_export]
macro_rules! hopper_lazy_entrypoint {
    ( $process:expr ) => {
        $crate::__hopper_native::hopper_lazy_entrypoint!($process);
    };
}

/// Backward-compatible alias for the lazy Hopper entrypoint macro.
#[macro_export]
macro_rules! lazy_entrypoint {
    ( $process:expr ) => {
        $crate::hopper_lazy_entrypoint!($process);
    };
}

#[macro_export]
macro_rules! no_allocator {
    () => {
        #[cfg(target_os = "solana")]
        mod __hopper_allocator {
            struct NoAlloc;

            unsafe impl core::alloc::GlobalAlloc for NoAlloc {
                unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
                    // A no-alloc program must never reach here. Returning null
                    // fails the allocation and builds on stable SBF without the
                    // experimental inline-asm arch feature. Reach for
                    // `hopper_native::no_allocator!` (which traps via asm) when
                    // the crate already enables `asm_experimental_arch`.
                    core::ptr::null_mut()
                }

                unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
            }

            #[global_allocator]
            static ALLOCATOR: NoAlloc = NoAlloc;
        }
    };
}

/// Install the default bump allocator over the SVM heap region. Opt-in
/// counterpart to [`no_allocator!`] for programs that need `alloc` on a
/// cold path. See [`hopper_native::BumpAllocator`].
#[macro_export]
macro_rules! default_allocator {
    () => {
        #[cfg(target_os = "solana")]
        #[global_allocator]
        static ALLOCATOR: $crate::__hopper_native::BumpAllocator =
            $crate::__hopper_native::BumpAllocator {
                start: $crate::__hopper_native::HEAP_START_ADDRESS,
                len: $crate::__hopper_native::HEAP_LENGTH,
            };
    };
}

#[macro_export]
macro_rules! nostd_panic_handler {
    () => {
        #[cfg(target_os = "solana")]
        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {
            // Stable-SBF panic handler: no inline asm, so it builds without the
            // experimental `asm_experimental_arch` feature. The runtime caps
            // compute, so the spin terminates the transaction. Use
            // `hopper_native::nostd_panic_handler!` for the asm trap variant
            // when the crate already enables that feature.
            let _ = _info;
            loop {
                core::hint::spin_loop();
            }
        }
    };
}
