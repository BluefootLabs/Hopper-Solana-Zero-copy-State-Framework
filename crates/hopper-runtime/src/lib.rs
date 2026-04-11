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
pub mod audit;
pub mod borrow;
pub(crate) mod borrow_registry;
pub mod cpi;
pub mod field_map;
pub mod segment_borrow;
pub mod instruction;
pub mod layout;
pub mod context;
pub mod pda;
pub mod system;
pub mod token;

pub use account::{AccountView, RemainingAccounts};
pub use address::Address;
pub use audit::{AccountAudit, DuplicateAccount};
pub use borrow::{Ref, RefMut};
pub use context::Context;
pub use cpi::{invoke, invoke_signed};
pub use error::ProgramError;
pub use field_map::{FieldInfo, FieldMap};
#[cfg(feature = "hopper-native-backend")]
pub use instruction::CpiAccount;
pub use instruction::{InstructionAccount, InstructionView, Seed, Signer};
pub use layout::{HopperHeader, LayoutContract, LayoutInfo};
pub use result::ProgramResult;
pub use segment_borrow::{AccessKind, SegmentBorrow, SegmentBorrowRegistry};

pub const MAX_TX_ACCOUNTS: usize = compat::BACKEND_MAX_TX_ACCOUNTS;
pub const SUCCESS: u64 = compat::BACKEND_SUCCESS;

#[cfg(feature = "hopper-native-backend")]
#[doc(hidden)]
pub use hopper_native as __hopper_native;

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
        $crate::__hopper_native::hopper_program_entrypoint!($process_instruction, $maximum);

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
            unsafe { core::arch::asm!("unimp", options(noreturn)) }
        }
    };
}
