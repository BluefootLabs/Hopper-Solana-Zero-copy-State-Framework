//! Program entrypoint ownership for Hopper Native.
//!
//! This file is the only raw program-entry boundary owner in Hopper Native.
//! Loader input parsing lives in [`crate::raw_input`], while the public macros
//! below own the raw `entrypoint(input: *mut u8)` boundary and delegate into
//! Hopper callbacks.

use core::mem::MaybeUninit;

use crate::account_view::AccountView;
use crate::address::Address;
use crate::error::ProgramError;

/// Convert a handler's `ProgramError` into the Solana runtime's u64 return code.
///
/// Outlined `#[cold] #[inline(never)]` so an entrypoint's success tail lowers to
/// a bare `return SUCCESS` and the `ProgramError -> u64` mapping (the 25-arm
/// `From<ProgramError> for u64` match) is never inlined into the hot frame,
/// where it would add code size and stack traffic that every successful
/// invocation pays for. This mirrors Pinocchio's cold error outline.
///
/// The conversion is exactly `Into::<u64>::into(e)`, byte-for-byte identical to
/// the previous inline `error.into()`, so the runtime error codes are unchanged.
#[cold]
#[inline(never)]
pub fn err_to_u64(e: ProgramError) -> u64 {
    e.into()
}

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
        // SAFETY: `input` is the loader's input buffer (this function's
        // contract) and `accounts` has room for the 254 accounts the parser
        // may write.
        unsafe { crate::raw_input::deserialize_accounts::<254>(input, &mut accounts) };

    // Respect MAX: only pass up to MAX accounts to the callback.
    let effective_count = count.min(MAX);
    // SAFETY: The parser initialized the first `count` slots and
    // `effective_count <= count`; `MaybeUninit<AccountView>` has the layout
    // of `AccountView`.
    let account_slice = unsafe {
        core::slice::from_raw_parts(accounts.as_ptr() as *const AccountView<'_>, effective_count)
    };

    match process_instruction(program_id, account_slice, instruction_data) {
        Ok(()) => crate::SUCCESS,
        Err(error) => err_to_u64(error),
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

            // SAFETY: `input` is the loader's input buffer (the entrypoint's
            // contract) and `accounts` has `$maximum` slots, the bound the
            // parser is given.
            let (program_id, count, instruction_data) = unsafe {
                $crate::raw_input::deserialize_accounts::<$maximum>(input, &mut accounts)
            };

            match $process_instruction(
                program_id,
                // SAFETY: The parser initialized the first `count` slots;
                // `MaybeUninit<AccountView>` has the layout of `AccountView`.
                unsafe {
                    core::slice::from_raw_parts(
                        accounts.as_ptr() as *const $crate::AccountView<'_>,
                        count,
                    )
                },
                instruction_data,
            ) {
                Ok(()) => $crate::SUCCESS,
                Err(error) => $crate::entrypoint::err_to_u64(error),
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
/// Uses the SVM's second entrypoint register (`r2`), which carries a
/// direct pointer to instruction data under [SIMD-0321], letting the
/// entrypoint skip locating the instruction tail. Measured honestly
/// (2026-07-21, post the 2026-07-07 fused single-pass walk): the fused
/// scanning entrypoint already hops records by their `data_len` headers
/// without touching account data, so on programs whose accounts fit the
/// declared maximum the r2 path is CU-neutral (+/- 2 CU in controlled
/// A/Bs) and costs ~368 bytes for carrying both paths. The historical
/// "~30-40 CU" figure described the pre-fusion two-pass scanner. The r2
/// path earns its keep as the base of the SIMD-0449 O(1) account-pointer
/// table, and for instructions whose transaction carries many more
/// accounts than the program materializes.
///
/// # Feature gating (`simd-0321`)
///
/// SIMD-0321 is **activated on all three public clusters** (feature gate
/// `5xXZc66h4UdB6Yq7FzdBxBiRAFMMScMLwHxk2QZDaNZL`; mainnet-beta at slot
/// 410,400,000, 2026-04-01). Current agave sets `r2` unconditionally, so
/// builds may enable the feature for any cluster target; on a runtime
/// that ever leaves `r2` zero, the null-check below still falls back to
/// the scanning parse.
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
                // loader serialization layout; `Address` is a transparent
                // `[u8; 32]`, so a reference into the buffer is valid at any
                // offset and lives as long as the invocation.
                let program_id: &'static $crate::Address =
                    unsafe { &*(ix_data.add(ix_len) as *const $crate::Address) };

                if $crate::raw_input::SIMD_0449_TABLE_ENABLED {
                    // SIMD-0449 build: consume the runtime's appended
                    // pre-deduplicated account-pointer table, O(1)
                    // resolution plus one pointer copy per account. The
                    // gate is a `const`, so the untaken branch folds
                    // away entirely.
                    // SAFETY: the `simd-0449` feature asserts the SIMD
                    // is active on the target cluster (table present);
                    // `instruction_data`/`program_id` were derived from
                    // the SIMD-0321 r2 register above.
                    unsafe {
                        $crate::raw_input::deserialize_accounts_0449_into::<$maximum>(
                            input,
                            &mut accounts,
                            instruction_data,
                            program_id,
                        )
                    }
                } else {
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
                }
            };

            match $process_instruction(
                program_id,
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
                Err(error) => $crate::entrypoint::err_to_u64(error),
            }
        }
    };
}

/// Without the `simd-0321` feature the "fast" entrypoint is an alias for
/// the standard scanning entrypoint. The SIMD-0321 gate is live on every
/// public cluster (mainnet-beta 2026-04-01); the r2 form is sound to build
/// and stays opt-in only because it measured CU-neutral against the fused
/// scanning walk for ~368 bytes of extra `.text`. The two-argument r2 form
/// also null-checks the register and falls back to scanning, so it is safe
/// even where the gate is somehow inactive.
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
            // SAFETY: `input` is the loader's input buffer (the entrypoint's
            // contract), which is what `lazy_deserialize` requires.
            let mut ctx = unsafe { $crate::lazy::lazy_deserialize(input) };
            match $process(&mut ctx) {
                Ok(()) => $crate::SUCCESS,
                Err(error) => $crate::entrypoint::err_to_u64(error),
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
/// allocate immediately aborts the invocation through the SVM's abort syscall.
/// No experimental inline assembly is required. Returning null would also be
/// valid for `GlobalAlloc`; this allocator deliberately fails immediately.
#[macro_export]
macro_rules! no_allocator {
    () => {
        #[cfg(target_os = "solana")]
        mod __hopper_allocator {
            struct NoAlloc;

            unsafe impl core::alloc::GlobalAlloc for NoAlloc {
                unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
                    // SAFETY: abort accepts no pointers and never returns.
                    unsafe { $crate::syscalls::abort() }
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

/// Bytes of the heap's BOTTOM reserved as Hopper runtime scratch, starting
/// right after the [`BumpAllocator`] cursor word: the byte range
/// `[HEAP_START + 8, HEAP_START + 8 + HEAP_RUNTIME_RESERVED)`.
///
/// Why this exists: deployed SBF programs cannot carry writable sections,
/// the loader rejects `.bss`/`.data` outright (`WritableSectionNotSupported`),
/// so a `static mut` is not merely costly, it makes the program FAIL TO
/// LOAD. The only writable, per-invocation, zero-initialized memory a
/// program owns is this VM heap region. Hopper's instruction-scoped
/// runtime state (today: the lamport gate in
/// `hopper_runtime::write_policy`) therefore lives at the heap bottom,
/// which works precisely because the VM zeroes the region on every
/// invocation and every such structure is valid all-zero.
///
/// The [`BumpAllocator`] treats this range as out of bounds (its floor sits
/// above it), so `alloc` can never hand it out. Programs that install a
/// custom allocator over the heap must honor the same reservation if they
/// link any hopper-runtime feature that uses it.
pub const HEAP_RUNTIME_RESERVED: usize = 20 * 1024;

/// Bytes at the top of the reserved scratch that hold the per-invocation
/// Rent cache (`hopper_runtime::rent::live_rent`): the rate, the threshold
/// bits, and a loaded flag. All-zero is the empty cache, so the VM's
/// zeroed heap needs no initialization, exactly like the gate store below
/// it. The gate store and the touch log assert that they end before
/// [`RENT_CACHE_HEAP_OFFSET`].
pub const RENT_CACHE_BYTES: usize = 32;

/// Heap offset of the Rent cache, relative to [`HEAP_START_ADDRESS`].
pub const RENT_CACHE_HEAP_OFFSET: usize = HEAP_RUNTIME_RESERVED - RENT_CACHE_BYTES;

/// The largest heap a transaction can request with
/// `ComputeBudgetInstruction::RequestHeapFrame` (256 KiB).
pub const MAX_HEAP_LENGTH: usize = 256 * 1024;

/// A bump allocator over the SVM heap region.
///
/// Single pass and never frees, like the Solana SDK's and pinocchio's: the
/// first word of the heap holds the cursor and `dealloc` does nothing. It
/// is the right allocator for the cold paths of a program that wants
/// `alloc` (a `Vec` while building a CPI) while the hot path allocates
/// nothing. For programs that must never allocate, prefer [`no_allocator!`]
/// so a stray allocation traps.
///
/// # It grows upward, so a requested heap frame is usable
///
/// The cursor starts just above Hopper's reserved scratch and moves up. A
/// program that declares a larger heap with `default_allocator!(heap = N)`
/// can use all of it when the transaction carries
/// `RequestHeapFrame(N)`. When the transaction does not, the first 32 KiB
/// still work exactly as before, because small allocations land at the
/// bottom either way; only an allocation that reaches past the memory the
/// VM mapped faults, and the VM, not the allocator, stops it. An allocator
/// that grows downward from the top of a 256 KiB region would fault on its
/// first allocation in every transaction that forgot the request.
///
/// # The last allocation resizes in place
///
/// `realloc` of the most recent allocation moves the cursor and copies
/// nothing. A `Vec` that grows while nothing else allocates, the usual
/// case in a handler, costs its final size and not the sum of every size
/// it passed through.
///
/// # Checkpoints
///
/// [`mark`](Self::mark) and [`release_to`](Self::release_to) give a loop
/// the heap back on every iteration, which a bump allocator otherwise
/// cannot do.
///
/// Install it with `default_allocator!`.
pub struct BumpAllocator {
    /// Heap region start address.
    pub start: usize,
    /// Heap region length in bytes.
    pub len: usize,
}

/// A position of the heap cursor, taken with [`BumpAllocator::mark`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapMark(usize);

impl HeapMark {
    /// The mark a host build hands out: there is no VM heap off chain.
    #[cfg(not(target_os = "solana"))]
    pub(crate) const fn host() -> Self {
        Self(0)
    }
}

impl BumpAllocator {
    /// An allocator over the SVM heap, `len` bytes long. `len` is the heap
    /// the program expects: 32 KiB by default, up to 256 KiB when its
    /// transactions request a heap frame. Anything else is a compile
    /// error in a `static`.
    pub const fn new(len: usize) -> Self {
        assert!(
            len >= HEAP_LENGTH && len <= MAX_HEAP_LENGTH,
            "the heap is between 32 KiB and 256 KiB"
        );
        assert!(
            len.is_multiple_of(1024),
            "a heap frame is a multiple of 1 KiB"
        );
        Self {
            start: HEAP_START_ADDRESS,
            len,
        }
    }

    /// The lowest address the allocator hands out: above the cursor word
    /// and the Hopper runtime scratch ([`HEAP_RUNTIME_RESERVED`]).
    #[inline(always)]
    const fn floor(&self) -> usize {
        self.start + core::mem::size_of::<usize>() + HEAP_RUNTIME_RESERVED
    }

    #[inline(always)]
    fn cursor(&self) -> usize {
        // SAFETY: `start` is the heap's first word, which this allocator
        // owns as its cursor; the VM zeroes the heap, and a program runs on
        // one thread.
        let pos = unsafe { *(self.start as *const usize) };
        // Zero is the VM's fresh heap: nothing allocated yet.
        if pos == 0 {
            self.floor()
        } else {
            pos
        }
    }

    #[inline(always)]
    fn set_cursor(&self, pos: usize) {
        // SAFETY: as in `cursor`; the word is written only here.
        unsafe { *(self.start as *mut usize) = pos };
    }

    /// Bytes handed out so far, alignment padding included.
    #[inline]
    pub fn used(&self) -> usize {
        self.cursor() - self.floor()
    }

    /// Bytes left before the declared end of the heap.
    #[inline]
    pub fn remaining(&self) -> usize {
        (self.start + self.len).saturating_sub(self.cursor())
    }

    /// The current position of the heap, to return to with
    /// [`release_to`](Self::release_to).
    #[inline]
    pub fn mark(&self) -> HeapMark {
        HeapMark(self.cursor())
    }

    /// Give back every allocation made since `mark` was taken.
    ///
    /// # Safety
    ///
    /// Nothing allocated after `mark` may be used again: every `Box`,
    /// `Vec`, and `String` created since must already be dropped or
    /// forgotten. `mark` must come from this allocator in this invocation.
    #[inline]
    pub unsafe fn release_to(&self, mark: HeapMark) {
        let pos = mark.0;
        if pos >= self.floor() && pos <= self.cursor() {
            self.set_cursor(pos);
        }
    }
}

// SAFETY: every pointer returned lies in `[floor, start + len)`, is aligned
// as the layout asks, and is never returned twice while live: the cursor
// only moves past what was handed out, except in `realloc` of the most
// recent block (which stays where it is) and in `release_to` (whose caller
// promises the released blocks are dead). A program runs on one thread, so
// the cursor word is never accessed concurrently.
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    // SAFETY: `GlobalAlloc::alloc` is `unsafe` by the trait's signature.
    // The body is integer arithmetic on the cursor with every sum checked;
    // it asks nothing of the caller beyond a valid `Layout`, whose
    // alignment is a nonzero power of two by construction.
    #[inline]
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let mask = layout.align() - 1;
        let start = match self.cursor().checked_add(mask) {
            Some(bumped) => bumped & !mask,
            None => return core::ptr::null_mut(),
        };
        let end = match start.checked_add(layout.size()) {
            Some(end) => end,
            None => return core::ptr::null_mut(),
        };
        if end > self.start + self.len {
            return core::ptr::null_mut();
        }
        self.set_cursor(end);
        start as *mut u8
    }

    // SAFETY: `GlobalAlloc::dealloc` is `unsafe` by the trait's signature.
    // This body reads and writes nothing, so it holds for any pointer and
    // layout: a bump allocator reclaims its memory when the instruction
    // ends.
    #[inline]
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}

    // SAFETY: `GlobalAlloc::realloc` is `unsafe` by the trait's signature.
    // The caller vouches that `ptr` is a live block of this allocator with
    // layout `layout`; the body moves the cursor or copies `layout.size()`
    // bytes out of that block, and nothing else.
    #[inline]
    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        let block = ptr as usize;
        // The most recent allocation ends at the cursor: it grows or
        // shrinks where it is.
        if block.checked_add(layout.size()) == Some(self.cursor()) {
            return match block.checked_add(new_size) {
                Some(end) if end <= self.start + self.len => {
                    self.set_cursor(end);
                    ptr
                }
                _ => core::ptr::null_mut(),
            };
        }
        if new_size <= layout.size() {
            // An older block that shrinks keeps its place.
            return ptr;
        }
        // SAFETY: `new_size` is nonzero (it exceeds the old size) and the
        // alignment is the old layout's, which the caller vouches for.
        let new_layout =
            unsafe { core::alloc::Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: forwarded `GlobalAlloc` contract.
        let new_ptr = unsafe { self.alloc(new_layout) };
        if !new_ptr.is_null() {
            // SAFETY: the old block is valid for `layout.size()` bytes, the
            // new one for more, and a fresh block cannot overlap a live one.
            unsafe { core::ptr::copy_nonoverlapping(ptr, new_ptr, layout.size()) };
        }
        new_ptr
    }
}

/// Install the default bump allocator over the SVM heap region.
///
/// Opt-in counterpart to [`no_allocator!`]: use this when a program needs
/// `alloc` (e.g. heap `Vec`/`String` on a cold path) while keeping the
/// zero-copy hot path allocation-free. Never frees within an instruction;
/// the whole heap is reclaimed when the instruction returns.
///
/// `default_allocator!(heap = 128 * 1024)` declares a larger heap, up to
/// 256 KiB. The transactions that need it carry
/// `ComputeBudgetInstruction::RequestHeapFrame` with the same size; the
/// ones that allocate less than 32 KiB work without it.
#[macro_export]
macro_rules! default_allocator {
    () => {
        $crate::default_allocator!(heap = $crate::HEAP_LENGTH);
    };
    (heap = $len:expr) => {
        #[cfg(target_os = "solana")]
        #[global_allocator]
        static ALLOCATOR: $crate::BumpAllocator = $crate::BumpAllocator::new($len);
    };
}

/// Whether a panic reports where it happened (`panic-location`).
pub const PANIC_REPORTS_LOCATION: bool = cfg!(feature = "panic-location");
/// Whether a panic logs its message (`panic-message`).
pub const PANIC_REPORTS_MESSAGE: bool = cfg!(feature = "panic-message");

/// What the panic handler does, as a function so the features that shape
/// it are this crate's and not the calling program's.
///
/// - By default: abort. Nothing is logged and no formatting code is
///   linked, so a release build pays nothing for having a handler.
/// - With `panic-location`: the runtime logs `panicked at file:line:col`
///   (through `sol_panic_`) before the transaction fails. The file names
///   are the only cost, a few hundred bytes of read-only data.
/// - With `panic-message`: the panic's message is logged first, cut at
///   the last whole character that fits 256 bytes. This links the
///   formatting machinery, so it is for debugging builds.
///
/// Turn the features on through `hopper` (`features = ["panic-location"]`)
/// while a program is being debugged, and off again for the build that
/// ships.
#[cfg(target_os = "solana")]
#[inline(always)]
pub fn report_panic(info: &core::panic::PanicInfo<'_>) -> ! {
    #[cfg(feature = "panic-message")]
    {
        use core::fmt::Write;
        let mut buf = [0u8; 256];
        let mut writer = crate::log::StackWriter::new(&mut buf);
        let _ = write!(writer, "{}", info.message());
        crate::log::log(writer.as_str());
    }
    #[cfg(feature = "panic-location")]
    if let Some(location) = info.location() {
        let file = location.file();
        // SAFETY: the pointer and the length describe one live `&str`;
        // the syscall logs it with the line and column and never returns.
        unsafe {
            crate::syscalls::sol_panic_(
                file.as_ptr(),
                file.len() as u64,
                location.line() as u64,
                location.column() as u64,
            )
        }
    }
    let _ = info;
    // SAFETY: abort accepts no pointers and never returns.
    unsafe { crate::syscalls::abort() }
}

/// The `no_std` panic handler.
///
/// Aborts through the SVM's abort syscall, without experimental inline
/// assembly or a compute-consuming spin loop; the runtime rolls the
/// instruction back. With the `panic-location` feature it first reports
/// the file, line, and column, and with `panic-message` the message. See
/// [`report_panic`](crate::entrypoint::report_panic).
#[macro_export]
macro_rules! nostd_panic_handler {
    () => {
        #[cfg(target_os = "solana")]
        #[panic_handler]
        fn panic(info: &core::panic::PanicInfo) -> ! {
            $crate::entrypoint::report_panic(info)
        }
    };
}

#[cfg(test)]
mod entrypoint_tail_tests {
    extern crate std;

    use std::vec;
    use std::vec::Vec;

    use super::*;

    /// Serialize a zero-account loader frame: `u64` account count (0), then the
    /// `u64` ix-data length prefix, the ix-data bytes, and the 32-byte program
    /// id. Returns an 8-aligned `u64` backing (matching `MM_INPUT_START`).
    fn build_zero_account_frame(ix_data: &[u8], program_id: [u8; 32]) -> Vec<u64> {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&0u64.to_le_bytes()); // account_count = 0
        buf.extend_from_slice(&(ix_data.len() as u64).to_le_bytes());
        buf.extend_from_slice(ix_data);
        buf.extend_from_slice(&program_id);
        let mut words = vec![0u64; buf.len().div_ceil(8)];
        // SAFETY: `words` has at least `buf.len()` bytes of capacity and the
        // regions do not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), words.as_mut_ptr() as *mut u8, buf.len());
        }
        words
    }

    fn ok_handler<'a>(
        _: &'a Address,
        _: &'a [AccountView<'a>],
        _: &'a [u8],
    ) -> crate::ProgramResult {
        Ok(())
    }

    fn custom_err_handler<'a>(
        _: &'a Address,
        _: &'a [AccountView<'a>],
        _: &'a [u8],
    ) -> crate::ProgramResult {
        Err(ProgramError::Custom(4242))
    }

    fn builtin_err_handler<'a>(
        _: &'a Address,
        _: &'a [AccountView<'a>],
        _: &'a [u8],
    ) -> crate::ProgramResult {
        Err(ProgramError::MissingRequiredSignature)
    }

    #[test]
    fn ok_returns_bare_success_zero() {
        let mut frame = build_zero_account_frame(&[1, 2, 3], [7u8; 32]);
        // SAFETY: `frame` is a well-formed, 8-aligned zero-account loader frame.
        let code = unsafe { process_entrypoint::<4>(frame.as_mut_ptr() as *mut u8, ok_handler) };
        assert_eq!(code, 0);
        assert_eq!(code, crate::SUCCESS);
    }

    #[test]
    fn custom_err_maps_through_cold_outline_unchanged() {
        let mut frame = build_zero_account_frame(&[], [0u8; 32]);
        // SAFETY: well-formed, 8-aligned zero-account loader frame.
        let code =
            unsafe { process_entrypoint::<4>(frame.as_mut_ptr() as *mut u8, custom_err_handler) };
        // Cold outline must equal the direct `From<ProgramError> for u64` mapping.
        assert_eq!(code, u64::from(ProgramError::Custom(4242)));
        assert_eq!(code, err_to_u64(ProgramError::Custom(4242)));
        assert_eq!(code, 4242);
    }

    #[test]
    fn builtin_err_maps_through_cold_outline_unchanged() {
        let mut frame = build_zero_account_frame(&[], [0u8; 32]);
        // SAFETY: well-formed, 8-aligned zero-account loader frame.
        let code =
            unsafe { process_entrypoint::<4>(frame.as_mut_ptr() as *mut u8, builtin_err_handler) };
        assert_eq!(code, u64::from(ProgramError::MissingRequiredSignature));
        assert_eq!(code, err_to_u64(ProgramError::MissingRequiredSignature));
    }

    /// The cold outline is a byte-for-byte alias of `From<ProgramError> for u64`
    /// across the full variant space (custom-zero, custom, and builtins).
    #[test]
    fn err_to_u64_matches_from_impl_for_all_variants() {
        let cases = [
            ProgramError::Custom(0),
            ProgramError::Custom(1),
            ProgramError::Custom(u32::MAX),
            ProgramError::InvalidArgument,
            ProgramError::MissingRequiredSignature,
            ProgramError::AccountBorrowFailed,
            ProgramError::ArithmeticOverflow,
            ProgramError::IncorrectAuthority,
        ];
        for e in cases {
            assert_eq!(err_to_u64(e.clone()), u64::from(e));
        }
    }
}

#[cfg(test)]
mod allocator_tests {
    extern crate std;

    use super::*;
    use core::alloc::{GlobalAlloc, Layout};
    use std::vec;
    use std::vec::Vec;

    /// A zeroed, 8-aligned region that stands in for the VM's heap, and an
    /// allocator over it.
    fn test_heap(len: usize) -> (Vec<u64>, BumpAllocator) {
        let mut backing = vec![0u64; len / 8];
        let allocator = BumpAllocator {
            start: backing.as_mut_ptr() as usize,
            len,
        };
        (backing, allocator)
    }

    const SCRATCH_END: usize = 8 + HEAP_RUNTIME_RESERVED;

    fn layout(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    #[test]
    fn dealloc_gives_nothing_back_and_touches_nothing() {
        let (backing, heap) = test_heap(HEAP_LENGTH);
        // SAFETY: a test allocation from a private region.
        let a = unsafe { heap.alloc(layout(64, 8)) };
        let used = heap.used();
        // SAFETY: `a` came from this allocator with this layout.
        unsafe { heap.dealloc(a, layout(64, 8)) };
        assert_eq!(heap.used(), used);
        // SAFETY: as above.
        let b = unsafe { heap.alloc(layout(64, 8)) };
        assert_eq!(b as usize, a as usize + 64, "the freed block is not reused");
        // Only the cursor word was ever written.
        assert!(backing[1..].iter().all(|word| *word == 0));
    }

    #[test]
    fn allocations_start_above_the_scratch_and_grow_upward() {
        let (backing, heap) = test_heap(HEAP_LENGTH);
        assert_eq!(heap.used(), 0);
        assert_eq!(heap.remaining(), HEAP_LENGTH - SCRATCH_END);
        // SAFETY: a test allocation from a private region, never freed.
        let a = unsafe { heap.alloc(layout(10, 1)) } as usize;
        // SAFETY: as above.
        let b = unsafe { heap.alloc(layout(8, 8)) } as usize;
        // SAFETY: as above.
        let c = unsafe { heap.alloc(layout(1, 1)) } as usize;
        assert_eq!(a, heap.start + SCRATCH_END);
        assert_eq!(
            b,
            heap.start + SCRATCH_END + 16,
            "aligned up past the 10 bytes"
        );
        assert_eq!(c, b + 8);
        assert_eq!(heap.used(), 25);
        // The scratch region was not written.
        assert!(backing[1..SCRATCH_END / 8].iter().all(|w| *w == 0));
    }

    #[test]
    fn every_alignment_is_honoured() {
        let (_backing, heap) = test_heap(HEAP_LENGTH);
        for shift in 0..8 {
            let align = 1usize << shift;
            // SAFETY: a test allocation from a private region.
            let odd = unsafe { heap.alloc(layout(3, 1)) };
            assert!(!odd.is_null());
            // SAFETY: as above.
            let ptr = unsafe { heap.alloc(layout(5, align)) } as usize;
            assert_eq!(ptr % align, 0, "alignment {align}");
        }
    }

    #[test]
    fn the_heap_is_exhausted_at_its_declared_end_and_not_before() {
        let (_backing, heap) = test_heap(HEAP_LENGTH);
        let room = HEAP_LENGTH - SCRATCH_END;
        // SAFETY: a test allocation from a private region.
        let all = unsafe { heap.alloc(layout(room, 1)) };
        assert!(!all.is_null());
        assert_eq!(heap.remaining(), 0);
        // SAFETY: as above.
        assert!(unsafe { heap.alloc(layout(1, 1)) }.is_null());
        // A refused allocation leaves the cursor where it was.
        assert_eq!(heap.used(), room);

        let (_backing, heap) = test_heap(HEAP_LENGTH);
        // SAFETY: as above.
        assert!(unsafe { heap.alloc(layout(room + 1, 1)) }.is_null());
        assert_eq!(heap.used(), 0);
        // A size that would wrap the address space is refused too.
        // SAFETY: as above.
        assert!(unsafe { heap.alloc(layout(isize::MAX as usize - 64, 1)) }.is_null());
    }

    #[test]
    fn a_requested_heap_frame_is_usable_to_its_end() {
        let (_backing, heap) = test_heap(MAX_HEAP_LENGTH);
        // More than the default heap holds in one allocation.
        // SAFETY: a test allocation from a private region.
        let big = unsafe { heap.alloc(layout(200 * 1024, 8)) };
        assert!(!big.is_null());
        // SAFETY: the block is 200 KiB of the private region.
        unsafe { core::ptr::write_bytes(big, 0xA5, 200 * 1024) };
        assert_eq!(heap.remaining(), MAX_HEAP_LENGTH - SCRATCH_END - 200 * 1024);
        // A small allocation made first lands in the first 32 KiB, which
        // every transaction has.
        let (_backing, heap) = test_heap(MAX_HEAP_LENGTH);
        // SAFETY: as above.
        let small = unsafe { heap.alloc(layout(64, 8)) } as usize;
        assert!(small + 64 <= heap.start + HEAP_LENGTH);
    }

    #[test]
    fn the_last_allocation_resizes_in_place() {
        let (_backing, heap) = test_heap(HEAP_LENGTH);
        let first = layout(16, 8);
        // SAFETY: test allocations from a private region; each block is
        // used only within its size.
        unsafe {
            let ptr = heap.alloc(first);
            core::ptr::write_bytes(ptr, 7, 16);
            let grown = heap.realloc(ptr, first, 64);
            assert_eq!(grown, ptr, "grown where it is");
            assert_eq!(heap.used(), 64);
            assert_eq!(*grown.add(15), 7);
            let shrunk = heap.realloc(grown, layout(64, 8), 8);
            assert_eq!(shrunk, ptr);
            assert_eq!(heap.used(), 8, "the tail was given back");

            // Growing past the end is refused and changes nothing.
            let refused = heap.realloc(shrunk, layout(8, 8), HEAP_LENGTH);
            assert!(refused.is_null());
            assert_eq!(heap.used(), 8);
        }
    }

    #[test]
    fn an_older_allocation_moves_when_it_grows() {
        let (_backing, heap) = test_heap(HEAP_LENGTH);
        let old = layout(8, 8);
        // SAFETY: test allocations from a private region; each block is
        // used only within its size.
        unsafe {
            let a = heap.alloc(old);
            core::ptr::write_bytes(a, 0x11, 8);
            let b = heap.alloc(layout(8, 8));
            core::ptr::write_bytes(b, 0x22, 8);
            let moved = heap.realloc(a, old, 24);
            assert!(
                moved as usize > b as usize,
                "a new block above the newer one"
            );
            assert_eq!(core::slice::from_raw_parts(moved, 8), &[0x11; 8]);
            assert_eq!(
                core::slice::from_raw_parts(b, 8),
                &[0x22; 8],
                "the neighbour is intact"
            );
            // Shrinking an older block keeps it in place.
            assert_eq!(heap.realloc(b, layout(8, 8), 4), b);
        }
    }

    #[test]
    fn a_checkpoint_gives_the_heap_back() {
        let (_backing, heap) = test_heap(HEAP_LENGTH);
        // SAFETY: test allocations from a private region; nothing
        // allocated after the mark is used after the release.
        unsafe {
            let kept = heap.alloc(layout(32, 8));
            let mark = heap.mark();
            let mut previous: *mut u8 = core::ptr::null_mut();
            for _ in 0..1_000 {
                let scratch = heap.alloc(layout(4096, 8));
                assert!(!scratch.is_null(), "the loop never runs out");
                if !previous.is_null() {
                    assert_eq!(scratch, previous, "the same bytes every time");
                }
                previous = scratch;
                heap.release_to(mark);
            }
            assert_eq!(heap.used(), 32);
            assert_eq!(heap.mark(), mark);
            assert!(!kept.is_null());
            // A mark from above the cursor or below the floor is ignored.
            let after = heap.alloc(layout(8, 8));
            let high = heap.mark();
            heap.release_to(mark);
            heap.release_to(high);
            assert_eq!(heap.used(), 32, "a stale mark cannot move the cursor up");
            assert!(!after.is_null());
        }
    }

    #[test]
    fn the_declared_heap_is_checked() {
        assert_eq!(BumpAllocator::new(HEAP_LENGTH).len, HEAP_LENGTH);
        assert_eq!(
            BumpAllocator::new(MAX_HEAP_LENGTH).start,
            HEAP_START_ADDRESS
        );
        for bad in [
            0,
            HEAP_LENGTH - 1024,
            HEAP_LENGTH + 1,
            MAX_HEAP_LENGTH + 1024,
        ] {
            assert!(
                std::panic::catch_unwind(|| BumpAllocator::new(bad)).is_err(),
                "{bad}"
            );
        }
    }
}
