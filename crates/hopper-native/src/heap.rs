//! The program's heap, as the allocator `default_allocator!` installs sees
//! it: how much is used, and checkpoints that give it back.
//!
//! A bump allocator never frees, so a handler that builds a temporary
//! `Vec` on every iteration of a loop runs out of heap no matter how small
//! each one is. A checkpoint fixes that without a different allocator:
//!
//! ```ignore
//! let mark = hopper::heap::mark();
//! for item in items {
//!     {
//!         let scratch = build(item);   // allocates
//!         apply(&scratch)?;
//!     }                                // every allocation is dropped here
//!     // SAFETY: nothing allocated since `mark` is still alive.
//!     unsafe { hopper::heap::release_to(mark) };
//! }
//! ```
//!
//! These functions read and write the allocator's cursor, the first word
//! of the VM's heap. They are for programs that installed
//! `default_allocator!`. Off chain there is no VM heap: `used` is zero and
//! a release does nothing.

pub use crate::entrypoint::HeapMark;

#[cfg(target_os = "solana")]
const HEAP: crate::entrypoint::BumpAllocator = crate::entrypoint::BumpAllocator {
    start: crate::entrypoint::HEAP_START_ADDRESS,
    len: crate::entrypoint::MAX_HEAP_LENGTH,
};

/// Bytes the allocator has handed out in this invocation, alignment
/// padding included.
#[inline]
pub fn used() -> usize {
    #[cfg(target_os = "solana")]
    {
        HEAP.used()
    }
    #[cfg(not(target_os = "solana"))]
    {
        0
    }
}

/// The heap's current position.
#[inline]
pub fn mark() -> HeapMark {
    #[cfg(target_os = "solana")]
    {
        HEAP.mark()
    }
    #[cfg(not(target_os = "solana"))]
    {
        HeapMark::host()
    }
}

/// Give back every allocation made since `mark` was taken.
///
/// # Safety
///
/// Nothing allocated after `mark` may be used again: every `Box`, `Vec`,
/// and `String` created since must already be dropped or forgotten. `mark`
/// must have been taken in this invocation.
#[inline]
pub unsafe fn release_to(mark: HeapMark) {
    #[cfg(target_os = "solana")]
    // SAFETY: this function's contract is the allocator's, forwarded.
    unsafe {
        HEAP.release_to(mark)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = mark;
    }
}
