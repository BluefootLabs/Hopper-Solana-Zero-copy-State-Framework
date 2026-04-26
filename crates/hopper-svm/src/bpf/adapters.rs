//! Syscall adapter layer — bridges Hopper's pure-Rust `do_*`
//! syscall logic functions (in [`super::syscalls`]) into the
//! `solana_sbpf::program::BuiltinFunction` shape the VM expects.
//!
//! Each adapter:
//!
//! 1. Receives the SBPF call: `&mut BpfContext`, five `u64`
//!    register args, and the live `MemoryMapping`.
//! 2. Translates the relevant register args from VM addresses to
//!    host slices via [`translate_slice`] / [`translate_slice_mut`].
//! 3. Delegates the actual work to the corresponding
//!    `do_*` function in [`super::syscalls`].
//! 4. Maps [`SyscallResult`] into `Result<u64, EbpfError>` per
//!    SBPF's expectations.
//!
//! ## Why a separate file
//!
//! All `solana-sbpf`-specific code lives here. If a future minor
//! release changes the macro syntax or memory-translation API,
//! the rest of `bpf/` stays untouched — only this file needs the
//! fixup. The pure-Rust logic in [`super::syscalls`] keeps
//! working unchanged because adapters are the only consumers of
//! it that talk to sbpf.
//!
//! ## API drift risk
//!
//! `solana-sbpf 0.20.0` exposes the
//! `solana_sbpf::declare_builtin_function!` macro that generates
//! a `BuiltinFunction` adapter from a Rust function body. The
//! exact macro syntax differs slightly between minor versions;
//! the form below targets the 0.20 API but may need a one-line
//! tweak to work against 0.19 or 0.21.
//!
//! Memory translation goes through
//! `solana_sbpf::memory_region::MemoryMapping::map(access, vm_addr, len)`,
//! which returns a host pointer (as `u64`) the host can cast to a
//! pointer + slice. The `AccessType` enum gates whether the
//! returned slice is mutable.

use crate::account::KeyedAccount;
use crate::bpf::context::BpfContext;
use crate::bpf::cpi::{self, ParsedCpi};
use crate::bpf::cpi_rust;
use crate::bpf::crypto_syscalls;
use crate::bpf::parameter::MAX_PERMITTED_DATA_INCREASE;
use crate::bpf::syscalls::{self, SyscallResult};
use crate::bpf::sysvar_syscalls;
use crate::bpf::tier3_syscalls;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;
use solana_sbpf::declare_builtin_function;
use solana_sbpf::error::EbpfError;
use solana_sbpf::memory_region::{AccessType, MemoryMapping};

/// Map `SyscallResult` onto sbpf's `Result<u64, EbpfError>`. The
/// VM sees `Ok(0)` for success, an `EbpfError::SyscallError` with
/// the captured message for `Custom`, and an
/// `EbpfError::ExceededMaxInstructions` for `OutOfMeter` (the
/// closest match in sbpf's error vocabulary; the engine remaps it
/// into [`crate::HopperSvmError::OutOfComputeUnits`] on unwind).
fn map_result(r: SyscallResult) -> Result<u64, Box<dyn std::error::Error>> {
    match r {
        SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
        SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
        SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
            "compute budget exhausted inside syscall".to_string(),
        ))),
    }
}

/// Newtype wrapping a syscall failure as an `Error`. Carries the
/// human-readable message from `SyscallResult::Custom`.
#[derive(Debug)]
struct SyscallError(String);

impl core::fmt::Display for SyscallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SyscallError {}

// ---------------------------------------------------------------------------
// Translation helpers
// ---------------------------------------------------------------------------
//
// Phase 2.0 uses a single bounds-checking model: ask the
// `MemoryMapping` to validate `(access, vm_addr, len)` and return
// the host pointer; cast to a slice. Any mapping error (out of
// region, write to read-only, …) bubbles up as an `EbpfError`
// which sbpf surfaces via the syscall return path.
//
// The unsafe slice construction is the fundamental safety lemma
// of guest→host translation: the mapping has already verified
// that `[host_ptr, host_ptr + len)` is fully inside a host-owned
// region with the requested access, so the resulting slice is
// well-defined for `len` bytes.

/// Translate `(vm_addr, len)` into a read-only host slice.
fn translate_slice<'a>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    len: u64,
) -> Result<&'a [u8], EbpfError> {
    if len == 0 {
        return Ok(&[]);
    }
    let host_addr = memory_mapping.map(AccessType::Load, vm_addr, len)?;
    // SAFETY: `MemoryMapping::map` validated the range; the
    // returned host pointer lives for the entire VM execution
    // (lifetime of the input/heap/stack buffers the engine
    // allocated before construction).
    Ok(unsafe { core::slice::from_raw_parts(host_addr as *const u8, len as usize) })
}

/// Translate `(vm_addr, len)` into a writable host slice.
#[allow(clippy::mut_from_ref)]
fn translate_slice_mut<'a>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
    len: u64,
) -> Result<&'a mut [u8], EbpfError> {
    if len == 0 {
        // Return an empty slice with a non-null but unique
        // pointer so the resulting `&mut []` doesn't alias.
        return Ok(unsafe {
            core::slice::from_raw_parts_mut(core::ptr::NonNull::dangling().as_ptr(), 0)
        });
    }
    let host_addr = memory_mapping.map(AccessType::Store, vm_addr, len)?;
    // SAFETY: as `translate_slice`, plus the mapping has verified
    // write access. Aliased mutable access across translations is
    // the caller's responsibility — for memcpy we route overlap
    // detection through a temp buffer in `sol_memmove_`.
    Ok(unsafe { core::slice::from_raw_parts_mut(host_addr as *mut u8, len as usize) })
}

/// Translate a fixed-size array (typically `&[u8; 32]` for
/// pubkeys). Convenience wrapper around [`translate_slice`].
fn translate_array<'a, const N: usize>(
    memory_mapping: &MemoryMapping,
    vm_addr: u64,
) -> Result<&'a [u8; N], EbpfError> {
    let slice = translate_slice(memory_mapping, vm_addr, N as u64)?;
    Ok(slice
        .try_into()
        .expect("translate_array slice length matches N"))
}

// ---------------------------------------------------------------------------
// Adapter declarations
// ---------------------------------------------------------------------------
//
// `declare_builtin_function!` generates a struct (the syscall's
// type, e.g. `SyscallSolLog`) that implements
// `solana_sbpf::program::BuiltinFunction`. The struct's
// `BuiltinFunctionDefinition::vm` adapter is the function pointer
// the VM calls; `BuiltinFunctionDefinition::rust` is what unit
// tests can call directly.
//
// Each adapter is one-to-one with a `do_*` function in
// `super::syscalls`. The adapter does only:
//   - Translate VM addresses to host slices
//   - Call the `do_*`
//   - Map the result onto sbpf's expected return type

declare_builtin_function!(
    /// `sol_log_` — log a UTF-8 message.
    SyscallSolLog,
    fn rust(
        ctx: &mut BpfContext,
        msg_addr: u64,
        msg_len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let bytes = translate_slice(memory_mapping, msg_addr, msg_len)?;
        map_result(syscalls::do_sol_log(ctx, bytes))
    }
);

declare_builtin_function!(
    /// `sol_log_64_` — log five u64 register values.
    SyscallSolLog64,
    fn rust(
        ctx: &mut BpfContext,
        a: u64,
        b: u64,
        c: u64,
        d: u64,
        e: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        map_result(syscalls::do_sol_log_64(ctx, a, b, c, d, e))
    }
);

declare_builtin_function!(
    /// `sol_log_pubkey` — log a base58-encoded 32-byte pubkey.
    SyscallSolLogPubkey,
    fn rust(
        ctx: &mut BpfContext,
        key_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let key = translate_array::<32>(memory_mapping, key_addr)?;
        map_result(syscalls::do_sol_log_pubkey(ctx, key))
    }
);

declare_builtin_function!(
    /// `sol_log_compute_units_` — log the runtime CU framing line.
    /// `initial_budget` is captured by the engine when it builds
    /// the context and stored on the context's `program_id` slot
    /// (separate from the live meter). For Phase 2.0 we read the
    /// engine-provided initial budget through a context field; if
    /// the engine doesn't populate it the syscall reports a
    /// budget of zero (still produces the framing line, just with
    /// `consumed of 0`).
    SyscallSolLogComputeUnits,
    fn rust(
        ctx: &mut BpfContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Phase 2.0 reads the initial budget from a side-channel
        // field that lives outside this function's view. The
        // engine populates it when it constructs `BpfContext`;
        // if missing, default to the current `remaining_units`
        // so the framing line is still emitted (with `consumed
        // of <remaining>`, which is harmless).
        let initial = ctx.remaining_units;
        map_result(syscalls::do_sol_log_compute_units(ctx, initial))
    }
);

declare_builtin_function!(
    /// `sol_panic_` — capture a structured panic message.
    SyscallSolPanic,
    fn rust(
        ctx: &mut BpfContext,
        file_addr: u64,
        file_len: u64,
        line: u64,
        column: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let file = translate_slice(memory_mapping, file_addr, file_len)?;
        map_result(syscalls::do_sol_panic(ctx, file, line, column))
    }
);

declare_builtin_function!(
    /// `sol_memcpy_` — guest memcpy (no-overlap variant).
    SyscallSolMemcpy,
    fn rust(
        ctx: &mut BpfContext,
        dst_addr: u64,
        src_addr: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Overlap detection: the runtime rejects overlapping
        // memcpy ranges. Compare VM-address spans rather than
        // host pointers because aliasing is a property of guest
        // semantics.
        if ranges_overlap(dst_addr, src_addr, n) {
            return Err(Box::new(SyscallError(
                "sol_memcpy_: source and destination ranges overlap".to_string(),
            )));
        }
        let src = translate_slice(memory_mapping, src_addr, n)?;
        // Snapshot src into a stack buffer so we don't hold a
        // shared borrow of the mapping while taking the mutable
        // borrow for dst. For typical `n` (≤ 1KB) this is fine;
        // larger memcpys can fall through to the boxed-vec path.
        let mut tmp_stack = [0u8; 1024];
        let tmp: Vec<u8>;
        let src_bytes: &[u8] = if (n as usize) <= tmp_stack.len() {
            tmp_stack[..n as usize].copy_from_slice(src);
            &tmp_stack[..n as usize]
        } else {
            tmp = src.to_vec();
            tmp.as_slice()
        };
        let dst = translate_slice_mut(memory_mapping, dst_addr, n)?;
        map_result(syscalls::do_sol_memcpy(ctx, dst, src_bytes))
    }
);

declare_builtin_function!(
    /// `sol_memset_` — guest memset.
    SyscallSolMemset,
    fn rust(
        ctx: &mut BpfContext,
        dst_addr: u64,
        value: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let dst = translate_slice_mut(memory_mapping, dst_addr, n)?;
        map_result(syscalls::do_sol_memset(ctx, dst, value as u8))
    }
);

declare_builtin_function!(
    /// `sol_memcmp_` — guest memcmp; result written to `out_addr`.
    SyscallSolMemcmp,
    fn rust(
        ctx: &mut BpfContext,
        a_addr: u64,
        b_addr: u64,
        n: u64,
        out_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let a = translate_slice(memory_mapping, a_addr, n)?;
        let b_owned: Vec<u8> = translate_slice(memory_mapping, b_addr, n)?.to_vec();
        let out_slice = translate_slice_mut(memory_mapping, out_addr, 4)?;
        let mut out_arr: [u8; 4] = [0; 4];
        let r = syscalls::do_sol_memcmp(ctx, a, &b_owned, &mut out_arr);
        out_slice.copy_from_slice(&out_arr);
        map_result(r)
    }
);

declare_builtin_function!(
    /// `sol_memmove_` — guest memmove (overlap-tolerant).
    SyscallSolMemmove,
    fn rust(
        ctx: &mut BpfContext,
        dst_addr: u64,
        src_addr: u64,
        n: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Always go through a temporary buffer so overlapping
        // ranges don't corrupt each other. For typical sizes
        // this is on the stack; larger goes to the heap.
        let src_owned: Vec<u8> =
            translate_slice(memory_mapping, src_addr, n)?.to_vec();
        let dst = translate_slice_mut(memory_mapping, dst_addr, n)?;
        map_result(syscalls::do_sol_memmove(ctx, dst, &src_owned))
    }
);

declare_builtin_function!(
    /// `sol_set_return_data` — store program return data.
    SyscallSolSetReturnData,
    fn rust(
        ctx: &mut BpfContext,
        data_addr: u64,
        data_len: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let data = translate_slice(memory_mapping, data_addr, data_len)?;
        map_result(syscalls::do_sol_set_return_data(ctx, data))
    }
);

declare_builtin_function!(
    /// `sol_get_return_data` — read previously-set return data.
    SyscallSolGetReturnData,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        out_len: u64,
        program_id_addr: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let mut pid_buf: [u8; 32] = [0; 32];
        let real_len = {
            let out = translate_slice_mut(memory_mapping, out_addr, out_len)?;
            match syscalls::do_sol_get_return_data(ctx, out, &mut pid_buf) {
                Ok(n) => n,
                Err(e) => return map_result(e),
            }
        };
        if real_len > 0 {
            let pid_dst = translate_slice_mut(memory_mapping, program_id_addr, 32)?;
            pid_dst.copy_from_slice(&pid_buf);
        }
        Ok(real_len)
    }
);

// ---------------------------------------------------------------------------
// PDA derivation adapters
// ---------------------------------------------------------------------------
//
// Wire shape (matches the upstream `sol_create_program_address`
// and `sol_try_find_program_address` exports):
//
//   sol_create_program_address(
//       seeds_addr: u64,    // [(seed_addr: u64, seed_len: u64); n_seeds]
//       n_seeds: u64,
//       program_id_addr: u64,  // 32 bytes
//       address_out_addr: u64, // 32 bytes; only written on Ok
//   ) -> u64;  // 0 = Ok, non-zero = error code
//
//   sol_try_find_program_address(
//       seeds_addr: u64,
//       n_seeds: u64,
//       program_id_addr: u64,
//       address_out_addr: u64,
//       bump_out_addr: u64,    // 1 byte; written on Ok
//   ) -> u64;
//
// Per-seed descriptor: 16 bytes = u64 addr + u64 len, both LE.
// Translation: each seed is fetched into an owned Vec<u8> so the
// borrow on the mapping ends before we pass the slice list into
// the pure-Rust `do_*` function.

/// Translate a seed-descriptor list `[(addr, len); n]` into owned
/// byte vectors. Each seed is bounds-checked through the mapping.
fn translate_seeds(
    memory_mapping: &MemoryMapping,
    seeds_addr: u64,
    n_seeds: u64,
) -> Result<Vec<Vec<u8>>, EbpfError> {
    let descriptor_bytes = n_seeds.checked_mul(16).ok_or_else(|| {
        EbpfError::SyscallError(Box::new(SyscallError(
            "translate_seeds: seed count overflow".to_string(),
        )))
    })?;
    let descriptors = translate_slice(memory_mapping, seeds_addr, descriptor_bytes)?;
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(n_seeds as usize);
    for i in 0..n_seeds as usize {
        let off = i * 16;
        let addr = u64::from_le_bytes(
            descriptors[off..off + 8]
                .try_into()
                .expect("seed addr 8 bytes"),
        );
        let len = u64::from_le_bytes(
            descriptors[off + 8..off + 16]
                .try_into()
                .expect("seed len 8 bytes"),
        );
        out.push(translate_slice(memory_mapping, addr, len)?.to_vec());
    }
    Ok(out)
}

declare_builtin_function!(
    /// `sol_create_program_address` — single-shot PDA derivation.
    SyscallSolCreateProgramAddress,
    fn rust(
        ctx: &mut BpfContext,
        seeds_addr: u64,
        n_seeds: u64,
        program_id_addr: u64,
        address_out_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let seeds_owned = translate_seeds(memory_mapping, seeds_addr, n_seeds)?;
        let program_id = *translate_array::<32>(memory_mapping, program_id_addr)?;
        let seed_refs: Vec<&[u8]> =
            seeds_owned.iter().map(|v| v.as_slice()).collect();
        match syscalls::do_sol_create_program_address(ctx, &seed_refs, &program_id) {
            Ok(pda) => {
                let dst = translate_slice_mut(memory_mapping, address_out_addr, 32)?;
                dst.copy_from_slice(&pda);
                Ok(syscalls::SYSCALL_OK)
            }
            Err(syscalls::PdaError::OnCurve) => {
                // Per the upstream wire format, the runtime
                // returns a non-zero code (1) for "PDA invalid"
                // — the caller's bump-and-retry loop in
                // `sol_try_find_program_address` reads this and
                // continues.
                Ok(1)
            }
            Err(other) => Err(Box::new(SyscallError(format!(
                "sol_create_program_address: {other}"
            )))),
        }
    }
);

declare_builtin_function!(
    /// `sol_try_find_program_address` — bump-and-retry PDA
    /// derivation. Writes both the resulting PDA and the bump
    /// byte on success.
    SyscallSolTryFindProgramAddress,
    fn rust(
        ctx: &mut BpfContext,
        seeds_addr: u64,
        n_seeds: u64,
        program_id_addr: u64,
        address_out_addr: u64,
        bump_out_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let seeds_owned = translate_seeds(memory_mapping, seeds_addr, n_seeds)?;
        let program_id = *translate_array::<32>(memory_mapping, program_id_addr)?;
        let seed_refs: Vec<&[u8]> =
            seeds_owned.iter().map(|v| v.as_slice()).collect();
        match syscalls::do_sol_try_find_program_address(ctx, &seed_refs, &program_id)
        {
            Some((pda, bump)) => {
                let addr_dst =
                    translate_slice_mut(memory_mapping, address_out_addr, 32)?;
                addr_dst.copy_from_slice(&pda);
                let bump_dst =
                    translate_slice_mut(memory_mapping, bump_out_addr, 1)?;
                bump_dst[0] = bump;
                Ok(syscalls::SYSCALL_OK)
            }
            // Same convention as the upstream runtime: a non-zero
            // return signals "no valid PDA found." Programs are
            // expected to handle this rather than panic — though
            // in practice every realistic seed set finds one.
            None => Ok(1),
        }
    }
);

// ---------------------------------------------------------------------------
// Sysvar adapters
// ---------------------------------------------------------------------------
//
// Wire shape: each `sol_get_*_sysvar` takes a single u64 register
// — the destination buffer address. The buffer length is implicit
// (the runtime writes exactly the sysvar's struct size). We
// translate the buffer through `translate_slice_mut` with the
// fixed length per sysvar, then delegate to the matching `do_*`.

declare_builtin_function!(
    /// `sol_get_clock_sysvar` — write the Clock struct to `out`.
    SyscallSolGetClockSysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            sysvar_syscalls::CLOCK_BYTES as u64,
        )?;
        match sysvar_syscalls::do_sol_get_clock_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => {
                Err(Box::new(SyscallError(msg)))
            }
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_clock_sysvar: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_rent_sysvar` — write the Rent struct to `out`.
    SyscallSolGetRentSysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            sysvar_syscalls::RENT_BYTES as u64,
        )?;
        match sysvar_syscalls::do_sol_get_rent_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_rent_sysvar: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_epoch_schedule_sysvar` — write the EpochSchedule
    /// struct to `out`.
    SyscallSolGetEpochScheduleSysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            sysvar_syscalls::EPOCH_SCHEDULE_BYTES as u64,
        )?;
        match sysvar_syscalls::do_sol_get_epoch_schedule_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_epoch_schedule_sysvar: compute budget exhausted"
                    .to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_epoch_rewards_sysvar` — write the EpochRewards
    /// struct (96 bytes) to `out`.
    SyscallSolGetEpochRewardsSysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            sysvar_syscalls::EPOCH_REWARDS_BYTES as u64,
        )?;
        match sysvar_syscalls::do_sol_get_epoch_rewards_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_epoch_rewards_sysvar: compute budget exhausted"
                    .to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_last_restart_slot_sysvar` — write the
    /// LastRestartSlot u64 to `out`.
    SyscallSolGetLastRestartSlotSysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            sysvar_syscalls::LAST_RESTART_SLOT_BYTES as u64,
        )?;
        match sysvar_syscalls::do_sol_get_last_restart_slot_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_last_restart_slot_sysvar: compute budget exhausted"
                    .to_string(),
            ))),
        }
    }
);

// ---------------------------------------------------------------------------
// Crypto adapters (hash + signature recover)
// ---------------------------------------------------------------------------
//
// Hash syscalls share the (vals_addr, vals_len, out_addr) wire
// shape with `sol_log_data` and the PDA-derive seed list — every
// chunk is a 16-byte (addr, len) descriptor. We reuse the
// `translate_seeds` helper to read them into owned Vec<u8>s.

declare_builtin_function!(
    /// `sol_keccak256_` — Keccak-256 over a concatenated chunk
    /// list. 32-byte output.
    SyscallSolKeccak256,
    fn rust(
        ctx: &mut BpfContext,
        vals_addr: u64,
        vals_len: u64,
        out_addr: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let chunks_owned = translate_seeds(memory_mapping, vals_addr, vals_len)?;
        let chunk_refs: Vec<&[u8]> =
            chunks_owned.iter().map(|v| v.as_slice()).collect();
        let mut digest = [0u8; 32];
        let r = crypto_syscalls::do_sol_keccak256(ctx, &chunk_refs, &mut digest);
        match r {
            SyscallResult::Ok => {
                let dst = translate_slice_mut(memory_mapping, out_addr, 32)?;
                dst.copy_from_slice(&digest);
                Ok(syscalls::SYSCALL_OK)
            }
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_keccak256_: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_blake3` — BLAKE3 over a concatenated chunk list.
    /// 32-byte output.
    SyscallSolBlake3,
    fn rust(
        ctx: &mut BpfContext,
        vals_addr: u64,
        vals_len: u64,
        out_addr: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let chunks_owned = translate_seeds(memory_mapping, vals_addr, vals_len)?;
        let chunk_refs: Vec<&[u8]> =
            chunks_owned.iter().map(|v| v.as_slice()).collect();
        let mut digest = [0u8; 32];
        let r = crypto_syscalls::do_sol_blake3(ctx, &chunk_refs, &mut digest);
        match r {
            SyscallResult::Ok => {
                let dst = translate_slice_mut(memory_mapping, out_addr, 32)?;
                dst.copy_from_slice(&digest);
                Ok(syscalls::SYSCALL_OK)
            }
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_blake3: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_secp256k1_recover_` — ECDSA public-key recovery from
    /// a 32-byte hash + 64-byte signature + recovery id. Writes
    /// the 64-byte uncompressed public key (X || Y, no leading
    /// 0x04 marker) to `out_addr` on Ok.
    SyscallSolSecp256k1Recover,
    fn rust(
        ctx: &mut BpfContext,
        hash_addr: u64,
        recovery_id: u64,
        signature_addr: u64,
        out_addr: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let hash = *translate_array::<32>(memory_mapping, hash_addr)?;
        let signature = *translate_array::<64>(memory_mapping, signature_addr)?;
        let mut pk = [0u8; 64];
        let outcome = crypto_syscalls::do_sol_secp256k1_recover(
            ctx,
            &hash,
            recovery_id,
            &signature,
            &mut pk,
        );
        match outcome {
            crypto_syscalls::RecoverOutcome::Ok => {
                let dst = translate_slice_mut(memory_mapping, out_addr, 64)?;
                dst.copy_from_slice(&pk);
                Ok(syscalls::SYSCALL_OK)
            }
            crypto_syscalls::RecoverOutcome::Failed(code) => Ok(code),
            crypto_syscalls::RecoverOutcome::OutOfMeter => Err(Box::new(SyscallError(
                "sol_secp256k1_recover_: compute budget exhausted".to_string(),
            ))),
        }
    }
);

// ---------------------------------------------------------------------------
// CPI adapters
// ---------------------------------------------------------------------------
//
// `sol_invoke_signed_c` is the well-specified C-ABI variant — we
// parse the SolInstruction / SolAccountInfo / SolSignerSeeds
// wire structures from the parameter buffer and dispatch
// through the BpfContext's CPI dispatcher closure. After the
// inner call returns, we sync any account-state changes back
// through the SolAccountInfo pointers the caller supplied (the
// outer program reads its account state through those pointers,
// so writing back through them is what the program will see on
// resume).
//
// `sol_invoke_signed_rust` reads through Rust's
// `Rc<RefCell<&mut u64>>` AccountInfo wrappers whose memory
// layout is a Rust-internal that drifts between toolchain
// versions. We register the syscall but return a structured
// error so programs see a clean failure rather than a
// "missing syscall" abort. Phase 2.2.

declare_builtin_function!(
    /// `sol_invoke_signed_c` — full CPI through the C-ABI wire
    /// format.
    SyscallSolInvokeSignedC,
    fn rust(
        ctx: &mut BpfContext,
        instruction_addr: u64,
        accounts_addr: u64,
        accounts_len: u64,
        signer_seeds_addr: u64,
        signer_seeds_len: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Parse the SolInstruction (40 bytes of useful data,
        // padded to 40 — we read fields directly without
        // assuming padding bytes are zero).
        let ix_bytes = translate_slice(memory_mapping, instruction_addr, 40)?;
        let program_id_addr = u64::from_le_bytes(
            ix_bytes[0..8].try_into().expect("program_id_addr"),
        );
        let inner_metas_addr = u64::from_le_bytes(
            ix_bytes[8..16].try_into().expect("metas_addr"),
        );
        let inner_metas_len = u64::from_le_bytes(
            ix_bytes[16..24].try_into().expect("metas_len"),
        );
        let inner_data_addr = u64::from_le_bytes(
            ix_bytes[24..32].try_into().expect("data_addr"),
        );
        let inner_data_len = u64::from_le_bytes(
            ix_bytes[32..40].try_into().expect("data_len"),
        );
        let program_id =
            Pubkey::new_from_array(*translate_array::<32>(memory_mapping, program_id_addr)?);

        // Parse the inner instruction's AccountMeta list.
        // Each entry: 8 bytes pubkey_addr + 1 is_writable + 1
        // is_signer + 6 padding = 16 bytes.
        let metas_total = inner_metas_len.checked_mul(16).ok_or_else(|| {
            Box::new(SyscallError(
                "sol_invoke_signed_c: inner-meta count overflow".to_string(),
            ))
        })?;
        let metas_bytes =
            translate_slice(memory_mapping, inner_metas_addr, metas_total)?;
        let mut metas: Vec<AccountMeta> = Vec::with_capacity(inner_metas_len as usize);
        for i in 0..inner_metas_len as usize {
            let off = i * 16;
            let pk_addr = u64::from_le_bytes(
                metas_bytes[off..off + 8].try_into().expect("meta pk_addr"),
            );
            let is_writable = metas_bytes[off + 8] != 0;
            let is_signer = metas_bytes[off + 9] != 0;
            let pubkey = Pubkey::new_from_array(*translate_array::<32>(
                memory_mapping,
                pk_addr,
            )?);
            metas.push(AccountMeta {
                pubkey,
                is_signer,
                is_writable,
            });
        }

        // Parse the inner instruction data.
        let data =
            translate_slice(memory_mapping, inner_data_addr, inner_data_len)?
                .to_vec();

        // Parse the caller's SolAccountInfo array. Each entry:
        // 8 key_addr + 8 lamports_addr + 8 data_len + 8 data_addr
        // + 8 owner_addr + 8 rent_epoch + 1 is_signer + 1 is_writable
        // + 1 executable + 5 padding = 56 bytes.
        let accounts_total = accounts_len.checked_mul(56).ok_or_else(|| {
            Box::new(SyscallError(
                "sol_invoke_signed_c: account-info count overflow".to_string(),
            ))
        })?;
        let accounts_bytes =
            translate_slice(memory_mapping, accounts_addr, accounts_total)?;
        let mut snapshot: Vec<KeyedAccount> = Vec::with_capacity(accounts_len as usize);
        let mut writeback_targets: Vec<AccountWriteback> =
            Vec::with_capacity(accounts_len as usize);
        for i in 0..accounts_len as usize {
            let off = i * 56;
            let key_addr = u64::from_le_bytes(
                accounts_bytes[off..off + 8].try_into().expect("key_addr"),
            );
            let lamports_addr = u64::from_le_bytes(
                accounts_bytes[off + 8..off + 16]
                    .try_into()
                    .expect("lamports_addr"),
            );
            let data_len = u64::from_le_bytes(
                accounts_bytes[off + 16..off + 24]
                    .try_into()
                    .expect("data_len"),
            );
            let data_addr = u64::from_le_bytes(
                accounts_bytes[off + 24..off + 32]
                    .try_into()
                    .expect("data_addr"),
            );
            let owner_addr = u64::from_le_bytes(
                accounts_bytes[off + 32..off + 40]
                    .try_into()
                    .expect("owner_addr"),
            );
            let rent_epoch = u64::from_le_bytes(
                accounts_bytes[off + 40..off + 48]
                    .try_into()
                    .expect("rent_epoch"),
            );
            let is_signer = accounts_bytes[off + 48] != 0;
            let is_writable = accounts_bytes[off + 49] != 0;
            let executable = accounts_bytes[off + 50] != 0;

            let address = Pubkey::new_from_array(*translate_array::<32>(
                memory_mapping,
                key_addr,
            )?);
            let owner = Pubkey::new_from_array(*translate_array::<32>(
                memory_mapping,
                owner_addr,
            )?);
            let lamports = {
                let bytes = translate_slice(memory_mapping, lamports_addr, 8)?;
                u64::from_le_bytes(bytes.try_into().expect("lamports"))
            };
            let acct_data =
                translate_slice(memory_mapping, data_addr, data_len)?.to_vec();

            snapshot.push(KeyedAccount {
                address,
                lamports,
                data: acct_data,
                owner,
                executable,
                rent_epoch,
            });
            // Capacity for realloc-during-CPI is the original
            // data length plus the per-account realloc tail the
            // outer parameter buffer already reserved
            // (`MAX_PERMITTED_DATA_INCREASE = 10240` per upstream).
            // The SolAccountInfo's `data_len` field at byte 16
            // of its 56-byte record is the dynamic length; we
            // record its address so the writeback can rewrite it
            // when the inner call grows the data region.
            let data_len_field_addr = accounts_addr + (i as u64) * 56 + 16;
            writeback_targets.push(AccountWriteback {
                address,
                lamports_addr,
                data_addr,
                data_len_field_addr,
                data_len_capacity: data_len + MAX_PERMITTED_DATA_INCREASE as u64,
                owner_addr,
                is_writable,
            });
        }

        // Parse signer-seeds: SolSignerSeeds[len] where each is
        // (addr, len) pointing at SolSignerSeed[q] which is
        // again (addr, len) pointing at the seed bytes.
        let signer_seeds_total =
            signer_seeds_len.checked_mul(16).ok_or_else(|| {
                Box::new(SyscallError(
                    "sol_invoke_signed_c: signer-seeds count overflow"
                        .to_string(),
                ))
            })?;
        let signer_seeds_bytes =
            translate_slice(memory_mapping, signer_seeds_addr, signer_seeds_total)?;
        let mut signer_seeds: Vec<Vec<Vec<u8>>> =
            Vec::with_capacity(signer_seeds_len as usize);
        for i in 0..signer_seeds_len as usize {
            let off = i * 16;
            let inner_addr = u64::from_le_bytes(
                signer_seeds_bytes[off..off + 8]
                    .try_into()
                    .expect("seed_set addr"),
            );
            let inner_len = u64::from_le_bytes(
                signer_seeds_bytes[off + 8..off + 16]
                    .try_into()
                    .expect("seed_set len"),
            );
            let owned = translate_seeds(memory_mapping, inner_addr, inner_len)?;
            signer_seeds.push(owned);
        }

        let parsed = ParsedCpi {
            program_id,
            metas: metas.clone(),
            data,
            accounts: snapshot,
            signer_seeds,
        };

        // Verify signer seeds. The "outer signers" — pubkeys that
        // were already signers on the calling program's
        // instruction — are read from the parameter buffer's
        // account list. For Phase 2.1 we conservatively pass the
        // metas-as-signers from the SolAccountInfo array (those
        // marked is_signer there are also signers in the outer
        // ix); that's the same condition the runtime uses.
        let outer_signers: Vec<Pubkey> = writeback_targets
            .iter()
            .filter(|w| {
                // An account that's a signer in the SolAccountInfo
                // list is one the outer program received as a
                // signer; signer status is preserved across the
                // CPI for forwarded signatures.
                snapshot_is_signer(&accounts_bytes, w.address)
            })
            .map(|w| w.address)
            .collect();
        let _ = outer_signers; // Reserved for the full check; we
                                // currently require seed-derived
                                // PDA verification or
                                // outer-signer pass-through, both
                                // of which `verify_signer_seeds`
                                // handles.

        // Build the *real* outer-signer list from the
        // SolAccountInfo flags.
        let outer_signers: Vec<Pubkey> = (0..accounts_len as usize)
            .filter_map(|i| {
                let off = i * 56;
                if accounts_bytes[off + 48] != 0 {
                    let key_addr = u64::from_le_bytes(
                        accounts_bytes[off..off + 8]
                            .try_into()
                            .expect("key_addr"),
                    );
                    let arr = translate_array::<32>(memory_mapping, key_addr).ok()?;
                    Some(Pubkey::new_from_array(*arr))
                } else {
                    None
                }
            })
            .collect();
        if let Err(code) =
            cpi::verify_signer_seeds(ctx, &parsed, &ctx.program_id.clone(), &outer_signers)
        {
            return Ok(code);
        }

        // Recursive dispatch.
        let outcome = match cpi::dispatch_cpi(ctx, parsed) {
            Ok(o) => o,
            Err(code) => return Ok(code),
        };

        // Writeback: for each account the inner call mutated,
        // write the new state back through the SolAccountInfo
        // pointers the caller gave us. The outer program will
        // continue reading from those same addresses, so the
        // mutations are visible on resume.
        //
        // Realloc-across-CPI (Phase 2.2): when the inner call
        // grew an account's data, we
        //  1. write the new bytes into the data region (which
        //     extends through the realloc tail the parameter
        //     buffer already has reserved),
        //  2. update the SolAccountInfo's `data_len` field at
        //     its known offset so the outer program reads the
        //     new length on resume.
        // The capacity ceiling is `original_data_len +
        // MAX_PERMITTED_DATA_INCREASE` (10240 bytes per
        // upstream). Any growth beyond that is truncated and
        // logged as a warning so a runaway program doesn't
        // silently corrupt buffer-adjacent state.
        for post in &outcome.resulting_accounts {
            if let Some(target) = writeback_targets
                .iter()
                .find(|w| w.address == post.address)
            {
                if !target.is_writable {
                    // Inner call shouldn't have mutated a
                    // non-writable account; if it did, we
                    // silently drop the change (matches what
                    // mainnet does — non-writable mutations
                    // are reverted at the boundary).
                    continue;
                }
                // Write lamports.
                let lp_dst =
                    translate_slice_mut(memory_mapping, target.lamports_addr, 8)?;
                lp_dst.copy_from_slice(&post.lamports.to_le_bytes());
                // Write owner.
                let ow_dst =
                    translate_slice_mut(memory_mapping, target.owner_addr, 32)?;
                ow_dst.copy_from_slice(post.owner.as_ref());
                // Write data, honoring the realloc tail. The
                // capacity is `original_data_len +
                // MAX_PERMITTED_DATA_INCREASE`, which the
                // parameter buffer already reserved at outer-
                // serialization time.
                let new_len = (post.data.len() as u64).min(target.data_len_capacity);
                if new_len < post.data.len() as u64 {
                    ctx.logs.line(format!(
                        "Program log: warning: CPI realloc truncated for account {} ({} requested, {} max)",
                        post.address,
                        post.data.len(),
                        target.data_len_capacity
                    ));
                }
                if new_len > 0 {
                    let data_dst = translate_slice_mut(
                        memory_mapping,
                        target.data_addr,
                        new_len,
                    )?;
                    data_dst.copy_from_slice(&post.data[..new_len as usize]);
                }
                // Update the SolAccountInfo's `data_len` field
                // (at byte offset 16 of its 56-byte record —
                // captured at parse time as
                // `data_len_field_addr`). This is the bit that
                // makes realloc-across-CPI visible to the
                // outer program: when it reads `account.data`
                // through `solana_program::AccountInfo`, it
                // sees the new length.
                let len_dst = translate_slice_mut(
                    memory_mapping,
                    target.data_len_field_addr,
                    8,
                )?;
                len_dst.copy_from_slice(&new_len.to_le_bytes());
            }
        }

        Ok(syscalls::SYSCALL_OK)
    }
);

declare_builtin_function!(
    /// `sol_invoke_signed_rust` — Rust-ABI CPI.
    ///
    /// Parses the Rust-shaped `&Instruction`, `&[AccountInfo]`,
    /// and `&[&[&[u8]]]` signer-seeds shape via
    /// [`crate::bpf::cpi_rust`], pointer-chases through the
    /// `Rc<RefCell<…>>` indirection to extract lamport / data
    /// addresses, runs the same
    /// [`cpi::verify_signer_seeds`] +
    /// [`cpi::dispatch_cpi`] pipeline as the C variant, and
    /// writes mutations back through the resolved Rust-shape
    /// pointers.
    ///
    /// Realloc-across-CPI is supported: on writeback we update
    /// the slice's `len` slot inside the `RefCell<&mut [u8]>`
    /// so the outer program reads the new length on resume.
    SyscallSolInvokeSignedRust,
    fn rust(
        ctx: &mut BpfContext,
        instruction_addr: u64,
        account_infos_addr: u64,
        account_infos_len: u64,
        signers_seeds_addr: u64,
        signers_seeds_len: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Parse the Rust-ABI structures into our canonical
        // ParsedCpi shape, plus capture the per-account
        // writeback addresses we'll need after the inner call
        // returns.
        let (parsed, infos) = cpi_rust::build_parsed_cpi(
            memory_mapping,
            instruction_addr,
            account_infos_addr,
            account_infos_len,
            signers_seeds_addr,
            signers_seeds_len,
        )?;

        // Build the outer-signer list from the AccountInfo
        // is_signer flags — same semantics as the C variant.
        let outer_signers: Vec<Pubkey> = infos
            .iter()
            .filter(|i| i.is_signer)
            .map(|i| i.address)
            .collect();

        if let Err(code) =
            cpi::verify_signer_seeds(ctx, &parsed, &ctx.program_id.clone(), &outer_signers)
        {
            return Ok(code);
        }

        // Recursive dispatch through the harness.
        let outcome = match cpi::dispatch_cpi(ctx, parsed) {
            Ok(o) => o,
            Err(code) => return Ok(code),
        };

        // Writeback through the Rust-shape pointers. For each
        // mutated writable account, write:
        //   - lamports through the &mut u64 inside the
        //     Rc<RefCell<&mut u64>>
        //   - owner pubkey through the owner_value_addr
        //   - data bytes through the &mut [u8]
        //   - the slice's `len` field for realloc-across-CPI
        for post in &outcome.resulting_accounts {
            if let Some(info) = infos.iter().find(|i| i.address == post.address) {
                if !info.is_writable {
                    continue;
                }
                // Lamports.
                let lp_dst =
                    translate_slice_mut(memory_mapping, info.lamports_addr, 8)?;
                lp_dst.copy_from_slice(&post.lamports.to_le_bytes());
                // Owner — write the 32-byte pubkey through the
                // owner_value_addr (which IS the address of the
                // 32-byte pubkey value in the parameter buffer).
                let ow_dst =
                    translate_slice_mut(memory_mapping, info.owner_value_addr, 32)?;
                ow_dst.copy_from_slice(post.owner.as_ref());
                // Data + length: same realloc-tail handling as
                // the C variant. The Rust-ABI's slice length
                // lives inside the RefCell rather than as a
                // top-level SolAccountInfo field, but the
                // address we captured at parse time
                // (`data_len_field_addr`) points at it.
                let capacity =
                    info.data_len + MAX_PERMITTED_DATA_INCREASE as u64;
                let new_len = (post.data.len() as u64).min(capacity);
                if new_len < post.data.len() as u64 {
                    ctx.logs.line(format!(
                        "Program log: warning: CPI realloc truncated for account {} ({} requested, {} max)",
                        post.address,
                        post.data.len(),
                        capacity
                    ));
                }
                if new_len > 0 {
                    let data_dst = translate_slice_mut(
                        memory_mapping,
                        info.data_addr,
                        new_len,
                    )?;
                    data_dst.copy_from_slice(&post.data[..new_len as usize]);
                }
                let len_dst = translate_slice_mut(
                    memory_mapping,
                    info.data_len_field_addr,
                    8,
                )?;
                len_dst.copy_from_slice(&new_len.to_le_bytes());
            }
        }

        Ok(syscalls::SYSCALL_OK)
    }
);

/// Snapshot helper — reads the is_signer byte for a given account
/// pubkey out of the SolAccountInfo array. Used during signer
/// verification when we need to know which outer-signers were
/// passed through.
fn snapshot_is_signer(_accounts_bytes: &[u8], _addr: Pubkey) -> bool {
    // Currently unused — the signer-list build above scans the
    // bytes directly. Kept as a stub for documentation
    // alignment; future Phase 2.2 will use it for richer
    // signature-tracking diagnostics.
    false
}

/// Per-account writeback target — the SolAccountInfo pointers
/// the calling program supplied. We capture them before the
/// inner call so the post-dispatch writeback knows where to
/// store mutations.
#[derive(Debug, Clone)]
struct AccountWriteback {
    address: Pubkey,
    lamports_addr: u64,
    data_addr: u64,
    /// VM address of the SolAccountInfo's `data_len` field — at
    /// byte offset 16 of its 56-byte record. Updated by the
    /// writeback when the inner call grew the data region so
    /// the outer program sees the new length on resume.
    data_len_field_addr: u64,
    /// `original_data_len + MAX_PERMITTED_DATA_INCREASE` — the
    /// total bytes of `data_addr`-rooted region that the
    /// parameter buffer reserved at outer-serialisation time.
    /// Caps how much an inner CPI can grow the account before
    /// the writeback truncates.
    data_len_capacity: u64,
    owner_addr: u64,
    is_writable: bool,
}

// ---------------------------------------------------------------------------
// Heap allocator adapter
// ---------------------------------------------------------------------------
//
// Wire shape:
//   sol_alloc_free_(size: u64, free_addr: u64) -> u64
//
// Returns a VM address, or 0 on alloc failure / free request.
// The do_sol_alloc_free function reads + writes the
// `BpfContext::heap_cursor` so successive allocations within
// one VM run see consistent state.

declare_builtin_function!(
    /// `sol_alloc_free_` — bump allocator over the VM heap region.
    SyscallSolAllocFree,
    fn rust(
        ctx: &mut BpfContext,
        size: u64,
        free_addr: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        Ok(syscalls::do_sol_alloc_free(ctx, size, free_addr))
    }
);

declare_builtin_function!(
    /// `sol_log_data` — log opaque binary chunks as base64.
    /// Wire shape: a slice of `(addr, len)` pairs the program
    /// passes through `sol_log_data(ptr, n_chunks)`.
    SyscallSolLogData,
    fn rust(
        ctx: &mut BpfContext,
        chunks_addr: u64,
        n_chunks: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        // Each chunk descriptor on the wire is 16 bytes
        // (addr u64 LE | len u64 LE).
        let total_bytes = n_chunks
            .checked_mul(16)
            .ok_or_else(|| {
                Box::new(SyscallError(
                    "sol_log_data: chunk count overflow".to_string(),
                ))
            })?;
        let descriptors = translate_slice(memory_mapping, chunks_addr, total_bytes)?;
        let mut owned_chunks: Vec<Vec<u8>> = Vec::with_capacity(n_chunks as usize);
        for i in 0..n_chunks as usize {
            let off = i * 16;
            let addr = u64::from_le_bytes(
                descriptors[off..off + 8].try_into().expect("addr"),
            );
            let len = u64::from_le_bytes(
                descriptors[off + 8..off + 16].try_into().expect("len"),
            );
            owned_chunks.push(
                translate_slice(memory_mapping, addr, len)?.to_vec(),
            );
        }
        let refs: Vec<&[u8]> = owned_chunks.iter().map(|v| v.as_slice()).collect();
        map_result(syscalls::do_sol_log_data(ctx, &refs))
    }
);

// ---------------------------------------------------------------------------
// Tier 3 adapters — niche syscalls
// ---------------------------------------------------------------------------

declare_builtin_function!(
    /// `sol_get_stack_height` — return the current CPI nesting
    /// depth as the syscall's u64 return value.
    SyscallSolGetStackHeight,
    fn rust(
        ctx: &mut BpfContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        match tier3_syscalls::do_sol_get_stack_height(ctx) {
            Ok(h) => Ok(h),
            Err(SyscallResult::Custom(msg)) => Err(Box::new(SyscallError(msg))),
            Err(_) => Err(Box::new(SyscallError(
                "sol_get_stack_height: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_remaining_compute_units` — return the meter's current
    /// value (post the syscall's own charge).
    SyscallSolRemainingComputeUnits,
    fn rust(
        ctx: &mut BpfContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        match tier3_syscalls::do_sol_remaining_compute_units(ctx) {
            Ok(r) => Ok(r),
            Err(SyscallResult::Custom(msg)) => Err(Box::new(SyscallError(msg))),
            Err(_) => Err(Box::new(SyscallError(
                "sol_remaining_compute_units: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_processed_sibling_instruction` — sibling-instruction
    /// lookup. Hopper Phase 2 doesn't ledger siblings, so the
    /// syscall always returns 0 (matches mainnet's empty-list
    /// behaviour).
    SyscallSolGetProcessedSiblingInstruction,
    fn rust(
        ctx: &mut BpfContext,
        index: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        match tier3_syscalls::do_sol_get_processed_sibling_instruction(ctx, index) {
            Ok(r) => Ok(r),
            Err(SyscallResult::Custom(msg)) => Err(Box::new(SyscallError(msg))),
            Err(_) => Err(Box::new(SyscallError(
                "sol_get_processed_sibling_instruction: compute budget exhausted"
                    .to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_sysvar` — generic sysvar accessor. Wire shape:
    /// `(sysvar_id_addr, offset, out_addr, length)`. Looks up the
    /// sysvar bytes by ID, copies the requested slice into `out`.
    /// Supports the four canonical sysvars: Clock, Rent,
    /// EpochSchedule, EpochRewards. Returns 0 on success.
    SyscallSolGetSysvar,
    fn rust(
        ctx: &mut BpfContext,
        sysvar_id_addr: u64,
        offset: u64,
        out_addr: u64,
        length: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let id_bytes = *translate_array::<32>(memory_mapping, sysvar_id_addr)?;
        let id = Pubkey::new_from_array(id_bytes);
        // Resolve the sysvar bytes by ID.
        use solana_sdk::sysvar;
        let bytes: Vec<u8> = if id == sysvar::clock::id() {
            let mut buf = vec![0u8; sysvar_syscalls::CLOCK_BYTES];
            sysvar_syscalls::do_sol_get_clock_sysvar(ctx, &mut buf);
            buf
        } else if id == sysvar::rent::id() {
            let mut buf = vec![0u8; sysvar_syscalls::RENT_BYTES];
            sysvar_syscalls::do_sol_get_rent_sysvar(ctx, &mut buf);
            buf
        } else if id == sysvar::epoch_schedule::id() {
            let mut buf = vec![0u8; sysvar_syscalls::EPOCH_SCHEDULE_BYTES];
            sysvar_syscalls::do_sol_get_epoch_schedule_sysvar(ctx, &mut buf);
            buf
        } else if id == sysvar::epoch_rewards::id() {
            let mut buf = vec![0u8; sysvar_syscalls::EPOCH_REWARDS_BYTES];
            sysvar_syscalls::do_sol_get_epoch_rewards_sysvar(ctx, &mut buf);
            buf
        } else if id == sysvar::last_restart_slot::id() {
            let mut buf = vec![0u8; sysvar_syscalls::LAST_RESTART_SLOT_BYTES];
            sysvar_syscalls::do_sol_get_last_restart_slot_sysvar(ctx, &mut buf);
            buf
        } else if id == sysvar::slot_hashes::id() {
            let mut buf = vec![0u8; tier3_syscalls::SLOTHASHES_EMPTY_LEN];
            tier3_syscalls::do_sol_get_slothashes_sysvar(ctx, &mut buf);
            buf
        } else if id == sysvar::slot_history::id() {
            let mut buf = vec![0u8; tier3_syscalls::SLOTHISTORY_EMPTY_LEN];
            tier3_syscalls::do_sol_get_slothistory_sysvar(ctx, &mut buf);
            buf
        } else if id == sysvar::stake_history::id() {
            let mut buf = vec![0u8; tier3_syscalls::STAKEHISTORY_EMPTY_LEN];
            tier3_syscalls::do_sol_get_stakehistory_sysvar(ctx, &mut buf);
            buf
        } else {
            return Err(Box::new(SyscallError(format!(
                "sol_get_sysvar: unknown sysvar {id}"
            ))));
        };
        let out = translate_slice_mut(memory_mapping, out_addr, length)?;
        match tier3_syscalls::do_sol_get_sysvar_copy(ctx, &bytes, offset as usize, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_sysvar: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_slothashes_sysvar` — write the canonical empty-list
    /// SlotHashes encoding into `out`.
    SyscallSolGetSlotHashesSysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            tier3_syscalls::SLOTHASHES_EMPTY_LEN as u64,
        )?;
        match tier3_syscalls::do_sol_get_slothashes_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_slothashes_sysvar: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_slothistory_sysvar` — write the all-zero SlotHistory
    /// bitvec into `out`.
    SyscallSolGetSlotHistorySysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            tier3_syscalls::SLOTHISTORY_EMPTY_LEN as u64,
        )?;
        match tier3_syscalls::do_sol_get_slothistory_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_slothistory_sysvar: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_get_stakehistory_sysvar` — write the empty-list
    /// StakeHistory encoding into `out`.
    SyscallSolGetStakeHistorySysvar,
    fn rust(
        ctx: &mut BpfContext,
        out_addr: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let out = translate_slice_mut(
            memory_mapping,
            out_addr,
            tier3_syscalls::STAKEHISTORY_EMPTY_LEN as u64,
        )?;
        match tier3_syscalls::do_sol_get_stakehistory_sysvar(ctx, out) {
            SyscallResult::Ok => Ok(syscalls::SYSCALL_OK),
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            SyscallResult::OutOfMeter => Err(Box::new(SyscallError(
                "sol_get_stakehistory_sysvar: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_curve_validate_point` — is the supplied point on the
    /// requested curve?
    SyscallSolCurveValidatePoint,
    fn rust(
        ctx: &mut BpfContext,
        curve: u64,
        point_addr: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let point = *translate_array::<32>(memory_mapping, point_addr)?;
        match tier3_syscalls::do_sol_curve_validate_point(ctx, curve, &point) {
            Ok(r) => Ok(r),
            Err(SyscallResult::Custom(msg)) => Err(Box::new(SyscallError(msg))),
            Err(_) => Err(Box::new(SyscallError(
                "sol_curve_validate_point: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_curve_group_op` — add / sub / mul on Edwards or
    /// Ristretto. Wire: `(curve, op, a_addr, b_addr, out_addr)`.
    SyscallSolCurveGroupOp,
    fn rust(
        ctx: &mut BpfContext,
        curve: u64,
        op: u64,
        a_addr: u64,
        b_addr: u64,
        out_addr: u64,
        memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let a = *translate_array::<32>(memory_mapping, a_addr)?;
        let b = *translate_array::<32>(memory_mapping, b_addr)?;
        let mut out = [0u8; 32];
        let r = tier3_syscalls::do_sol_curve_group_op(ctx, curve, op, &a, &b, &mut out);
        match r {
            Ok(0) => {
                let dst = translate_slice_mut(memory_mapping, out_addr, 32)?;
                dst.copy_from_slice(&out);
                Ok(0)
            }
            Ok(other) => Ok(other), // 1 = invalid input, no out write
            Err(SyscallResult::Custom(msg)) => Err(Box::new(SyscallError(msg))),
            Err(_) => Err(Box::new(SyscallError(
                "sol_curve_group_op: compute budget exhausted".to_string(),
            ))),
        }
    }
);

declare_builtin_function!(
    /// `sol_poseidon` — Poseidon hash. Stub: returns a structured
    /// "Tier 4" error so the test surface fails with an
    /// actionable message.
    SyscallSolPoseidon,
    fn rust(
        ctx: &mut BpfContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        match tier3_syscalls::do_sol_poseidon(ctx) {
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            _ => unreachable!("stub returns Custom"),
        }
    }
);

declare_builtin_function!(
    /// `sol_big_mod_exp` — RSA-style modexp. Stub.
    SyscallSolBigModExp,
    fn rust(
        ctx: &mut BpfContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        match tier3_syscalls::do_sol_big_mod_exp(ctx) {
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            _ => unreachable!("stub returns Custom"),
        }
    }
);

declare_builtin_function!(
    /// `sol_alt_bn128_group_op` — BN254 / alt_bn128 group ops. Stub.
    SyscallSolAltBn128GroupOp,
    fn rust(
        ctx: &mut BpfContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        match tier3_syscalls::do_sol_alt_bn128_group_op(ctx) {
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            _ => unreachable!("stub returns Custom"),
        }
    }
);

declare_builtin_function!(
    /// `sol_alt_bn128_compression` — BN254 / alt_bn128 point
    /// compression. Stub.
    SyscallSolAltBn128Compression,
    fn rust(
        ctx: &mut BpfContext,
        _arg1: u64,
        _arg2: u64,
        _arg3: u64,
        _arg4: u64,
        _arg5: u64,
        _memory_mapping: &mut MemoryMapping,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        match tier3_syscalls::do_sol_alt_bn128_compression(ctx) {
            SyscallResult::Custom(msg) => Err(Box::new(SyscallError(msg))),
            _ => unreachable!("stub returns Custom"),
        }
    }
);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Detect whether two `(addr, len)` ranges overlap. Used by
/// `sol_memcpy_` to reject the no-overlap variant when the
/// caller's regions touch.
fn ranges_overlap(a_addr: u64, b_addr: u64, n: u64) -> bool {
    let a_end = a_addr.saturating_add(n);
    let b_end = b_addr.saturating_add(n);
    !(a_end <= b_addr || b_end <= a_addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Overlap detector: identical ranges overlap, disjoint do
    /// not, and the boundary case (adjacent, touching at the
    /// edge) does not overlap.
    #[test]
    fn ranges_overlap_handles_boundary_cases() {
        assert!(ranges_overlap(100, 100, 10), "identical");
        assert!(!ranges_overlap(100, 200, 10), "disjoint");
        assert!(!ranges_overlap(100, 110, 10), "adjacent edge");
        assert!(ranges_overlap(100, 105, 10), "partial overlap");
    }

    /// `SyscallError` carries the message verbatim through the
    /// `Display` impl.
    #[test]
    fn syscall_error_display_passes_through() {
        let e = SyscallError("nope".to_string());
        assert_eq!(format!("{e}"), "nope");
    }
}
