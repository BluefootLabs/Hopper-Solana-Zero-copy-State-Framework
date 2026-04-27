#[cfg(feature = "hopper-native-backend")]
mod native;
#[cfg(feature = "pinocchio-backend")]
mod pinocchio;
#[cfg(feature = "solana-program-backend")]
mod solana_program;

#[cfg(feature = "hopper-native-backend")]
#[doc(hidden)]
pub use native::{
    BACKEND_MAX_TX_ACCOUNTS,
    BACKEND_SUCCESS,
    BackendAccountView,
    BackendAddress,
    BackendProgramResult,
    bridge_to_runtime,
    process_entrypoint,
};

#[cfg(feature = "pinocchio-backend")]
#[doc(hidden)]
pub use pinocchio::{
    BACKEND_MAX_TX_ACCOUNTS,
    BACKEND_SUCCESS,
    BackendAccountView,
    BackendAddress,
    BackendProgramResult,
    bridge_to_runtime,
    process_entrypoint,
};

#[cfg(feature = "solana-program-backend")]
#[doc(hidden)]
pub use solana_program::{
    BACKEND_MAX_TX_ACCOUNTS,
    BACKEND_SUCCESS,
    BackendAccountView,
    BackendAddress,
    BackendProgramResult,
    bridge_to_runtime,
    process_entrypoint,
};

#[cfg(feature = "hopper-native-backend")]
pub(crate) use native::*;
#[cfg(feature = "pinocchio-backend")]
pub(crate) use pinocchio::*;
#[cfg(feature = "solana-program-backend")]
pub(crate) use solana_program::*;

#[doc(hidden)]
#[macro_export]
macro_rules! __hopper_compat_entrypoint {
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

        #[cfg(all(target_os = "solana", feature = "solana-program-backend"))]
        $crate::__solana_program::custom_heap_default!();

        #[cfg(all(target_os = "solana", feature = "solana-program-backend", not(feature = "custom-panic")))]
        #[no_mangle]
        fn custom_panic(_info: &core::panic::PanicInfo<'_>) {
            let _ = _info;
            loop {
                core::hint::spin_loop();
            }
        }
    };
}
