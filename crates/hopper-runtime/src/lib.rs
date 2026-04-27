//! Hopper Runtime -- canonical semantic runtime surface.
//!
//! Hopper Runtime owns the public rules, validation, typed loading, CPI
//! semantics, and execution context that authored Hopper code targets.
//! Hopper Native owns the raw execution boundary. Pinocchio and
//! solana-program remain compatibility backends isolated behind `compat/`.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(test)]
extern crate std;

#[cfg(feature = "solana-program-backend")]
extern crate alloc;

#[cfg(any(
    all(feature = "hopper-native-backend", feature = "pinocchio-backend"),
    all(feature = "hopper-native-backend", feature = "solana-program-backend"),
    all(feature = "pinocchio-backend", feature = "solana-program-backend"),
))]
compile_error!(
    "Only one backend feature may be enabled at a time: hopper-native-backend, pinocchio-backend, or solana-program-backend"
);

#[cfg(not(any(
    feature = "hopper-native-backend",
    feature = "pinocchio-backend",
    feature = "solana-program-backend",
)))]
compile_error!(
    "At least one backend feature must be enabled: hopper-native-backend, pinocchio-backend, or solana-program-backend"
);

#[doc(hidden)]
pub mod compat;

pub mod error;
pub mod result;
pub mod address;
pub mod account;
pub mod account_wrappers;
pub mod audit;
pub mod borrow;
pub(crate) mod borrow_registry;
pub mod cpi;
pub mod cpi_event;
pub mod crank;
pub mod dyn_cpi;
pub mod utils;
pub mod field_map;
pub mod foreign;
pub mod migrate;
pub mod tail;
pub mod interop;
pub mod log;
pub mod pod;
pub mod segment;
pub mod zerocopy;
pub mod policy;
pub mod ref_only;
// Re-export the sealed marker module at the crate root so macro
// codegen can address it as `::hopper_runtime::__sealed::...`. It's
// doc-hidden because it's the audit's Step 5 enforcement surface,
// not a normal-user-facing API.
#[doc(hidden)]
pub use zerocopy::__sealed;
pub mod segment_borrow;
pub mod segment_lease;
pub mod instruction;
pub mod layout;
pub mod context;
pub mod pda;
pub mod rent;
pub mod syscall;
pub mod syscalls;
pub mod system;
pub mod option_byte;
pub mod remaining;
pub mod token;
pub mod token_2022_ext;

pub use account::{AccountView, RemainingAccounts};
pub use account_wrappers::{Account, InitAccount, Program, ProgramId, Signer as HopperSigner, SystemId};
pub use address::Address;
pub use audit::{AccountAudit, DuplicateAccount};
pub use borrow::{Ref, RefMut};
pub use policy::{HopperInstructionPolicy, HopperProgramPolicy};
pub use ref_only::HopperRefOnly;
pub use context::Context;
pub use cpi::{invoke, invoke_signed};
pub use error::ProgramError;
pub use field_map::{FieldInfo, FieldMap};
pub use foreign::{ForeignLens, ForeignManifest};
pub use interop::TransparentAddress;
pub use migrate::{apply_pending_migrations, LayoutMigration, MigrationEdge};
pub use tail::{read_tail, read_tail_len, tail_payload, write_tail, TailCodec};

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
#[cfg(feature = "hopper-native-backend")]
pub use instruction::CpiAccount;
pub use instruction::{InstructionAccount, InstructionView, Seed, Signer};
pub use layout::{HopperHeader, LayoutContract, LayoutInfo};
pub use result::ProgramResult;
pub use pod::Pod;
pub use zerocopy::{AccountLayout, WireLayout, ZeroCopy};
pub use segment::{Segment, TypedSegment};
pub use segment_borrow::{AccessKind, SegmentBorrow, SegmentBorrowGuard, SegmentBorrowRegistry};
pub use segment_lease::{SegRef, SegRefMut, SegmentLease};

pub const MAX_TX_ACCOUNTS: usize = compat::BACKEND_MAX_TX_ACCOUNTS;
pub const SUCCESS: u64 = compat::BACKEND_SUCCESS;

#[cfg(feature = "hopper-native-backend")]
#[doc(hidden)]
pub use hopper_native as __hopper_native;

#[cfg(feature = "solana-program-backend")]
#[doc(hidden)]
pub use ::solana_program as __solana_program;

#[doc(hidden)]
pub use five8_const as __five8_const;

/// Compile-time base58 address literal.
#[macro_export]
macro_rules! address {
    ( $literal:expr ) => {
        $crate::Address::new_from_array($crate::__five8_const::decode_32_const($literal))
    };
}

/// Early-return with an error if the condition is false.
#[macro_export]
macro_rules! require {
    ( $cond:expr, $err:expr ) => {
        if !($cond) { return Err($err); }
    };
    ( $cond:expr ) => {
        if !($cond) { return Err($crate::ProgramError::InvalidArgument); }
    };
}

/// Assert two values are equal, returning an error on mismatch.
#[macro_export]
macro_rules! require_eq {
    ( $left:expr, $right:expr, $err:expr ) => {
        if ($left) != ($right) { return Err($err); }
    };
    ( $left:expr, $right:expr ) => {
        if ($left) != ($right) { return Err($crate::ProgramError::InvalidArgument); }
    };
}

/// Assert two values are not equal. Early-returns with the supplied
/// error on match (or `ProgramError::InvalidArgument` in the short
/// form). Symmetric with [`require_eq!`].
#[macro_export]
macro_rules! require_neq {
    ( $left:expr, $right:expr, $err:expr ) => {
        if ($left) == ($right) { return Err($err); }
    };
    ( $left:expr, $right:expr ) => {
        if ($left) == ($right) { return Err($crate::ProgramError::InvalidArgument); }
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
        if ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left)
            != ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right)
        {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left)
            != ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right)
        {
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
        if ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left)
            == ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right)
        {
            return Err($err);
        }
    };
    ( $left:expr, $right:expr ) => {
        if ::core::convert::AsRef::<[u8; 32]>::as_ref(&$left)
            == ::core::convert::AsRef::<[u8; 32]>::as_ref(&$right)
        {
            return Err($crate::ProgramError::InvalidAccountData);
        }
    };
}

/// Assert `left >= right`, returning the supplied error on underrun.
/// Useful for lamport / balance checks.
#[macro_export]
macro_rules! require_gte {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left >= $right) { return Err($err); }
    };
    ( $left:expr, $right:expr ) => {
        if !($left >= $right) { return Err($crate::ProgramError::InsufficientFunds); }
    };
}

/// Assert `left > right` strictly.
#[macro_export]
macro_rules! require_gt {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left > $right) { return Err($err); }
    };
    ( $left:expr, $right:expr ) => {
        if !($left > $right) { return Err($crate::ProgramError::InvalidArgument); }
    };
}

/// Assert `left < right` strictly. Anchor-parity sibling of
/// [`require_gt!`]. Default error is `ProgramError::InvalidArgument`
/// because a failed ordering check most often flags a bad user input.
#[macro_export]
macro_rules! require_lt {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left < $right) { return Err($err); }
    };
    ( $left:expr, $right:expr ) => {
        if !($left < $right) { return Err($crate::ProgramError::InvalidArgument); }
    };
}

/// Assert `left <= right`. Anchor-parity sibling of [`require_gte!`].
#[macro_export]
macro_rules! require_lte {
    ( $left:expr, $right:expr, $err:expr ) => {
        if !($left <= $right) { return Err($err); }
    };
    ( $left:expr, $right:expr ) => {
        if !($left <= $right) { return Err($crate::ProgramError::InvalidArgument); }
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

/// Alias for [`err!`]. Anchor compatibility shim so ported code needs
/// no rename. Functionally identical.
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
                unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
            );
        }
        #[cfg(not(target_os = "solana"))]
        {
            let _ = ($fmt, $($arg)*);
        }
    }};
}

/// Emit a Hopper event via self-CPI for reliable indexing.
///
/// Wraps [`cpi_event::encode_event_cpi`] and a call into the active
/// backend's `invoke_signed` so indexers see the event as an inner
/// instruction in the transaction metadata. Logs truncate; inner
/// instructions do not. Anchor's `emit_cpi!` solves the same problem
/// with the same trick; Hopper's lives in pure Rust so it works under
/// `no_std` and any of the three backends.
///
/// ## Required program plumbing
///
/// The caller must declare a sentinel handler so the runtime routes
/// the self-CPI somewhere:
///
/// ```ignore
/// #[instruction(discriminator = [0xE0, 0x1E])]
/// fn __hopper_event_sink(_ctx: &mut Context<'_>) -> ProgramResult {
///     Ok(())
/// }
/// ```
///
/// And a PDA named `event_authority` seeded with
/// `[b"__hopper_event_authority"]` so the CPI has a signer.
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
/// Expands to: build instruction bytes, invoke_signed with the
/// event_authority PDA as the signer. One CPI, bounded stack
/// allocation, zero heap.
#[macro_export]
macro_rules! hopper_emit_cpi {
    ( $program_id:expr, $event_authority:expr, $bump:expr, $event:expr ) => {{
        // Build the wire format into a stack buffer. 512 payload bytes
        // fits every sensibly-sized event; callers with larger events
        // should grow the buffer at the call site or use `emit!` with
        // the log-based path.
        let __ev = $event;
        let __tag: u8 = ::core::convert::Into::<u8>::into(__ev.tag());
        let __payload: &[u8] = __ev.as_bytes();
        let mut __buf = [0u8; 2 + 1 + 512];
        let __n = $crate::cpi_event::encode_event_cpi(__tag, __payload, &mut __buf[..])
            .ok_or($crate::ProgramError::InvalidInstructionData)?;
        // Signer seeds for the event-authority PDA. The caller
        // derived and cached `$bump` so this is a stored-bump CPI.
        let __bump_byte: [u8; 1] = [$bump];
        let __seed_slices: [&[u8]; 2] = [b"__hopper_event_authority", &__bump_byte[..]];
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
        $crate::log::log_64(
            $a as u64, $b as u64, $c as u64, $d as u64, $e as u64,
        );
    }};
    // Bare message. Uses the one-argument `log::log` syscall.
    ($msg:expr) => {{
        $crate::log::log($msg);
    }};
}

/// Declare the explicit Hopper runtime entrypoint bridge.
///
/// With `hopper-native-backend`, this is a thin alias to Hopper Native's raw
/// entrypoint macro. With compatibility backends, it delegates to `compat/`.
#[macro_export]
macro_rules! hopper_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_entrypoint!($process_instruction, { $crate::MAX_TX_ACCOUNTS });
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        #[cfg(feature = "hopper-native-backend")]
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8) -> u64 {
            const UNINIT: core::mem::MaybeUninit<$crate::__hopper_native::AccountView> =
                core::mem::MaybeUninit::<$crate::__hopper_native::AccountView>::uninit();
            let mut accounts = [UNINIT; $maximum];

            let (program_id, count, instruction_data) = unsafe {
                $crate::__hopper_native::raw_input::deserialize_accounts::<$maximum>(
                    input,
                    &mut accounts,
                )
            };

            let hopper_program_id = unsafe {
                &*(
                    &program_id as *const $crate::__hopper_native::Address
                        as *const $crate::Address
                )
            };
            let hopper_accounts = unsafe {
                core::slice::from_raw_parts(accounts.as_ptr() as *const $crate::AccountView, count)
            };

            match $process_instruction(hopper_program_id, hopper_accounts, instruction_data) {
                Ok(()) => $crate::__hopper_native::SUCCESS,
                Err(error) => error.into(),
            }
        }

        #[cfg(any(feature = "pinocchio-backend", feature = "solana-program-backend"))]
        $crate::__hopper_compat_entrypoint!($process_instruction, $maximum);
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
/// Uses the SVM's second register to receive instruction data directly,
/// eliminating the full account-scanning pass. Saves ~30-40 CU per
/// instruction. Requires SVM runtime ≥1.17.
#[macro_export]
macro_rules! hopper_fast_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_fast_entrypoint!($process_instruction, { $crate::MAX_TX_ACCOUNTS });
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        #[cfg(feature = "hopper-native-backend")]
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer
        /// and `ix_data` points to the instruction data with its u64 length
        /// stored at offset -8.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8, ix_data: *const u8) -> u64 {
            const UNINIT: core::mem::MaybeUninit<$crate::__hopper_native::AccountView> =
                core::mem::MaybeUninit::<$crate::__hopper_native::AccountView>::uninit();
            let mut accounts = [UNINIT; $maximum];

            let ix_len = unsafe { *(ix_data.sub(8) as *const u64) as usize };
            let instruction_data: &'static [u8] =
                unsafe { core::slice::from_raw_parts(ix_data, ix_len) };
            let program_id = unsafe {
                core::ptr::read(ix_data.add(ix_len) as *const $crate::__hopper_native::Address)
            };

            let (program_id, count, instruction_data) = unsafe {
                $crate::__hopper_native::raw_input::deserialize_accounts_fast::<$maximum>(
                    input,
                    &mut accounts,
                    instruction_data,
                    program_id,
                )
            };

            let hopper_program_id = unsafe {
                &*(
                    &program_id as *const $crate::__hopper_native::Address
                        as *const $crate::Address
                )
            };
            let hopper_accounts = unsafe {
                core::slice::from_raw_parts(accounts.as_ptr() as *const $crate::AccountView, count)
            };

            match $process_instruction(hopper_program_id, hopper_accounts, instruction_data) {
                Ok(()) => $crate::__hopper_native::SUCCESS,
                Err(error) => error.into(),
            }
        }

        #[cfg(any(feature = "pinocchio-backend", feature = "solana-program-backend"))]
        compile_error!("hopper_fast_entrypoint! requires hopper-native-backend");
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
        #[cfg(feature = "hopper-native-backend")]
        $crate::__hopper_native::hopper_lazy_entrypoint!($process);

        #[cfg(any(feature = "pinocchio-backend", feature = "solana-program-backend"))]
        compile_error!("hopper_lazy_entrypoint! requires hopper-native-backend");
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
                    core::ptr::null_mut()
                }

                unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
            }

            #[global_allocator]
            static ALLOCATOR: NoAlloc = NoAlloc;
        }
    };
}

#[macro_export]
macro_rules! nostd_panic_handler {
    () => {
        #[cfg(target_os = "solana")]
        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {
            let _ = _info;
            loop {
                core::hint::spin_loop();
            }
        }
    };
}
