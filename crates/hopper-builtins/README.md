# hopper-builtins

Program-wide tuned memory intrinsics for the SBF target. `no_std`,
`no_alloc`, zero dependencies.

Under `cargo build-sbf` the platform-tools compiler-builtins archive ships
`memcmp`/`bcmp`/`memcpy`/`memset` as unconditional syscall shims: the SVM
`mem_op` cost model charges `max(10, n / 250)` CU per call plus call/shim
overhead, so a 3-byte compare pays the same 10 CU floor a 2.5 KB one does.
This crate overrides those symbols with a size dispatch (inline word loops
at or below 32 bytes, `sol_*` syscalls above) so small, hot operations
(discriminator checks, address compares, header stamps) stop paying the
syscall floor.

Opt in from the `hopper` facade with `--features builtins`; the anonymous
`use hopper_builtins as _;` import forces this rlib onto the linker line so
its `#[no_mangle]` definitions win over the platform archive. Host targets
export nothing. The override is SBF-only by construction.
