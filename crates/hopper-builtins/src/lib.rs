//! Program-wide tuned memory intrinsics for the SBF target.
//!
//! Under `cargo build-sbf` the platform-tools compiler-builtins archive
//! ships `memcmp`/`memcpy`/`memmove`/`memset` as unconditional syscall
//! shims. The SVM `mem_op` cost model charges `max(10, n / 250)` CU per
//! call plus a ~4–6 instruction call/shim overhead — so a 3-byte compare
//! pays the same 10-CU base as a 250-byte one. For small `n` an inline
//! `u64` word loop is strictly cheaper; above ~32 bytes the syscall's
//! flat base amortizes and wins. This crate overrides the C symbols with
//! that dispatch:
//!
//! - `n <= 32`: inline word loop (`u64::read_unaligned` + byte tail),
//!   no syscall.
//! - `n > 32`: the corresponding `sol_*` syscall.
//!
//! Overridden symbols: `memcmp`, `bcmp`, `memcpy`, `memset`. `memmove`
//! is deliberately **not** overridden — the toolchain shim (a straight
//! `sol_memmove_` call) is already the right shape, and an inline
//! forward loop would be incorrect for overlapping ranges.
//!
//! ## What this reprices — and what it does not
//!
//! LLVM inline-expands *fixed-size* small compares (including 32-byte
//! `[u8; 32]` equality) before any `memcmp` symbol is ever emitted, so
//! this crate does **not** change the cost of `Address` equality; the
//! manual word-wise `PartialEq` on `Address` carries that. The value
//! here is every *runtime-length* call site: `core` slice comparison
//! over long `&[u8]`, formatting/collection internals, and third-party
//! crates, which otherwise pay the 10-CU syscall base plus shim overhead
//! even for tiny `n`.
//!
//! ## Opt-in wiring
//!
//! Enable the `builtins` feature on `hopper-lang`. The facade then pulls
//! this crate in with `use hopper_builtins as _;` under
//! `target_os = "solana"`: the anonymous import forces the rlib onto the
//! linker command line ahead of the platform-tools compiler-builtins
//! archive.
//!
//! ## Linkage requirement (measured 2026-07-07, platform-tools v1.53)
//!
//! The toolchain's shims are GLOBAL (not weak) definitions, and
//! `rust-lld` **refuses the strong-vs-strong collision outright** — a
//! plain `cargo build-sbf --features builtins` fails with duplicate
//! `memcmp`/`memcpy`/`memset` symbols. Programs opting in must pass
//!
//! ```text
//! RUSTFLAGS="-C link-arg=--allow-multiple-definition" cargo build-sbf ...
//! ```
//!
//! With the flag, first-definition-wins ordering resolves to THIS crate
//! (verified in the built `.so`: the `__HOPPER_BUILTINS` marker is
//! present, the override bodies are ours, and the only call inside them
//! is the `sol_*` syscall relocation — no self-calls, see the
//! `no_builtins` note below). This also means the same crate shape
//! cannot work at all on a stock cargo-build-sbf route without the flag
//! — which applies equally to Quasar's reference implementation.

#![no_std]
// Without `no_builtins`, LLVM's loop-idiom recognition is allowed to
// rewrite the inline word/byte loops below back into `memset`/`memcpy`
// libcalls — which resolve to these very overrides, producing infinite
// self-recursion and deterministic stack exhaustion on-chain. This is
// the same reason `compiler-builtins` marks itself `no_builtins`; the
// hazard was reproduced empirically on rustc 1.96 at opt-level 2/3
// (adversarial review, 2026-07-07).
#![no_builtins]
#![deny(unsafe_op_in_unsafe_fn)]

/// Bytes per word-loop step.
const WORD: usize = core::mem::size_of::<u64>();

/// Word-at-a-time `memcmp` with a correct `<0 / 0 / >0` sign.
///
/// The words are read in native byte order; on a mismatch the in-memory
/// bytes are re-interpreted big-endian (`u64::from_be_bytes`) so the
/// integer comparison agrees with lexicographic per-byte order on every
/// host endianness. This is the ordering-correctness the naive
/// "`return 1` on word mismatch" shape gets wrong.
///
/// # Safety
///
/// - `a` and `b` must each be valid for reads of `n` bytes.
/// - The two ranges may overlap (this function only reads).
#[cfg_attr(not(any(test, target_os = "solana")), allow(dead_code))]
pub(crate) unsafe fn memcmp_inline(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0usize;
    while i + WORD <= n {
        // SAFETY: caller guarantees `a` and `b` are readable for `n`
        // bytes and the loop condition gives `i + 8 <= n`, so both
        // unaligned u64 reads stay in bounds; `read_unaligned` imposes
        // no alignment requirement.
        let (wa, wb) = unsafe {
            (
                (a.add(i) as *const u64).read_unaligned(),
                (b.add(i) as *const u64).read_unaligned(),
            )
        };
        if wa != wb {
            // `to_ne_bytes` recovers the in-memory byte order;
            // `from_be_bytes` makes byte 0 most significant, i.e.
            // lexicographic order.
            let la = u64::from_be_bytes(wa.to_ne_bytes());
            let lb = u64::from_be_bytes(wb.to_ne_bytes());
            return if la < lb { -1 } else { 1 };
        }
        i += WORD;
    }
    while i < n {
        // SAFETY: `i < n` and the caller guarantees `n` readable bytes
        // behind both pointers.
        let (ba, bb) = unsafe { (*a.add(i), *b.add(i)) };
        if ba != bb {
            return ba as i32 - bb as i32;
        }
        i += 1;
    }
    0
}

/// Equality-only compare: returns `0` if the ranges are byte-equal,
/// nonzero otherwise (the `bcmp` contract — no ordering promised).
///
/// # Safety
///
/// - `a` and `b` must each be valid for reads of `n` bytes.
/// - The two ranges may overlap (this function only reads).
#[cfg_attr(not(any(test, target_os = "solana")), allow(dead_code))]
pub(crate) unsafe fn bcmp_inline(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0usize;
    while i + WORD <= n {
        // SAFETY: caller guarantees `n` readable bytes behind both
        // pointers and `i + 8 <= n`; unaligned reads are permitted.
        let (wa, wb) = unsafe {
            (
                (a.add(i) as *const u64).read_unaligned(),
                (b.add(i) as *const u64).read_unaligned(),
            )
        };
        if wa != wb {
            return 1;
        }
        i += WORD;
    }
    while i < n {
        // SAFETY: `i < n`, both pointers readable for `n` bytes.
        if unsafe { *a.add(i) != *b.add(i) } {
            return 1;
        }
        i += 1;
    }
    0
}

/// Word-at-a-time forward copy (word body + byte tail).
///
/// Forward only: this matches the `memcpy` contract, where the caller
/// guarantees the ranges do not overlap. Overlapping copies must go
/// through `memmove` (not overridden by this crate).
///
/// # Safety
///
/// - `src` must be valid for reads of `n` bytes; `dst` must be valid
///   for writes of `n` bytes.
/// - The two ranges must not overlap.
#[cfg_attr(not(any(test, target_os = "solana")), allow(dead_code))]
pub(crate) unsafe fn memcpy_inline(dst: *mut u8, src: *const u8, n: usize) {
    let mut i = 0usize;
    while i + WORD <= n {
        // SAFETY: caller guarantees `src` readable / `dst` writable for
        // `n` bytes, non-overlapping, and `i + 8 <= n`; unaligned
        // access is explicit.
        unsafe {
            let w = (src.add(i) as *const u64).read_unaligned();
            (dst.add(i) as *mut u64).write_unaligned(w);
        }
        i += WORD;
    }
    while i < n {
        // SAFETY: `i < n`; same read/write validity as above.
        unsafe { *dst.add(i) = *src.add(i) };
        i += 1;
    }
}

/// Word-splat fill (word body + byte tail).
///
/// # Safety
///
/// - `s` must be valid for writes of `n` bytes.
#[cfg_attr(not(any(test, target_os = "solana")), allow(dead_code))]
pub(crate) unsafe fn memset_inline(s: *mut u8, c: u8, n: usize) {
    let splat = u64::from_ne_bytes([c; WORD]);
    let mut i = 0usize;
    while i + WORD <= n {
        // SAFETY: caller guarantees `s` writable for `n` bytes and
        // `i + 8 <= n`; unaligned write is explicit.
        unsafe { (s.add(i) as *mut u64).write_unaligned(splat) };
        i += WORD;
    }
    while i < n {
        // SAFETY: `i < n`, `s` writable for `n` bytes.
        unsafe { *s.add(i) = c };
        i += 1;
    }
}

// ---------------------------------------------------------------------
// SBF-only intrinsic overrides.
// ---------------------------------------------------------------------

#[cfg(target_os = "solana")]
mod sbf {
    use super::{bcmp_inline, memcmp_inline, memcpy_inline, memset_inline};

    /// Dispatch threshold: at or below this byte count the inline word
    /// loop beats the syscall's `max(10, n / 250)` CU base + shim
    /// overhead; above it the flat base amortizes and the syscall wins.
    const INLINE_MAX: usize = 32;

    // Dynamic-relocation syscall declarations, same shape as
    // `hopper-native/src/syscalls.rs` (the loader resolves these by
    // symbol name at load time — no transmute-by-hash indirection).
    extern "C" {
        /// Compare `n` bytes. Sets `*result` to <0, 0, or >0.
        fn sol_memcmp_(s1: *const u8, s2: *const u8, n: u64, result: *mut i32);
        /// Copy `n` bytes from `src` to `dst` (non-overlapping).
        fn sol_memcpy_(dst: *mut u8, src: *const u8, n: u64);
        /// Fill `n` bytes with `c`.
        fn sol_memset_(s: *mut u8, c: u8, n: u64);
    }

    /// Link marker: CI greps built `.so` files for this symbol to
    /// confirm the override archive actually made it into the link
    /// (see the duplicate-symbol caveat in the crate docs).
    #[no_mangle]
    pub static __HOPPER_BUILTINS: u8 = 1;

    /// C `memcmp` override: inline word loop for `n <= 32`, the
    /// `sol_memcmp_` syscall above.
    ///
    /// # Safety
    ///
    /// C `memcmp` contract: `s1` and `s2` are each valid for reads of
    /// `n` bytes.
    #[no_mangle]
    pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
        // Volatile read so the marker static is observably used and the
        // linker cannot strip it from the final `.so`.
        // SAFETY: reading a live static u8 through a reference is
        // always valid.
        let _ = unsafe { core::ptr::read_volatile(&__HOPPER_BUILTINS) };
        if n <= INLINE_MAX {
            // SAFETY: forwarded from the C memcmp contract above.
            unsafe { memcmp_inline(s1, s2, n) }
        } else {
            let mut result: i32 = 0;
            // SAFETY: pointers readable for `n` bytes per the C
            // contract; `result` is a live local out-param.
            unsafe { sol_memcmp_(s1, s2, n as u64, &mut result) };
            result
        }
    }

    /// C `bcmp` override (equality only): returns 0 iff the ranges are
    /// byte-equal; any nonzero value otherwise.
    ///
    /// # Safety
    ///
    /// C `bcmp` contract: `s1` and `s2` are each valid for reads of
    /// `n` bytes.
    #[no_mangle]
    pub unsafe extern "C" fn bcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
        if n <= INLINE_MAX {
            // SAFETY: forwarded from the C bcmp contract above.
            unsafe { bcmp_inline(s1, s2, n) }
        } else {
            let mut result: i32 = 0;
            // SAFETY: pointers readable for `n` bytes per the C
            // contract; `result` is a live local out-param. bcmp only
            // needs zero/nonzero, which the memcmp result satisfies.
            unsafe { sol_memcmp_(s1, s2, n as u64, &mut result) };
            result
        }
    }

    /// C `memcpy` override: inline word copy for `n <= 32`, the
    /// `sol_memcpy_` syscall above. Returns `dst` per the C contract.
    ///
    /// # Safety
    ///
    /// C `memcpy` contract: `src` valid for reads of `n` bytes, `dst`
    /// valid for writes of `n` bytes, ranges non-overlapping.
    #[no_mangle]
    pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        if n <= INLINE_MAX {
            // SAFETY: forwarded from the C memcpy contract above
            // (including non-overlap, which the forward loop requires).
            unsafe { memcpy_inline(dst, src, n) };
        } else {
            // SAFETY: forwarded from the C memcpy contract above.
            unsafe { sol_memcpy_(dst, src, n as u64) };
        }
        dst
    }

    /// C `memset` override: inline word splat for `n <= 32`, the
    /// `sol_memset_` syscall above. Returns `s` per the C contract.
    ///
    /// # Safety
    ///
    /// C `memset` contract: `s` valid for writes of `n` bytes.
    #[no_mangle]
    pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
        if n <= INLINE_MAX {
            // SAFETY: forwarded from the C memset contract above.
            unsafe { memset_inline(s, c as u8, n) };
        } else {
            // SAFETY: forwarded from the C memset contract above.
            unsafe { sol_memset_(s, c as u8, n as u64) };
        }
        s
    }
}

// ---------------------------------------------------------------------
// Host-side differential tests.
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference byte-loop memcmp (the semantics we must match).
    fn memcmp_ref(a: &[u8], b: &[u8]) -> i32 {
        debug_assert_eq!(a.len(), b.len());
        for i in 0..a.len() {
            if a[i] != b[i] {
                return a[i] as i32 - b[i] as i32;
            }
        }
        0
    }

    fn sign(x: i32) -> i32 {
        if x < 0 {
            -1
        } else if x > 0 {
            1
        } else {
            0
        }
    }

    /// A varied fill so word contents differ across the buffer.
    fn pattern(n: usize) -> [u8; 64] {
        let mut buf = [0u8; 64];
        for (i, byte) in buf.iter_mut().enumerate().take(n) {
            *byte = (i.wrapping_mul(37).wrapping_add(11) % 251) as u8;
        }
        buf
    }

    #[test]
    fn memcmp_inline_matches_reference_at_every_mismatch_index() {
        for n in 0..=40usize {
            let a = pattern(n);
            // Equal buffers compare as 0 at every length (incl. 0).
            // SAFETY: both slices are readable for `n` bytes.
            assert_eq!(
                unsafe { memcmp_inline(a.as_ptr(), a.as_ptr(), n) },
                0,
                "n={n}"
            );

            for k in 0..n {
                for delta in [1u8, 0x80, 0xFF] {
                    let mut b = a;
                    b[k] = a[k].wrapping_add(delta);
                    if b[k] == a[k] {
                        continue;
                    }
                    let expect = sign(memcmp_ref(&a[..n], &b[..n]));
                    // SAFETY: both slices are readable for `n` bytes.
                    let got = sign(unsafe { memcmp_inline(a.as_ptr(), b.as_ptr(), n) });
                    assert_eq!(got, expect, "n={n} k={k} delta={delta}");
                    // And the mirrored direction.
                    // SAFETY: both slices are readable for `n` bytes.
                    let got_rev = sign(unsafe { memcmp_inline(b.as_ptr(), a.as_ptr(), n) });
                    assert_eq!(got_rev, -expect, "n={n} k={k} delta={delta} (rev)");
                }
            }
        }
    }

    #[test]
    fn memcmp_inline_boundary_lengths() {
        // Explicit spot checks at the word-loop boundaries.
        for n in [0usize, 7, 8, 9, 31, 32, 33] {
            let a = pattern(n);
            let mut b = a;
            if n > 0 {
                b[n - 1] = a[n - 1].wrapping_add(1);
            }
            let expect = sign(memcmp_ref(&a[..n], &b[..n]));
            // SAFETY: both slices are readable for `n` bytes.
            let got = sign(unsafe { memcmp_inline(a.as_ptr(), b.as_ptr(), n) });
            assert_eq!(got, expect, "n={n}");
        }
    }

    #[test]
    fn memcmp_inline_is_lexicographic_not_native_word_order() {
        // Regression against the naive native-endian word compare: on a
        // little-endian host the u64 of `a` is larger, but byte 0
        // decides lexicographically and byte 0 says a < b.
        let a = [0u8, 0, 0, 0, 0, 0, 0, 0xFF];
        let b = [1u8, 0, 0, 0, 0, 0, 0, 0x00];
        // SAFETY: both arrays are readable for 8 bytes.
        assert!(unsafe { memcmp_inline(a.as_ptr(), b.as_ptr(), 8) } < 0);
        // SAFETY: both arrays are readable for 8 bytes.
        assert!(unsafe { memcmp_inline(b.as_ptr(), a.as_ptr(), 8) } > 0);
    }

    #[test]
    fn bcmp_inline_zero_iff_equal() {
        for n in 0..=40usize {
            let a = pattern(n);
            // SAFETY: both slices are readable for `n` bytes.
            assert_eq!(
                unsafe { bcmp_inline(a.as_ptr(), a.as_ptr(), n) },
                0,
                "n={n}"
            );
            for k in 0..n {
                let mut b = a;
                b[k] ^= 0x55;
                // SAFETY: both slices are readable for `n` bytes.
                let got = unsafe { bcmp_inline(a.as_ptr(), b.as_ptr(), n) };
                assert_ne!(got, 0, "n={n} k={k}");
            }
        }
    }

    #[test]
    fn memcpy_inline_byte_exact_and_bounded() {
        for n in 0..=40usize {
            let src = pattern(n);
            let mut dst = [0xAAu8; 64];
            // SAFETY: `src` readable and `dst` writable for `n` bytes;
            // distinct arrays, so non-overlapping.
            unsafe { memcpy_inline(dst.as_mut_ptr(), src.as_ptr(), n) };
            assert_eq!(&dst[..n], &src[..n], "n={n}");
            assert!(dst[n..].iter().all(|&b| b == 0xAA), "n={n} wrote past end");
        }
    }

    #[test]
    fn memset_inline_byte_exact_and_bounded() {
        for n in 0..=40usize {
            for c in [0u8, 0x5A, 0xFF] {
                let mut buf = [0xAAu8; 64];
                // SAFETY: `buf` is writable for `n` bytes.
                unsafe { memset_inline(buf.as_mut_ptr(), c, n) };
                assert!(buf[..n].iter().all(|&b| b == c), "n={n} c={c}");
                assert!(
                    buf[n..].iter().all(|&b| b == 0xAA),
                    "n={n} c={c} wrote past end"
                );
            }
        }
    }
}
