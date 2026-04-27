//! Program entrypoint ownership for Hopper Native.
//!
//! This file is the only raw program-entry boundary owner in Hopper Native.
//! Loader input parsing lives in [`crate::raw_input`], while the public macros
//! below own the raw `entrypoint(input: *mut u8)` boundary and delegate into
//! Hopper callbacks.

use core::mem::MaybeUninit;

use crate::account_view::AccountView;
use crate::address::Address;

/// Process the BPF entrypoint input.
///
/// This is the function called by the canonical Hopper Native entrypoint macro's
/// generated entrypoint.
///
/// # Safety
///
/// `input` must be the raw pointer provided by the Solana runtime.
#[inline(always)]
pub unsafe fn process_entrypoint<const MAX: usize>(
    input: *mut u8,
    process_instruction: fn(&Address, &[AccountView], &[u8]) -> crate::ProgramResult,
) -> u64 {
    const UNINIT: MaybeUninit<AccountView> = MaybeUninit::uninit();
    let mut accounts = [UNINIT; 254]; // MAX_TX_ACCOUNTS

    let (program_id, count, instruction_data) =
        unsafe { crate::raw_input::deserialize_accounts::<254>(input, &mut accounts) };

    // Respect MAX: only pass up to MAX accounts to the callback.
    let effective_count = count.min(MAX);
    let account_slice = unsafe {
        core::slice::from_raw_parts(accounts.as_ptr() as *const AccountView, effective_count)
    };

    match process_instruction(&program_id, account_slice, instruction_data) {
        Ok(()) => crate::SUCCESS,
        Err(error) => error.into(),
    }
}

/// Declare the canonical Hopper Native program entrypoint.
///
/// Generates the `extern "C" fn entrypoint` that the Solana runtime calls.
/// `program_entrypoint!` remains available as a backward-compatible alias.
///
/// # Usage
///
/// ```ignore
/// use hopper_native::hopper_program_entrypoint;
///
/// hopper_program_entrypoint!(process_instruction);
///
/// pub fn process_instruction(
///     program_id: &Address,
///     accounts: &[AccountView],
///     instruction_data: &[u8],
/// ) -> ProgramResult {
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! hopper_program_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_program_entrypoint!($process_instruction, { $crate::MAX_TX_ACCOUNTS });
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8) -> u64 {
            const UNINIT: core::mem::MaybeUninit<$crate::AccountView> =
                core::mem::MaybeUninit::<$crate::AccountView>::uninit();
            let mut accounts = [UNINIT; $maximum];

            let (program_id, count, instruction_data) = unsafe {
                $crate::raw_input::deserialize_accounts::<$maximum>(input, &mut accounts)
            };

            match $process_instruction(
                &program_id,
                unsafe { core::slice::from_raw_parts(accounts.as_ptr() as _, count) },
                instruction_data,
            ) {
                Ok(()) => $crate::SUCCESS,
                Err(error) => error.into(),
            }
        }
    };
}

/// Backward-compatible alias for `hopper_program_entrypoint!`.
#[macro_export]
macro_rules! program_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_program_entrypoint!($process_instruction);
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        $crate::hopper_program_entrypoint!($process_instruction, $maximum);
    };
}

/// Declare a fast two-argument Hopper Native program entrypoint.
///
/// Uses the SVM's second entrypoint register, which provides a direct
/// pointer to instruction data, eliminating the full account-scanning pass
/// that the single-argument entrypoint requires. Saves ~30-40 CU per
/// instruction invocation.
///
/// The SVM has provided the second argument since runtime ~1.17.
///
/// # Usage
///
/// ```ignore
/// use hopper_native::hopper_fast_entrypoint;
///
/// hopper_fast_entrypoint!(process_instruction, 3);
///
/// pub fn process_instruction(
///     program_id: &Address,
///     accounts: &[AccountView],
///     instruction_data: &[u8],
/// ) -> ProgramResult {
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! hopper_fast_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_fast_entrypoint!($process_instruction, { $crate::MAX_TX_ACCOUNTS });
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer
        /// and `ix_data` points to the instruction data with its u64 length
        /// stored at offset -8.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8, ix_data: *const u8) -> u64 {
            const UNINIT: core::mem::MaybeUninit<$crate::AccountView> =
                core::mem::MaybeUninit::<$crate::AccountView>::uninit();
            let mut accounts = [UNINIT; $maximum];

            // Instruction data length is the u64 immediately before the data pointer.
            let ix_len = unsafe { *(ix_data.sub(8) as *const u64) as usize };
            let instruction_data: &'static [u8] =
                unsafe { core::slice::from_raw_parts(ix_data, ix_len) };

            // Program ID immediately follows instruction data in the SVM buffer.
            let program_id =
                unsafe { core::ptr::read(ix_data.add(ix_len) as *const $crate::Address) };

            let (program_id, count, instruction_data) = unsafe {
                $crate::raw_input::deserialize_accounts_fast::<$maximum>(
                    input,
                    &mut accounts,
                    instruction_data,
                    program_id,
                )
            };

            match $process_instruction(
                &program_id,
                unsafe { core::slice::from_raw_parts(accounts.as_ptr() as _, count) },
                instruction_data,
            ) {
                Ok(()) => $crate::SUCCESS,
                Err(error) => error.into(),
            }
        }
    };
}

/// Backward-compatible alias for `hopper_fast_entrypoint!`.
#[macro_export]
macro_rules! fast_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_fast_entrypoint!($process_instruction);
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        $crate::hopper_fast_entrypoint!($process_instruction, $maximum);
    };
}

/// Declare the canonical lazy program entrypoint that defers account parsing.
#[macro_export]
macro_rules! hopper_lazy_entrypoint {
    ( $process:expr ) => {
        /// # Safety
        ///
        /// Called by the Solana runtime; `input` is a valid BPF input buffer.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8) -> u64 {
            let mut ctx = unsafe { $crate::lazy::lazy_deserialize(input) };
            match $process(&mut ctx) {
                Ok(()) => $crate::SUCCESS,
                Err(error) => error.into(),
            }
        }
    };
}

/// Backward-compatible alias for `hopper_lazy_entrypoint!`.
#[macro_export]
macro_rules! lazy_entrypoint {
    ( $process:expr ) => {
        $crate::hopper_lazy_entrypoint!($process);
    };
}

/// Set up a no-op global allocator that aborts on allocation.
///
/// Useful for `no_std` programs that must not allocate. Any attempt to
/// allocate will immediately abort the program rather than returning a
/// null pointer (which violates the `GlobalAlloc` contract).
#[macro_export]
macro_rules! no_allocator {
    () => {
        #[cfg(target_os = "solana")]
        mod __hopper_allocator {
            struct NoAlloc;

            unsafe impl core::alloc::GlobalAlloc for NoAlloc {
                unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
                    // Abort: returning null_mut violates the GlobalAlloc
                    // contract and causes UB. Abort is the correct response
                    // for a no-alloc program.
                    core::arch::asm!("mov r0, 1", "exit", options(noreturn));
                }
                unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
            }

            #[global_allocator]
            static ALLOCATOR: NoAlloc = NoAlloc;
        }
    };
}

/// Default no_std panic handler that aborts immediately.
///
/// On BPF, uses inline assembly to return error code 1 (aborts the
/// program). This is cheaper than `spin_loop()` which would burn CU
/// until the runtime kills the program.
#[macro_export]
macro_rules! nostd_panic_handler {
    () => {
        #[cfg(target_os = "solana")]
        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {
            // Abort immediately, spin_loop() would burn CU indefinitely.
            unsafe { core::arch::asm!("mov r0, 1", "exit", options(noreturn)) };
        }
    };
}
