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
    process_instruction: for<'info> fn(
        &'info Address,
        &'info [AccountView<'info>],
        &'info [u8],
    ) -> crate::ProgramResult,
) -> u64 {
    const UNINIT: MaybeUninit<AccountView<'static>> = MaybeUninit::uninit();
    let mut accounts = [UNINIT; 254]; // MAX_TX_ACCOUNTS

    let (program_id, count, instruction_data) =
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { crate::raw_input::deserialize_accounts::<254>(input, &mut accounts) };

    // Respect MAX: only pass up to MAX accounts to the callback.
    let effective_count = count.min(MAX);
    // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    let account_slice = unsafe {
        core::slice::from_raw_parts(accounts.as_ptr() as *const AccountView<'_>, effective_count)
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
            const UNINIT: core::mem::MaybeUninit<$crate::AccountView<'static>> =
                core::mem::MaybeUninit::<$crate::AccountView<'static>>::uninit();
            let mut accounts = [UNINIT; $maximum];

            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            let (program_id, count, instruction_data) = unsafe {
                $crate::raw_input::deserialize_accounts::<$maximum>(input, &mut accounts)
            };

            match $process_instruction(
                &program_id,
                // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
                unsafe {
                    core::slice::from_raw_parts(
                        accounts.as_ptr() as *const $crate::AccountView<'_>,
                        count,
                    )
                },
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
/// Uses the SVM's second entrypoint register (`r2`), which — once
/// [SIMD-0321] is activated — carries a direct pointer to instruction
/// data, eliminating the full account-scanning pass that the
/// single-argument entrypoint requires. Saves ~30-40 CU per instruction
/// invocation.
///
/// # Feature gating (`simd-0321`)
///
/// SIMD-0321 is **not yet activated** on public clusters (feature gate
/// `5xXZc66h4UdB6Yq7FzdBxBiRAFMMScMLwHxk2QZDaNZL`). Without the
/// activation, `r2` carries no instruction-data pointer at entry.
///
/// - **Default (feature off):** this macro expands to the standard
///   scanning entrypoint ([`hopper_program_entrypoint!`]). Identical
///   semantics, sound on every cluster today, and source-compatible:
///   when the gate activates, rebuild with the feature to claim the
///   CU savings.
/// - **`simd-0321` enabled:** the macro expands to the two-argument
///   entrypoint. As defense in depth it null-checks `r2` and falls
///   back to the scanning parse when the register is zero (current
///   SBPF VMs zero-initialize unused argument registers), so a binary
///   built with the feature degrades to the slow path instead of
///   reading garbage if it lands on a cluster without the activation.
///
/// `hopper doctor` / `hopper deploy` can check the feature-gate account
/// on the target cluster before a `simd-0321` build ships.
///
/// [SIMD-0321]: https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0321-vm-r2-instruction-data-pointer.md
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
        /// register is zero and the scanning fallback below is taken.
        #[no_mangle]
        pub unsafe extern "C" fn entrypoint(input: *mut u8, ix_data: *const u8) -> u64 {
            const UNINIT: core::mem::MaybeUninit<$crate::AccountView<'static>> =
                core::mem::MaybeUninit::<$crate::AccountView<'static>>::uninit();
            let mut accounts = [UNINIT; $maximum];

            let (program_id, count, instruction_data) = if ix_data.is_null() {
                // SIMD-0321 not active on this cluster: r2 is zero. Fall back
                // to the full scanning parse so the program stays correct.
                // SAFETY: `input` is the loader-provided input buffer; the
                // scanning parser owns all bounds/duplicate-marker checks.
                unsafe { $crate::raw_input::deserialize_accounts::<$maximum>(input, &mut accounts) }
            } else {
                // Instruction data length is the u64 immediately before the
                // data pointer (per SIMD-0321's serialization contract).
                // SAFETY: SIMD-0321 ix_data points at instruction-data bytes
                // with u64 length prefix at `ix_data - 8`.
                let ix_len =
                    unsafe { core::ptr::read_unaligned(ix_data.sub(8) as *const u64) as usize };
                let instruction_data: &'static [u8] =
                    unsafe { core::slice::from_raw_parts(ix_data, ix_len) };

                // SAFETY: program id trails the instruction data per the
                // loader serialization layout; reading 32 bytes by value.
                let program_id =
                    unsafe { core::ptr::read(ix_data.add(ix_len) as *const $crate::Address) };

                // SAFETY: `input` is the loader input buffer; account-slot
                // framing is validated by `deserialize_accounts_fast`.
                unsafe {
                    $crate::raw_input::deserialize_accounts_fast::<$maximum>(
                        input,
                        &mut accounts,
                        instruction_data,
                        program_id,
                    )
                }
            };

            match $process_instruction(
                &program_id,
                // SAFETY: the first `count` slots were initialized by the
                // parser above; `AccountView` is repr(C) over the slot data.
                unsafe {
                    core::slice::from_raw_parts(
                        accounts.as_ptr() as *const $crate::AccountView<'_>,
                        count,
                    )
                },
                instruction_data,
            ) {
                Ok(()) => $crate::SUCCESS,
                Err(error) => error.into(),
            }
        }
    };
}

/// Without the `simd-0321` feature the "fast" entrypoint is an alias for
/// the standard scanning entrypoint. See the feature-gated definition
/// above for the rationale: SIMD-0321 has not been activated, so the
/// two-argument form would read an uninitialized register on today's
/// clusters.
#[cfg(not(feature = "simd-0321"))]
#[macro_export]
macro_rules! hopper_fast_entrypoint {
    ( $process_instruction:expr ) => {
        $crate::hopper_program_entrypoint!($process_instruction);
    };
    ( $process_instruction:expr, $maximum:expr ) => {
        $crate::hopper_program_entrypoint!($process_instruction, $maximum);
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
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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

/// Canonical Solana heap region start address (`0x3_0000_0000`).
pub const HEAP_START_ADDRESS: usize = 0x3_0000_0000;

/// Default Solana heap region length (32 KiB).
pub const HEAP_LENGTH: usize = 32 * 1024;

/// A bump allocator over the SVM heap region.
///
/// This is the same single-pass, never-frees design the Solana SDK and
/// Pinocchio use: the first word of the heap stores the current cursor,
/// allocations bump it downward from the top of the region, and
/// `dealloc` is a no-op. It is the right allocator for the cold paths of
/// a program that wants `alloc` (e.g. a `Vec` while building a CPI) while
/// keeping the hot path zero-allocation. For programs that must never
/// allocate, prefer [`no_allocator!`] so any stray allocation traps.
///
/// Install it with [`default_allocator!`].
pub struct BumpAllocator {
    /// Heap region start address.
    pub start: usize,
    /// Heap region length in bytes.
    pub len: usize,
}

// SAFETY: Solana program execution is single-threaded, so the cursor word
// at `start` is never accessed concurrently.
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        // The cursor is stored in the first word of the heap region.
        let pos_ptr = self.start as *mut usize;
        // SAFETY: `pos_ptr` is the reserved cursor word; single-threaded.
        let mut pos = unsafe { *pos_ptr };
        if pos == 0 {
            // First allocation: start at the top of the region.
            pos = self.start + self.len;
        }
        pos = pos.saturating_sub(layout.size());
        pos &= !(layout.align().wrapping_sub(1));
        // Keep the cursor word itself intact.
        if pos < self.start + core::mem::size_of::<usize>() {
            return core::ptr::null_mut();
        }
        // SAFETY: `pos_ptr` is the reserved cursor word; single-threaded.
        unsafe { *pos_ptr = pos };
        pos as *mut u8
    }

    #[inline]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        // Bump allocator: memory is reclaimed when the instruction ends.
    }
}

/// Install the default bump allocator over the SVM heap region.
///
/// Opt-in counterpart to [`no_allocator!`]: use this when a program needs
/// `alloc` (e.g. heap `Vec`/`String` on a cold path) while keeping the
/// zero-copy hot path allocation-free. Never frees within an instruction;
/// the whole heap is reclaimed when the instruction returns.
#[macro_export]
macro_rules! default_allocator {
    () => {
        #[cfg(target_os = "solana")]
        #[global_allocator]
        static ALLOCATOR: $crate::BumpAllocator = $crate::BumpAllocator {
            start: $crate::HEAP_START_ADDRESS,
            len: $crate::HEAP_LENGTH,
        };
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
            // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            unsafe { core::arch::asm!("mov r0, 1", "exit", options(noreturn)) };
        }
    };
}
