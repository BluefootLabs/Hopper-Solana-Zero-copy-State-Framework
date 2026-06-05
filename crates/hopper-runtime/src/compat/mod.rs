mod native;

#[doc(hidden)]
pub use native::{
    bridge_to_runtime, process_entrypoint, BackendAccountSlice, BackendAccountView, BackendAddress,
    BackendProgramResult, BACKEND_MAX_TX_ACCOUNTS, BACKEND_SUCCESS,
};

pub(crate) use native::*;

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
                accounts: $crate::compat::BackendAccountSlice<'_>,
                data: &[u8],
            ) -> $crate::compat::BackendProgramResult {
                $crate::compat::bridge_to_runtime(program_id, accounts, data, $process_instruction)
            }

            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            unsafe { $crate::compat::process_entrypoint::<$maximum>(input, __hopper_bridge) }
        }

    };
}
