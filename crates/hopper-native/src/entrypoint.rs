//! Program entrypoint and BPF input deserialization.
//!
//! The `program_entrypoint!` macro declares the `entrypoint` function that
//! the Solana runtime calls. It deserializes the raw input buffer into
//! `AccountView` slices and delegates to the user's process function.

use core::mem::MaybeUninit;
use crate::address::Address;
use crate::account_view::{AccountView, RuntimeAccount};

/// Deserialize the raw BPF input buffer into AccountViews, instruction data,
/// and program ID.
///
/// # Safety
///
/// `input` must point to a valid Solana BPF input buffer.
///
/// # Returns
///
/// `(program_id, account_count, instruction_data)`
///
/// The accounts are written into `accounts[0..account_count]`.
#[inline]
pub unsafe fn deserialize<const MAX: usize>(
    input: *mut u8,
    accounts: &mut [MaybeUninit<AccountView>; MAX],
) -> (Address, usize, &'static [u8]) {
    unsafe {
        let mut offset = 0usize;

        // Number of accounts (u64 LE).
        let num_accounts = *(input.add(offset) as *const u64) as usize;
        offset += 8;

        let count = num_accounts.min(MAX);

        let mut i = 0;
        while i < count {
            let dup_marker = *input.add(offset);

            if dup_marker == u8::MAX {
                // Non-duplicate account: the RuntimeAccount header starts
                // at this position in the input buffer.
                let raw = input.add(offset) as *mut RuntimeAccount;
                accounts[i] = MaybeUninit::new(AccountView::new_unchecked(raw));

                // Skip past the RuntimeAccount header + data + padding + rent_epoch.
                let data_len = (*raw).data_len as usize;
                offset += core::mem::size_of::<RuntimeAccount>();
                offset += data_len;
                // Align to 8 bytes (BPF input buffer alignment).
                offset = (offset + 7) & !7;
                // rent_epoch (u64).
                offset += 8;
            } else {
                // Duplicate account: points to the same account as accounts[dup_marker].
                let original_idx = dup_marker as usize;
                // Skip the 8 bytes of padding after the duplicate marker.
                offset += 8;
                if original_idx < i {
                    accounts[i] = MaybeUninit::new(
                        accounts[original_idx].assume_init_read()
                    );
                } else {
                    // Invalid duplicate index: should not happen with valid input.
                    // Point to the first account as a safe fallback.
                    let first = accounts[0].as_ptr().read();
                    accounts[i] = MaybeUninit::new(first);
                }
            }

            i += 1;
        }

        // Skip any accounts beyond MAX.
        while i < num_accounts {
            let dup_marker = *input.add(offset);
            if dup_marker == u8::MAX {
                let raw = input.add(offset) as *const RuntimeAccount;
                let data_len = (*raw).data_len as usize;
                offset += core::mem::size_of::<RuntimeAccount>();
                offset += data_len;
                offset = (offset + 7) & !7;
                offset += 8;
            } else {
                offset += 8;
            }
            i += 1;
        }

        // Instruction data.
        let data_len = *(input.add(offset) as *const u64) as usize;
        offset += 8;
        let instruction_data = core::slice::from_raw_parts(input.add(offset), data_len);
        offset += data_len;

        // Program ID.
        let program_id_ptr = input.add(offset) as *const [u8; 32];
        let program_id = Address::new_from_array(*program_id_ptr);

        (program_id, count, instruction_data)
    }
}

/// Process the BPF entrypoint input.
///
/// This is the function called by the `program_entrypoint!` macro's
/// generated entrypoint.
///
/// # Safety
///
/// `input` must be the raw pointer provided by the Solana runtime.
#[inline]
pub unsafe fn process_entrypoint<const MAX: usize>(
    input: *mut u8,
    process_instruction: fn(&Address, &[AccountView], &[u8]) -> crate::ProgramResult,
) -> u64 {
    const UNINIT: MaybeUninit<AccountView> = MaybeUninit::uninit();
    let mut accounts = [UNINIT; 254]; // MAX_TX_ACCOUNTS

    // SAFETY: This is only used when MAX == MAX_TX_ACCOUNTS (254) in practice.
    // We use a fixed 254-element array to avoid const-generic array limitations.
    let accounts_ref: &mut [MaybeUninit<AccountView>; 254] = &mut accounts;

    // We need to reinterpret as the right size. For simplicity, always
    // deserialize into the 254-element array.
    let (program_id, count, instruction_data) = unsafe {
        deserialize::<254>(input, accounts_ref)
    };

    let account_slice = unsafe {
        core::slice::from_raw_parts(accounts_ref.as_ptr() as *const AccountView, count)
    };

    match process_instruction(&program_id, account_slice, instruction_data) {
        Ok(()) => crate::SUCCESS,
        Err(error) => error.into(),
    }
}

/// Declare the program entrypoint.
///
/// Generates the `extern "C" fn entrypoint` that the Solana runtime calls.
///
/// # Usage
///
/// ```ignore
/// use hopper_native::program_entrypoint;
///
/// program_entrypoint!(process_instruction);
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
macro_rules! program_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::program_entrypoint!($process_instruction, { $crate::MAX_TX_ACCOUNTS });
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
                $crate::entrypoint::deserialize::<$maximum>(input, &mut accounts)
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

/// Set up a no-op global allocator that panics on allocation.
///
/// Useful for `no_std` programs that must not allocate.
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

/// Default no_std panic handler that aborts.
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
