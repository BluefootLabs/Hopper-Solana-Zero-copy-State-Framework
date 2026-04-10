//! Hopper Runtime -- canonical low-level runtime surface.
//!
//! Hopper Runtime is the single low-level API that all Hopper crates target.
//! Hopper Native is the primary backend. Pinocchio and solana-program are
//! compatibility backends only.
//!
//! Hopper Runtime owns the public type surface.
//! Backends are implementation details.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(feature = "solana-program-backend")]
extern crate alloc;

// ── Compile-time backend exclusivity ─────────────────────────────────

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

// ── Hopper-owned type modules ────────────────────────────────────────

pub mod error;
pub mod result;
pub mod address;
pub mod account;
pub mod borrow;
pub mod instruction;
pub mod layout;
pub mod context;
pub mod cpi;

// PDA module for compatibility backends (hopper-native-backend re-exports
// hopper_native::pda directly via the pub use block below).
#[cfg(any(feature = "pinocchio-backend", feature = "solana-program-backend"))]
pub mod pda;

pub use error::ProgramError;
pub use result::ProgramResult;
pub use address::Address;
pub use account::{AccountView, RemainingAccounts};
pub use borrow::{Ref, RefMut};
pub use instruction::{InstructionAccount, InstructionView, Seed, Signer};
pub use layout::LayoutContract;
pub use layout::{HopperHeader, LayoutInfo};
pub use context::Context;

// ── Instruction types (CpiAccount is hopper-native-backend only) ─────

#[cfg(feature = "hopper-native-backend")]
pub use instruction::CpiAccount;

// ══════════════════════════════════════════════════════════════════════
//  hopper-native backend
// ══════════════════════════════════════════════════════════════════════

#[cfg(feature = "hopper-native-backend")]
pub use hopper_native::{
    RuntimeAccount,
    log,
    pda,
    entrypoint,
    MAX_TX_ACCOUNTS,
    SUCCESS,
    MAX_PERMITTED_DATA_INCREASE,
    NOT_BORROWED,
};

#[cfg(feature = "hopper-native-backend")]
pub use hopper_native::{
    msg,
    lazy_entrypoint, cu_trace, cu_measure,
};

#[cfg(feature = "pinocchio-backend")]
pub const MAX_TX_ACCOUNTS: usize = pinocchio::MAX_TX_ACCOUNTS;

#[cfg(feature = "pinocchio-backend")]
pub const SUCCESS: u64 = pinocchio::SUCCESS;

#[cfg(feature = "solana-program-backend")]
pub use solana_program::entrypoint::SUCCESS;

#[cfg(feature = "solana-program-backend")]
pub const MAX_TX_ACCOUNTS: usize = 254;

// Hidden re-exports for macro hygiene.
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
///
/// ```ignore
/// require!(account.is_signer(), ProgramError::MissingRequiredSignature);
/// ```
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
///
/// ```ignore
/// require_eq!(account.owner(), &expected_owner, ProgramError::IncorrectProgramId);
/// ```
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
/// This macro owns the backend-to-runtime conversion so the user callback
/// always receives `hopper_runtime::Address` and `hopper_runtime::AccountView`.
/// `program_entrypoint!` is the authored alias to this macro.
///
/// ```ignore
/// hopper_entrypoint!(process_instruction);
///
/// fn process_instruction(
///     program_id: &Address,
///     accounts: &[AccountView],
///     data: &[u8],
/// ) -> ProgramResult {
///     Ok(())
/// }
/// ```
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
            #[inline(always)]
            fn __hopper_bridge(
                program_id: &$crate::compat::BackendAddress,
                accounts: &[$crate::compat::BackendAccountView],
                data: &[u8],
            ) -> $crate::compat::BackendProgramResult {
                $crate::compat::bridge_to_runtime(program_id, accounts, data, $process_instruction)
            }

            unsafe { $crate::compat::process_entrypoint::<$maximum>(input, __hopper_bridge) }
        }
    };
}

/// Declare the canonical Hopper program entrypoint.
///
/// This is the authored entrypoint macro Hopper programs should use. It is a
/// thin alias over `hopper_entrypoint!` so the runtime, not the backend,
/// owns the public program boundary.
#[macro_export]
macro_rules! program_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_entrypoint!($process_instruction);
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        $crate::hopper_entrypoint!($process_instruction, $maximum);
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

pub mod syscalls {
    #[cfg(target_os = "solana")]
    unsafe extern "C" {
        pub fn sol_log_(message: *const u8, len: u64);
        pub fn sol_log_64_(arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64);
        pub fn sol_log_compute_units_();
        pub fn sol_log_data(data: *const u8, data_len: u64);
        pub fn sol_invoke_signed_c(
            instruction_addr: *const u8,
            account_infos_addr: *const u8,
            account_infos_len: u64,
            signers_seeds_addr: *const u8,
            signers_seeds_len: u64,
        ) -> u64;
        pub fn sol_create_program_address(
            seeds_addr: *const u8,
            seeds_len: u64,
            program_id_addr: *const u8,
            address_addr: *mut u8,
        ) -> u64;
        pub fn sol_try_find_program_address(
            seeds_addr: *const u8,
            seeds_len: u64,
            program_id_addr: *const u8,
            address_addr: *mut u8,
            bump_seed_addr: *mut u8,
        ) -> u64;
        pub fn sol_sha256(vals: *const u8, val_len: u64, hash_result: *mut u8) -> u64;
        pub fn sol_keccak256(vals: *const u8, val_len: u64, hash_result: *mut u8) -> u64;
        pub fn sol_set_return_data(data: *const u8, length: u64);
        pub fn sol_get_return_data(data: *mut u8, length: u64, program_id: *mut u8) -> u64;
        pub fn sol_get_clock_sysvar(addr: *mut u8) -> u64;
        pub fn sol_get_rent_sysvar(addr: *mut u8) -> u64;
        pub fn sol_get_epoch_schedule_sysvar(addr: *mut u8) -> u64;
        pub fn sol_panic_(file: *const u8, len: u64, line: u64, column: u64) -> !;
        pub fn sol_memcpy_(dst: *mut u8, src: *const u8, n: u64);
        pub fn sol_memmove_(dst: *mut u8, src: *const u8, n: u64);
        pub fn sol_memcmp_(s1: *const u8, s2: *const u8, n: u64, result: *mut i32);
        pub fn sol_memset_(s: *mut u8, c: u8, n: u64);
        pub fn sol_get_stack_height() -> u64;
        pub fn sol_get_processed_sibling_instruction(
            index: u64,
            meta: *mut u8,
            program_id: *mut u8,
            data: *mut u8,
            accounts: *mut u8,
        ) -> u64;
        pub fn sol_get_last_restart_slot(addr: *mut u8) -> u64;
    }
}

pub mod syscall {
    #[cfg(target_os = "solana")]
    #[inline(always)]
    pub fn sol_log_compute_units() {
        unsafe { super::syscalls::sol_log_compute_units_() }
    }
}

// ── Innovation modules (hopper-native only) ──────────────────────────

#[cfg(feature = "hopper-native-backend")]
pub use hopper_native::{
    LazyContext,
    LamportSnapshot,
    BalanceSnapshot,
    DataFingerprint,
    LeU64, LeU32, LeU16,
    LeI64, LeI32, LeI16,
    LeBool, LeU128,
};

#[cfg(feature = "hopper-native-backend")]
pub use hopper_native::capability::{
    SignerView, WritableView, MutableView, OwnedView, ReadonlyView, ExecutableView,
};

#[cfg(feature = "hopper-native-backend")]
pub use hopper_native::project::{Projectable, self as project};

#[cfg(feature = "hopper-native-backend")]
pub use hopper_native::{
    budget::CuBudget,
    ReturnData,
    hash,
    sysvar,
    batch,
    lazy,
    budget,
    return_data,
    capability,
    wire,
    verify,
    lens,
    introspect,
    mem,
};

// ── System / Token program ───────────────────────────────────────────

pub mod system;
pub mod token;
