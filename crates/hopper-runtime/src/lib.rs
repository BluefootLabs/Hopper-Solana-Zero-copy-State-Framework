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
pub mod field_map;
pub mod foreign;
pub mod migrate;
pub mod tail;
pub mod interop;
pub mod log;
pub mod pod;
pub mod segment;
pub mod zerocopy;
pub mod segment_borrow;
pub mod segment_lease;
pub mod instruction;
pub mod layout;
pub mod context;
pub mod pda;
pub mod syscall;
pub mod syscalls;
pub mod system;
pub mod token;

pub use account::{AccountView, RemainingAccounts};
pub use account_wrappers::{Account, InitAccount, Program, ProgramId, Signer as HopperSigner, SystemId};
pub use address::Address;
pub use audit::{AccountAudit, DuplicateAccount};
pub use borrow::{Ref, RefMut};
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
