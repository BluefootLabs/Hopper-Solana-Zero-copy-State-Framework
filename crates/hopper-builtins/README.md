# hopper-builtins

Program-wide tuned memory intrinsics for the SBF target. `no_std`,
`no_alloc`, zero dependencies.

Under `cargo build-sbf` the platform-tools compiler-builtins archive ships
`memcmp`/`memcpy`/`memmove`/`memset` as unconditional syscall shims: the SVM
`mem_op` cost model charges `max(10, n / 250)` CU per call plus call/shim
overhead, so a 3-byte compare pays the same 10 CU floor a 2.5 KB one does.
This crate overrides `memcmp`, `bcmp`, `memcpy`, and `memset` (not
`memmove`) with a size dispatch: inline word loops at or below 32 bytes
and `sol_*` syscalls above. It affects runtime-length memory operations. LLVM already inlines fixed-size comparisons such as
32-byte address equality, so those call sites do not use these overrides.

Opt in from the `hopper` facade with `--features builtins`. The current SBF
toolchain defines the same symbols strongly, so the build must also pass
`RUSTFLAGS="-C link-arg=--allow-multiple-definition"`. Verify the emitted ELF
before release; a stock build without that linker flag fails on duplicate
symbols. Host targets export nothing. The override is SBF-only.
