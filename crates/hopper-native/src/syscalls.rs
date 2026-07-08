//! Raw Solana syscall declarations.
//!
//! These are the functions provided by the Solana BPF/SBF runtime. Only
//! available when compiling for `target_os = "solana"`.
//!
//! # Dual-mode dispatch (SIMD-0178 readiness)
//!
//! Every declaration goes through [`define_syscall!`], which emits one of two
//! equivalent forms depending on the active build:
//!
//! * **Default (relocation).** With neither the `static-syscalls` cargo feature
//!   nor the `static-syscalls` target-feature enabled, the macro emits exactly
//!   the historical `extern "C"` declaration. This is byte-for-byte the same
//!   relocation-based syscall the framework has always used, so the default
//!   build is unchanged.
//!
//! * **Static (sBPF v3 / SIMD-0178).** When `static-syscalls` is enabled, the
//!   macro instead emits an `unsafe fn` that transmutes the murmur32 hash of the
//!   syscall name to the matching `extern "C"` function pointer and calls it.
//!   This is the static-syscall ABI the sBPF v3 loader uses in place of syscall
//!   relocations (SIMD-0178), matching the reference `solana-define-syscall`
//!   implementation.
//!
//! The feature is **opt-in and default-off**: it exists so Hopper is ready to
//! build for the v3 loader, without changing anything for today's v0..v2
//! targets. The hash is computed at const-eval time from the syscall name; the
//! constants are pinned by host tests against known Agave values (e.g.
//! `sol_memcmp_` → `0x5FDC_DE31`).

/// murmur3 (32-bit) hash of a syscall name, seed `0`.
///
/// This is the exact construction the Agave sBPF loader and the reference
/// `solana-define-syscall` crate use to derive the static-syscall dispatch key
/// under SIMD-0178. Kept `const` so the hash folds at compile time inside the
/// generated syscall stubs.
#[doc(hidden)]
pub const fn sys_hash(name: &str) -> usize {
    murmur3_32(name.as_bytes(), 0) as usize
}

/// `const`-evaluable murmur3-32 over `buf` with the given `seed`.
///
/// Mirrors the reference implementation in `solana-define-syscall` verbatim so
/// that Hopper's static-syscall dispatch keys are identical to Agave's.
const fn murmur3_32(buf: &[u8], seed: u32) -> u32 {
    const fn pre_mix(buf: [u8; 4]) -> u32 {
        u32::from_le_bytes(buf)
            .wrapping_mul(0xcc9e2d51)
            .rotate_left(15)
            .wrapping_mul(0x1b873593)
    }

    let mut hash = seed;

    let mut i = 0;
    while i < buf.len() / 4 {
        let buf = [buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]];
        hash ^= pre_mix(buf);
        hash = hash.rotate_left(13);
        hash = hash.wrapping_mul(5).wrapping_add(0xe6546b64);

        i += 1;
    }

    match buf.len() % 4 {
        0 => {}
        1 => {
            hash ^= pre_mix([buf[i * 4], 0, 0, 0]);
        }
        2 => {
            hash ^= pre_mix([buf[i * 4], buf[i * 4 + 1], 0, 0]);
        }
        3 => {
            hash ^= pre_mix([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], 0]);
        }
        _ => { /* unreachable */ }
    }

    hash ^= buf.len() as u32;
    hash ^= hash.wrapping_shr(16);
    hash = hash.wrapping_mul(0x85ebca6b);
    hash ^= hash.wrapping_shr(13);
    hash = hash.wrapping_mul(0xc2b2ae35);
    hash ^= hash.wrapping_shr(16);

    hash
}

/// Declare a Solana syscall in a build-mode-agnostic way.
///
/// See the module docs for the two emitted forms. Two surface syntaxes are
/// supported:
///
/// ```ignore
/// // 1. Rust name == the runtime symbol name (native decls).
/// define_syscall!(fn sol_log_(message: *const u8, len: u64));
///
/// // 2. Rust name differs from the symbol name; give the symbol explicitly
/// //    so the static-mode hash is taken over the real syscall name.
/// define_syscall!(fn syscall_sol_memcpy(dst: *mut u8, src: *const u8, n: u64);
///     link_name = "sol_memcpy_");
/// ```
///
/// Invocations are expected to be gated on `#[cfg(target_os = "solana")]` by the
/// caller, exactly as the historical `extern "C"` blocks were. The macro itself
/// does not add that gate, which lets host tests exercise both emitted forms.
#[macro_export]
macro_rules! define_syscall {
    // ── Relocation-mode emitter (default build) ──────────────────────
    // Byte-identical to the historical `extern "C"` declaration.
    (@reloc $(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident: $typ:ty),* $(,)?) -> $ret:ty) => {
        extern "C" {
            $(#[$attr])*
            $vis fn $name($($arg: $typ),*) -> $ret;
        }
    };
    (@reloc $(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident: $typ:ty),* $(,)?) -> $ret:ty; link_name = $sym:literal) => {
        extern "C" {
            #[link_name = $sym]
            $(#[$attr])*
            $vis fn $name($($arg: $typ),*) -> $ret;
        }
    };

    // ── Static-mode emitter (SIMD-0178 / sBPF v3) ────────────────────
    (@static $(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident: $typ:ty),* $(,)?) -> $ret:ty; hash = $sym:expr) => {
        $(#[$attr])*
        #[inline]
        $vis unsafe fn $name($($arg: $typ),*) -> $ret {
            // Forcing the hash through an enum discriminant guarantees it is
            // computed in a `const` context (matching the reference crate).
            #[repr(usize)]
            enum Syscall {
                Code = $crate::syscalls::sys_hash($sym),
            }
            // SAFETY: Under SIMD-0178 the sBPF v3 loader resolves syscalls by a
            // static dispatch key equal to murmur32(name); `sys_hash` computes
            // exactly that key at const-eval. Transmuting the key to a function
            // pointer whose signature mirrors the relocation declaration this
            // arm replaces, then calling it, is the loader's defined static-call
            // ABI. The invariant that makes this sound is that the hash equals
            // Agave's dispatch key — pinned by the host tests to known
            // constants — and that the pointer type matches the syscall's real
            // C signature (identical to the relocation decl). The caller upholds
            // the syscall's own pointer/length contract, unchanged from the
            // relocation path.
            let syscall: extern "C" fn($($arg: $typ),*) -> $ret =
                unsafe { core::mem::transmute(Syscall::Code) };
            // `syscall` is a *safe* `extern "C" fn` pointer, so the call itself
            // needs no `unsafe` (matches the reference `solana-define-syscall`).
            syscall($($arg),*)
        }
    };

    // ── Public surface: explicit runtime symbol via leading attribute ─
    // The Rust name differs from the runtime symbol; give the symbol as a
    // leading `#[link_name = "..."]`. This arm must precede the generic arm
    // so the symbol is not swallowed by `$(#[$attr])*`. Written as a normal
    // `#[attr] fn` item so `rustfmt` leaves it intact (no inner separators).
    (#[link_name = $sym:literal] $(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident: $typ:ty),* $(,)?) -> $ret:ty) => {
        #[cfg(not(any(feature = "static-syscalls", target_feature = "static-syscalls")))]
        $crate::define_syscall!(@reloc $(#[$attr])* $vis fn $name($($arg: $typ),*) -> $ret; link_name = $sym);
        #[cfg(any(feature = "static-syscalls", target_feature = "static-syscalls"))]
        $crate::define_syscall!(@static $(#[$attr])* $vis fn $name($($arg: $typ),*) -> $ret; hash = $sym);
    };
    (#[link_name = $sym:literal] $(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident: $typ:ty),* $(,)?)) => {
        $crate::define_syscall!(#[link_name = $sym] $(#[$attr])* $vis fn $name($($arg: $typ),*) -> ());
    };

    // ── Public surface: Rust name == symbol name ─────────────────────
    ($(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident: $typ:ty),* $(,)?) -> $ret:ty) => {
        #[cfg(not(any(feature = "static-syscalls", target_feature = "static-syscalls")))]
        $crate::define_syscall!(@reloc $(#[$attr])* $vis fn $name($($arg: $typ),*) -> $ret);
        #[cfg(any(feature = "static-syscalls", target_feature = "static-syscalls"))]
        $crate::define_syscall!(@static $(#[$attr])* $vis fn $name($($arg: $typ),*) -> $ret; hash = stringify!($name));
    };
    ($(#[$attr:meta])* $vis:vis fn $name:ident($($arg:ident: $typ:ty),* $(,)?)) => {
        $crate::define_syscall!($(#[$attr])* $vis fn $name($($arg: $typ),*) -> ());
    };
}

// The declarations below stay gated on `target_os = "solana"`, exactly as the
// single historical `extern "C"` block was: off-chain (host) builds get no
// syscall symbols and rely on each wrapper module's `not(target_os = "solana")`
// fallback. The only change is that each decl now flows through the macro so the
// same source is v3-ready under `--features static-syscalls`.

/// Log a UTF-8 message.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_log_(message: *const u8, len: u64));

/// Log a 64-bit value.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_log_64_(arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64));

/// Log the current compute unit consumption.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_log_compute_units_());

/// Log structured data segments (for events).
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_log_data(data: *const u8, data_len: u64));

/// Invoke a cross-program instruction (C ABI).
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_invoke_signed_c(
    instruction_addr: *const u8,
    account_infos_addr: *const u8,
    account_infos_len: u64,
    signers_seeds_addr: *const u8,
    signers_seeds_len: u64
) -> u64);

/// Create a program-derived address.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_create_program_address(
    seeds_addr: *const u8,
    seeds_len: u64,
    program_id_addr: *const u8,
    address_addr: *mut u8
) -> u64);

/// Find a program-derived address with bump seed.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_try_find_program_address(
    seeds_addr: *const u8,
    seeds_len: u64,
    program_id_addr: *const u8,
    address_addr: *mut u8,
    bump_seed_addr: *mut u8
) -> u64);

/// SHA-256 hash.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_sha256(vals: *const u8, val_len: u64, hash_result: *mut u8) -> u64);

/// Validate whether a point lies on the selected curve.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_curve_validate_point(
    curve_id: u64,
    point_addr: *const u8,
    result_point_addr: *mut u8
) -> u64);

/// Run a group operation on runtime-supported curve points.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_curve_group_op(
    curve_id: u64,
    group_op: u64,
    left_input_addr: *const u8,
    right_input_addr: *const u8,
    result_point_addr: *mut u8
) -> u64);

/// Run variable-length multiscalar multiplication on runtime-supported curves.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_curve_multiscalar_mul(
    curve_id: u64,
    scalars_addr: *const u8,
    points_addr: *const u8,
    points_len: u64,
    result_point_addr: *mut u8
) -> u64);

/// Keccak-256 hash.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_keccak256(vals: *const u8, val_len: u64, hash_result: *mut u8) -> u64);

/// BLAKE3 hash.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_blake3(vals: *const u8, val_len: u64, hash_result: *mut u8) -> u64);

/// Recover a secp256k1 public key from a 32-byte hash and compact signature.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_secp256k1_recover(
    hash: *const u8,
    recovery_id: u64,
    signature: *const u8,
    result: *mut u8
) -> u64);

/// Poseidon hash.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_poseidon(
    parameters: u64,
    endianness: u64,
    vals: *const u8,
    val_len: u64,
    hash_result: *mut u8
) -> u64);

/// BN254 / alt_bn128 group operation.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_alt_bn128_group_op(
    group_op: u64,
    input: *const u8,
    input_size: u64,
    result: *mut u8
) -> u64);

/// BN254 / alt_bn128 compression operation.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_alt_bn128_compression(
    op: u64,
    input: *const u8,
    input_size: u64,
    result: *mut u8
) -> u64);

/// Big integer modular exponentiation.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_big_mod_exp(params: *const u8, result: *mut u8) -> u64);

/// Set return data for the current instruction.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_set_return_data(data: *const u8, length: u64));

/// Get return data from the previous CPI.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_return_data(data: *mut u8, length: u64, program_id: *mut u8) -> u64);

/// Get the current clock sysvar.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_clock_sysvar(addr: *mut u8) -> u64);

/// Get the current rent sysvar.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_rent_sysvar(addr: *mut u8) -> u64);

/// Get epoch schedule sysvar.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_epoch_schedule_sysvar(addr: *mut u8) -> u64);

/// Abort program execution.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_panic_(file: *const u8, len: u64, line: u64, column: u64) -> !);

// ── Memory operations (SVM-optimized) ─────────────────────────

/// Copy `n` bytes from `src` to `dst` (non-overlapping).
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_memcpy_(dst: *mut u8, src: *const u8, n: u64));

/// Copy `n` bytes from `src` to `dst` (overlapping safe).
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_memmove_(dst: *mut u8, src: *const u8, n: u64));

/// Compare `n` bytes. Sets `*result` to <0, 0, or >0.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_memcmp_(s1: *const u8, s2: *const u8, n: u64, result: *mut i32));

/// Fill `n` bytes with `c`.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_memset_(s: *mut u8, c: u8, n: u64));

// ── Instruction introspection ────────────────────────────────

/// Get the current instruction stack height.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_stack_height() -> u64);

/// Get a previously processed sibling instruction.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_processed_sibling_instruction(
    index: u64,
    meta: *mut u8,
    program_id: *mut u8,
    data: *mut u8,
    accounts: *mut u8
) -> u64);

/// Get the last restart slot sysvar.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_last_restart_slot(addr: *mut u8) -> u64);

/// Generalized sysvar read: copy `length` bytes starting at `offset`
/// from the sysvar identified by `sysvar_id_addr` into `result`.
///
/// This is the modern replacement for the per-sysvar syscalls and the
/// only way to read large sysvars (SlotHashes, StakeHistory) without
/// passing them as accounts.
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_sysvar(
    sysvar_id_addr: *const u8,
    result: *mut u8,
    offset: u64,
    length: u64
) -> u64);

/// Get the activated stake (current epoch) of the vote account at
/// `vote_address`. A null `vote_address` returns the cluster-wide
/// total active stake (SIMD-0133).
#[cfg(target_os = "solana")]
define_syscall!(pub fn sol_get_epoch_stake(vote_address: *const u8) -> u64);

#[cfg(test)]
mod tests {
    // This module is intentionally *not* gated on `target_os = "solana"`, so
    // the host test build exercises whichever `define_syscall!` arm the active
    // feature selects:
    //   * `cargo test -p hopper-native`                       -> relocation arm
    //   * `cargo test -p hopper-native --features static-syscalls` -> static arm
    // The generated declaration is never *called* on the host (the relocation
    // symbol does not exist off-chain and the static hash is a fabricated
    // pointer); we only prove it compiles into a callable declaration.
    #[allow(dead_code)]
    mod generated {
        // Exercises the "symbol == name" surface.
        crate::define_syscall!(
            /// Test-only dummy syscall (never linked/called on host).
            pub fn __hopper_probe_syscall(a: *const u8, n: u64) -> u64
        );
        // Exercises the explicit-`link_name` surface + unit return.
        crate::define_syscall!(
            #[link_name = "sol_memset_"]
            pub fn __hopper_probe_alias(a: *mut u8, c: u8, n: u64)
        );
    }

    #[test]
    fn sys_hash_matches_known_agave_constants() {
        // `sol_memcmp_` is the canonical cross-check: the Quasar
        // `solana-compiler-builtins` reference pins it to 0x5FDC_DE31.
        assert_eq!(super::sys_hash("sol_memcmp_"), 0x5FDC_DE31);
        // Additional well-known Agave sBPF static-syscall dispatch keys.
        assert_eq!(super::sys_hash("abort"), 0xB6FC_1A11);
        assert_eq!(super::sys_hash("sol_memcpy_"), 0x717C_C4A3);
        assert_eq!(super::sys_hash("sol_memset_"), 0x3770_FB22);
        assert_eq!(super::sys_hash("sol_memmove_"), 0x4343_71F8);
        assert_eq!(super::sys_hash("sol_invoke_signed_c"), 0xA22B_9C85);
    }

    // Under the static feature the macro must produce a real, addressable
    // `unsafe fn`. Taking (but never calling) its address proves the static
    // arm expanded to a callable declaration.
    #[cfg(any(feature = "static-syscalls", target_feature = "static-syscalls"))]
    #[test]
    fn static_arm_expands_to_callable_fn() {
        let f: unsafe fn(*const u8, u64) -> u64 = generated::__hopper_probe_syscall;
        assert!(f as usize != 0);
    }
}
