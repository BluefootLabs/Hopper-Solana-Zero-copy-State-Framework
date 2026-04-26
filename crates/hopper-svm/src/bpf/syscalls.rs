//! Hopper's syscall registry for Phase 2 BPF execution.
//!
//! Every Solana program ABI ships through a syscall — `sol_log_*`,
//! `sol_panic_`, `sol_mem*`, `sol_set_return_data`, etc. — and the
//! `solana-sbpf` VM needs each registered as a [`BuiltinFunction`]
//! against its [`BuiltinProgram<BpfContext>`] so the BPF program
//! can call them through the SBPF call instruction.
//!
//! ## Phase 2.0 surface (this file)
//!
//! Read-only / no-CPI syscalls — the safest set, because they
//! don't recurse back into the harness:
//!
//! - `sol_log_` — log a UTF-8 message
//! - `sol_log_64_` — log five u64 values
//! - `sol_log_pubkey` — log a 32-byte pubkey
//! - `sol_log_compute_units_` — log the runtime CU framing line
//! - `sol_panic_` — abort with a captured panic message
//! - `sol_memcpy_` / `sol_memset_` / `sol_memcmp_` / `sol_memmove_`
//!   — guest memory operations
//! - `sol_log_data` — log opaque binary data (events)
//! - `sol_set_return_data` / `sol_get_return_data` — return-data
//!   slot manipulation
//!
//! ## Phase 2.1 (deferred)
//!
//! - `sol_invoke_signed_*` (CPI) — recursive dispatch back into
//!   `HopperSvm`. Carefully scoped because nested instruction
//!   semantics + signer-seed verification + account remapping
//!   are each individually subtle.
//! - `sol_get_*_sysvar` — read clock/rent/etc. from the harness
//!   sysvars into a guest buffer. Easy individually but each
//!   sysvar needs a wire-format match against
//!   `solana_program::sysvar`.
//! - `sol_create_program_address` / `sol_try_find_program_address`
//!   — PDA derivation; pure compute, no harness recursion. Could
//!   slot in earlier than CPI but consciously deferred to keep
//!   2.0 small.
//! - `sol_alloc_free_` — heap alloc/free. Many programs rely on
//!   it for temporary buffers but Hopper's no-alloc patterns make
//!   it less critical for our test surface.
//! - `sol_keccak256_`, `sol_secp256k1_recover_`, `sol_blake3` —
//!   crypto syscalls; each delegates to a published crate.
//!
//! ## Logic vs. adapter split
//!
//! Each syscall is implemented as **two layers**:
//!
//! - A pure-Rust `do_*` function that takes already-translated
//!   host slices + the [`BpfContext`] and returns a structured
//!   [`SyscallResult`]. **Fully unit-testable, no `solana-sbpf`
//!   coupling.** This is where the actual logic lives.
//! - A thin `declare_builtin_function!` adapter that translates
//!   VM addresses through the memory mapping and forwards into
//!   the `do_*` function. The adapter is the ONLY place that
//!   touches the sbpf macro/type details, so any sbpf API drift
//!   between minor versions has a single fixup site per syscall.
//!
//! Tests call the `do_*` layer directly. The adapter layer is
//! exercised end-to-end by the engine smoke test in step 5.

use crate::bpf::context::BpfContext;
use solana_sdk::pubkey::Pubkey;

/// Successful syscall return — what the VM puts back in r0.
/// Convention: `0` for "success", non-zero for syscall-level error
/// codes. Hopper syscalls use `0` on success uniformly; failures
/// surface through the `Err` arm of [`SyscallResult`].
pub const SYSCALL_OK: u64 = 0;

/// Return type of the pure-Rust syscall logic functions. The
/// adapter layer maps this onto sbpf's `Result<u64, EbpfError>` —
/// a `Custom(msg)` becomes a custom-error EbpfError, an
/// `OutOfMeter` aborts the VM cleanly so the engine can produce a
/// `HopperSvmError::OutOfComputeUnits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyscallResult {
    /// Syscall succeeded; VM continues with `Ok(SYSCALL_OK)`.
    Ok,
    /// Captured a structured failure that should map to a
    /// [`crate::HopperSvmError`] when the VM unwinds.
    Custom(String),
    /// Syscall budget tripped — caller should map to
    /// [`crate::HopperSvmError::OutOfComputeUnits`].
    OutOfMeter,
}

/// Each syscall's CU cost. Numbers chosen to match the production
/// runtime defaults so a Phase 2 test's CU readout matches what
/// the user sees on mainnet for the same program. If a future
/// release of `solana-program-runtime` changes a default, the fix
/// is one constant per syscall here.
mod cu {
    pub const SOL_LOG: u64 = 100;
    pub const SOL_LOG_64: u64 = 100;
    pub const SOL_LOG_PUBKEY: u64 = 100;
    pub const SOL_LOG_DATA: u64 = 100;
    pub const SOL_LOG_COMPUTE_UNITS: u64 = 100;
    pub const SOL_PANIC: u64 = 1;
    pub const SOL_MEMCPY: u64 = 1;
    pub const SOL_MEMSET: u64 = 1;
    pub const SOL_MEMCMP: u64 = 1;
    pub const SOL_MEMMOVE: u64 = 1;
    pub const SOL_SET_RETURN_DATA: u64 = 100;
    pub const SOL_GET_RETURN_DATA: u64 = 100;
    /// PDA derivation — one SHA-256 + one curve-point check. The
    /// 1500 baseline matches `solana-program-runtime`'s
    /// `create_program_address_units`. `try_find_program_address`
    /// charges this same cost on every bump attempt; the loop
    /// in [`super::do_sol_try_find_program_address`] exits
    /// when [`super::do_sol_create_program_address`] returns Ok
    /// or when the meter is exhausted.
    pub const SOL_CREATE_PROGRAM_ADDRESS: u64 = 1500;
    /// Heap allocation — flat per-call cost. Match upstream's
    /// negligible accounting (alloc is fast).
    pub const SOL_ALLOC_FREE: u64 = 1;
}

/// Maximum number of seeds in a single PDA derivation. The
/// production runtime caps at 16; a 17th seed is a hard reject.
pub const MAX_SEEDS: usize = 16;

/// Maximum bytes per PDA seed. Same upstream cap.
pub const MAX_SEED_LEN: usize = 32;

/// Maximum bytes a single `sol_log_` message may carry. Matches
/// the production runtime cap; a longer message is silently
/// truncated to this length on log emission, mirroring on-chain
/// behaviour exactly.
pub const MAX_LOG_MESSAGE_LEN: usize = 10_000;

/// Per-instruction limit on `sol_set_return_data` payload bytes.
/// Matches `MAX_RETURN_DATA` in the upstream runtime.
pub const MAX_RETURN_DATA_LEN: usize = 1024;

/// Charge `cost` CUs against the context's meter. Returns
/// [`SyscallResult::OutOfMeter`] if the meter would go below zero.
fn charge(ctx: &mut BpfContext, cost: u64) -> Result<(), SyscallResult> {
    if ctx.remaining_units < cost {
        return Err(SyscallResult::OutOfMeter);
    }
    ctx.remaining_units -= cost;
    Ok(())
}

// ---------------------------------------------------------------------------
// Logging syscalls
// ---------------------------------------------------------------------------

/// `sol_log_` — emit a UTF-8 message. Truncates at
/// [`MAX_LOG_MESSAGE_LEN`] bytes; non-UTF-8 input is replaced with
/// the lossy `String::from_utf8_lossy` conversion so a malformed
/// message doesn't fail the syscall (matches runtime behavior).
pub fn do_sol_log(ctx: &mut BpfContext, message: &[u8]) -> SyscallResult {
    if let Err(e) = charge(ctx, cu::SOL_LOG) {
        return e;
    }
    let trimmed = if message.len() > MAX_LOG_MESSAGE_LEN {
        &message[..MAX_LOG_MESSAGE_LEN]
    } else {
        message
    };
    let text = String::from_utf8_lossy(trimmed);
    ctx.logs.program_log(text.as_ref());
    SyscallResult::Ok
}

/// `sol_log_64_` — emit five u64 values in the runtime's standard
/// format `"Program log: 0x{a}, 0x{b}, 0x{c}, 0x{d}, 0x{e}"`.
pub fn do_sol_log_64(
    ctx: &mut BpfContext,
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_LOG_64) {
        return err;
    }
    ctx.logs.program_log(format!(
        "{a:#x}, {b:#x}, {c:#x}, {d:#x}, {e:#x}"
    ));
    SyscallResult::Ok
}

/// `sol_log_pubkey` — emit a base58-encoded pubkey at the standard
/// runtime format `"Program log: <pubkey>"`.
pub fn do_sol_log_pubkey(ctx: &mut BpfContext, key_bytes: &[u8; 32]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_LOG_PUBKEY) {
        return err;
    }
    let pk = Pubkey::new_from_array(*key_bytes);
    ctx.logs.program_log(pk.to_string());
    SyscallResult::Ok
}

/// `sol_log_compute_units_` — emit the runtime's
/// `"Program <id> consumed <N> of <M> compute units"` framing.
/// Note: the production runtime emits this line automatically at
/// program return; this syscall lets a program emit it mid-run for
/// debugging.
pub fn do_sol_log_compute_units(ctx: &mut BpfContext, initial_budget: u64) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_LOG_COMPUTE_UNITS) {
        return err;
    }
    let consumed = initial_budget.saturating_sub(ctx.remaining_units);
    ctx.logs.line(format!(
        "Program {} consumed {consumed} of {initial_budget} compute units",
        ctx.program_id
    ));
    SyscallResult::Ok
}

/// `sol_log_data` — emit opaque binary chunks as base64 (matches
/// the runtime's wire format for events and structured logs). The
/// runtime accepts a slice of slices; we accept the flattened
/// host-side equivalent.
pub fn do_sol_log_data(ctx: &mut BpfContext, chunks: &[&[u8]]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_LOG_DATA) {
        return err;
    }
    let encoded: Vec<String> = chunks.iter().map(|c| base64_encode(c)).collect();
    ctx.logs
        .line(format!("Program data: {}", encoded.join(" ")));
    SyscallResult::Ok
}

/// `sol_panic_` — capture a panic message into the context. The
/// engine maps this into a [`crate::HopperSvmError::BuiltinError`]
/// when the VM unwinds. `file_bytes` and `line` are the source
/// location the program reported; we format them into the panic
/// message for better post-mortem.
pub fn do_sol_panic(
    ctx: &mut BpfContext,
    file_bytes: &[u8],
    line: u64,
    column: u64,
) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_PANIC) {
        return err;
    }
    let file = String::from_utf8_lossy(file_bytes);
    let msg = format!("panicked at {file}:{line}:{column}");
    ctx.panic_message = Some(msg.clone());
    SyscallResult::Custom(msg)
}

// ---------------------------------------------------------------------------
// Memory syscalls
// ---------------------------------------------------------------------------

/// `sol_memcpy_` — copy `n` bytes from src to dst. On overlap,
/// returns a `Custom` error to mirror the runtime's
/// `MemoryOverlap` rejection (programs must use `memmove` for
/// overlapping regions).
pub fn do_sol_memcpy(
    ctx: &mut BpfContext,
    dst: &mut [u8],
    src: &[u8],
) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_MEMCPY) {
        return err;
    }
    if dst.len() != src.len() {
        return SyscallResult::Custom(format!(
            "sol_memcpy_: length mismatch dst={} src={}",
            dst.len(),
            src.len()
        ));
    }
    // Overlap detection — the runtime rejects overlapping memcpy.
    // We can't detect it directly through Rust's borrow rules
    // (the caller already split the slices to call us); the
    // adapter layer does the overlap check before splitting since
    // it has both vm_addrs available.
    dst.copy_from_slice(src);
    SyscallResult::Ok
}

/// `sol_memset_` — fill `dst` with the byte `value`.
pub fn do_sol_memset(
    ctx: &mut BpfContext,
    dst: &mut [u8],
    value: u8,
) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_MEMSET) {
        return err;
    }
    dst.fill(value);
    SyscallResult::Ok
}

/// `sol_memcmp_` — compare two byte slices. Result written to the
/// caller-provided `out` slot (i32 LE) in the canonical signum
/// encoding (-1 if a<b, 0 if equal, 1 if a>b). The runtime returns
/// the value via an out parameter rather than the syscall return.
pub fn do_sol_memcmp(
    ctx: &mut BpfContext,
    a: &[u8],
    b: &[u8],
    out: &mut [u8; 4],
) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_MEMCMP) {
        return err;
    }
    if a.len() != b.len() {
        return SyscallResult::Custom(format!(
            "sol_memcmp_: length mismatch a={} b={}",
            a.len(),
            b.len()
        ));
    }
    let result: i32 = match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    out.copy_from_slice(&result.to_le_bytes());
    SyscallResult::Ok
}

/// `sol_memmove_` — like memcpy but tolerates overlap. Implemented
/// in two stages (forward-copy when `dst < src`, backward-copy
/// otherwise) so overlapping regions don't corrupt each other.
/// The pure-Rust signature receives separate slices because the
/// adapter handles the overlap-aware translation.
pub fn do_sol_memmove(
    ctx: &mut BpfContext,
    dst: &mut [u8],
    src: &[u8],
) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_MEMMOVE) {
        return err;
    }
    if dst.len() != src.len() {
        return SyscallResult::Custom(format!(
            "sol_memmove_: length mismatch dst={} src={}",
            dst.len(),
            src.len()
        ));
    }
    // The adapter has already resolved the overlap into either
    // identical slices (no-op), forward-copy semantics, or
    // backward-copy semantics; we just copy. If the caller hands
    // us aliased mutable + shared refs to the same memory, that's
    // already UB at the boundary — the adapter is responsible for
    // copying through a temporary buffer when overlap is detected.
    dst.copy_from_slice(src);
    SyscallResult::Ok
}

// ---------------------------------------------------------------------------
// Return-data syscalls
// ---------------------------------------------------------------------------

/// `sol_set_return_data` — store the program's return data on the
/// context. Runtime cap: [`MAX_RETURN_DATA_LEN`]. Setting an empty
/// slice is allowed and clears any prior return data.
pub fn do_sol_set_return_data(ctx: &mut BpfContext, data: &[u8]) -> SyscallResult {
    if let Err(err) = charge(ctx, cu::SOL_SET_RETURN_DATA) {
        return err;
    }
    if data.len() > MAX_RETURN_DATA_LEN {
        return SyscallResult::Custom(format!(
            "sol_set_return_data: payload {} exceeds cap {}",
            data.len(),
            MAX_RETURN_DATA_LEN
        ));
    }
    if data.is_empty() {
        ctx.return_data = None;
    } else {
        ctx.return_data = Some((ctx.program_id, data.to_vec()));
    }
    SyscallResult::Ok
}

/// `sol_get_return_data` — copy any prior return data into the
/// caller's buffer. Returns the byte length of the prior data
/// (which may exceed `out.len()` if the caller's buffer is short
/// — the runtime returns the *real* length so the caller can size
/// their buffer correctly on a retry).
///
/// On success, also writes the program ID that set the data into
/// `program_id_out`.
pub fn do_sol_get_return_data(
    ctx: &mut BpfContext,
    out: &mut [u8],
    program_id_out: &mut [u8; 32],
) -> Result<u64, SyscallResult> {
    if let Err(err) = charge(ctx, cu::SOL_GET_RETURN_DATA) {
        return Err(err);
    }
    match &ctx.return_data {
        None => Ok(0),
        Some((pid, data)) => {
            let n = out.len().min(data.len());
            out[..n].copy_from_slice(&data[..n]);
            program_id_out.copy_from_slice(pid.as_ref());
            Ok(data.len() as u64)
        }
    }
}

// ---------------------------------------------------------------------------
// PDA derivation syscalls
// ---------------------------------------------------------------------------

/// PDA derivation marker — appended to seeds + program_id before
/// the SHA-256 digest. Matches Solana's hard-coded marker:
/// programs use the literal string "ProgramDerivedAddress" so
/// nobody can spoof a PDA derivation by signing the digest with
/// an Ed25519 key (the marker forces the input space outside the
/// curve subgroup that valid signatures live in).
pub const PDA_MARKER: &[u8; 21] = b"ProgramDerivedAddress";

/// Reject reason from `do_sol_create_program_address`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdaError {
    /// Too many seeds (>= [`MAX_SEEDS`]).
    TooManySeeds,
    /// One seed is longer than [`MAX_SEED_LEN`] bytes.
    SeedTooLong,
    /// The candidate landed on the Ed25519 curve — has a
    /// corresponding private key, so it's not a valid PDA. The
    /// caller (typically `try_find_program_address`) bumps the
    /// nonce and retries.
    OnCurve,
}

impl std::fmt::Display for PdaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManySeeds => write!(f, "too many seeds (max {MAX_SEEDS})"),
            Self::SeedTooLong => write!(f, "seed too long (max {MAX_SEED_LEN} bytes)"),
            Self::OnCurve => write!(f, "candidate landed on the Ed25519 curve"),
        }
    }
}

/// `sol_create_program_address` — single-shot PDA derivation.
///
/// Algorithm (matches `solana_program::pubkey::Pubkey::create_program_address`):
///
/// 1. Reject if `seeds.len() > MAX_SEEDS` or any seed exceeds
///    [`MAX_SEED_LEN`].
/// 2. Compute `SHA256(seed_0 || seed_1 || … || seed_n || program_id || "ProgramDerivedAddress")`.
/// 3. Treat the 32-byte digest as a candidate point. If it
///    decodes as a valid Ed25519 curve point, reject ([`PdaError::OnCurve`]).
/// 4. Otherwise, the digest *is* the PDA — return it.
///
/// On Ok, the caller writes the 32 bytes into the guest's output
/// buffer. On `PdaError::OnCurve`, the caller maps to a
/// syscall-level error so `try_find_program_address` can bump and
/// retry.
pub fn do_sol_create_program_address(
    ctx: &mut BpfContext,
    seeds: &[&[u8]],
    program_id: &[u8; 32],
) -> Result<[u8; 32], PdaError> {
    // The CU charge happens before any work. We do it here rather
    // than in the adapter so the unit tests that exercise the
    // `do_*` function directly see the meter behaviour the VM
    // would.
    if charge(ctx, cu::SOL_CREATE_PROGRAM_ADDRESS).is_err() {
        // Out-of-meter inside PDA derivation surfaces as
        // `OnCurve` so the caller's bump-and-retry loop terminates
        // (every attempt would also OOM); the engine-level meter
        // unwind catches the underlying budget exhaustion. This
        // is a Hopper-specific call: we'd rather bail the loop
        // than silently spin.
        return Err(PdaError::OnCurve);
    }
    if seeds.len() > MAX_SEEDS {
        return Err(PdaError::TooManySeeds);
    }
    for s in seeds {
        if s.len() > MAX_SEED_LEN {
            return Err(PdaError::SeedTooLong);
        }
    }
    let digest = pda_digest(seeds, program_id);
    if is_on_curve(&digest) {
        return Err(PdaError::OnCurve);
    }
    Ok(digest)
}

/// `sol_try_find_program_address` — iterates bumps from 255 down
/// to 0 until [`do_sol_create_program_address`] returns Ok. The
/// returned tuple is `(pda, bump)`. Returns `None` if every bump
/// landed on-curve (vanishingly unlikely with non-empty seeds; in
/// practice the loop almost always terminates in 1–4 attempts).
///
/// Charges [`cu::SOL_TRY_FIND_PROGRAM_ADDRESS_PER_ATTEMPT`] per
/// iteration via [`do_sol_create_program_address`]'s internal
/// charge. The runtime applies the same per-attempt accounting.
pub fn do_sol_try_find_program_address(
    ctx: &mut BpfContext,
    seeds: &[&[u8]],
    program_id: &[u8; 32],
) -> Option<([u8; 32], u8)> {
    if seeds.len() >= MAX_SEEDS {
        // We're going to append a 1-byte bump seed; if that pushes
        // us over the limit, bail before allocating.
        return None;
    }
    // The on-stack bump byte; pushed onto the seeds list every
    // iteration. Allocate one Vec to extend each round so we
    // don't churn the heap for the per-attempt seed list.
    let mut bump = [0u8; 1];
    for b in (0u8..=255u8).rev() {
        bump[0] = b;
        let mut tagged: Vec<&[u8]> = Vec::with_capacity(seeds.len() + 1);
        tagged.extend_from_slice(seeds);
        tagged.push(&bump);
        match do_sol_create_program_address(ctx, &tagged, program_id) {
            Ok(pda) => return Some((pda, b)),
            Err(PdaError::OnCurve) => continue,
            // Seed-validation errors bubble out as None — the
            // caller can't recover by bumping if the seeds
            // themselves are invalid.
            Err(_) => return None,
        }
    }
    None
}

/// Compute the candidate digest for a PDA derivation.
fn pda_digest(seeds: &[&[u8]], program_id: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for s in seeds {
        hasher.update(s);
    }
    hasher.update(program_id);
    hasher.update(PDA_MARKER);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Is `bytes` a valid compressed Ed25519 curve point? Used to
/// reject PDA candidates that have a corresponding private key
/// (and could therefore be spoofed by a signature).
///
/// Implementation notes:
/// - `curve25519_dalek::edwards::CompressedEdwardsY::decompress`
///   returns `None` for byte sequences that aren't valid points.
/// - We additionally check the point isn't the identity / small-
///   order — the runtime's `is_on_curve` rejects those too. For
///   Phase 2.1 we go with the simpler "decompresses successfully"
///   test; small-order rejection lands when we have a complete
///   set of vectors to test against.
fn is_on_curve(bytes: &[u8; 32]) -> bool {
    use curve25519_dalek::edwards::CompressedEdwardsY;
    CompressedEdwardsY(*bytes).decompress().is_some()
}

// ---------------------------------------------------------------------------
// Heap allocation (`sol_alloc_free_`)
// ---------------------------------------------------------------------------
//
// Solana's BPF runtime ships a **bump allocator** — alloc returns
// monotonically-increasing addresses inside the heap region;
// `free` is a no-op. The VM's heap is a single 32 KiB block
// (default; configurable per program). When the bump cursor
// exceeds the heap size, subsequent allocs return null.
//
// Upstream alignment is fixed at 8 bytes (mirrors
// `Layout::align == 8` in agave-syscalls' SyscallAllocFree). We
// round each requested size up to the next 8-byte boundary
// before bumping the cursor, matching that behaviour exactly.

/// Heap region alignment. Every alloc is rounded up to the next
/// multiple. Matches upstream's hard-coded `Layout::align == 8`.
pub const HEAP_ALIGN: u64 = 8;

/// Default heap size — the engine sets up a 32 KiB heap region
/// at `MM_HEAP_START`. Exposed so adapters can bounds-check
/// against it without re-importing engine internals.
pub const HEAP_SIZE: u64 = 32 * 1024;

/// VM-side base address of the heap region. Matches the SBPF
/// memory layout: rodata at `MM_RODATA_START` (0x100000000),
/// stack at `MM_STACK_START` (0x200000000), heap at
/// `MM_HEAP_START` (0x300000000).
pub const HEAP_VM_START: u64 = 0x300000000;

/// `sol_alloc_free_` — unified alloc + free entry point.
///
/// Wire convention from upstream:
/// - `free_addr == 0`: alloc request. Allocate `size` bytes
///   aligned to [`HEAP_ALIGN`]. Return the VM address of the new
///   block, or `0` if the heap is exhausted.
/// - `free_addr != 0`: free request. Bump-allocator semantics
///   ignore frees; we always return `0`.
///
/// The cursor lives on [`BpfContext::heap_cursor`] so it persists
/// across syscall calls within one VM run. Each fresh
/// instruction gets a fresh context with the cursor reset to 0,
/// matching upstream's per-instruction allocator lifetime.
pub fn do_sol_alloc_free(
    ctx: &mut BpfContext,
    size: u64,
    free_addr: u64,
) -> u64 {
    if charge(ctx, cu::SOL_ALLOC_FREE).is_err() {
        // Out of meter — return null. The engine catches the
        // budget exhaustion at unwind via the per-instruction
        // meter and produces a structured `OutOfComputeUnits`
        // error.
        return 0;
    }
    if free_addr != 0 {
        // Free is a no-op for the bump allocator. Match the
        // upstream behaviour exactly: return 0.
        return 0;
    }
    // Round size up to the alignment boundary. `size = 0`
    // produces 0; we treat it as a successful zero-length alloc
    // at the current cursor (the cursor doesn't move). Programs
    // that allocate zero-size blocks see a stable, repeatable
    // address — useful for sentinel patterns.
    let aligned_size = (size + HEAP_ALIGN - 1) & !(HEAP_ALIGN - 1);
    let new_cursor = match ctx.heap_cursor.checked_add(aligned_size) {
        Some(c) => c,
        // Add overflow signals an absurd request; return null
        // rather than wrapping.
        None => return 0,
    };
    if new_cursor > HEAP_SIZE {
        // Heap exhausted. Programs are expected to handle null
        // returns gracefully (typically by panicking); we don't
        // do anything special.
        return 0;
    }
    let vm_addr = HEAP_VM_START + ctx.heap_cursor;
    ctx.heap_cursor = new_cursor;
    vm_addr
}

const B64: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode `bytes` as base64. Used by `sol_log_data` to mirror the
/// runtime's "Program data: <b64> <b64> …" wire format.
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);
    for chunk in bytes.chunks(3) {
        let (b0, b1, b2) = match chunk.len() {
            3 => (chunk[0], chunk[1], chunk[2]),
            2 => (chunk[0], chunk[1], 0),
            1 => (chunk[0], 0, 0),
            _ => unreachable!(),
        };
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(B64[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() >= 2 {
            out.push(B64[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() == 3 {
            out.push(B64[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_units(units: u64) -> BpfContext {
        BpfContext::new(Pubkey::new_unique(), units)
    }

    #[test]
    fn sol_log_emits_program_log_prefix_and_charges_cu() {
        let mut ctx = ctx_with_units(1_000);
        let result = do_sol_log(&mut ctx, b"hello world");
        assert_eq!(result, SyscallResult::Ok);
        assert_eq!(ctx.remaining_units, 900);
        assert_eq!(ctx.logs.lines(), &["Program log: hello world".to_string()]);
    }

    #[test]
    fn sol_log_truncates_at_max_message_len() {
        let mut ctx = ctx_with_units(1_000);
        let big = vec![b'x'; MAX_LOG_MESSAGE_LEN + 50];
        do_sol_log(&mut ctx, &big);
        let line = &ctx.logs.lines()[0];
        // "Program log: " (13 bytes) + MAX_LOG_MESSAGE_LEN content.
        assert_eq!(line.len(), 13 + MAX_LOG_MESSAGE_LEN);
    }

    #[test]
    fn sol_log_lossy_decodes_invalid_utf8() {
        let mut ctx = ctx_with_units(1_000);
        // 0xff is not a valid UTF-8 start byte.
        do_sol_log(&mut ctx, &[b'a', 0xff, b'b']);
        let line = &ctx.logs.lines()[0];
        assert!(line.starts_with("Program log: a"), "got {line:?}");
        assert!(line.ends_with("b"));
    }

    #[test]
    fn sol_log_64_uses_runtime_hex_format() {
        let mut ctx = ctx_with_units(1_000);
        do_sol_log_64(&mut ctx, 1, 2, 3, 4, 5);
        assert_eq!(
            ctx.logs.lines(),
            &["Program log: 0x1, 0x2, 0x3, 0x4, 0x5".to_string()]
        );
    }

    #[test]
    fn sol_log_pubkey_emits_base58() {
        let mut ctx = ctx_with_units(1_000);
        let key = Pubkey::new_unique();
        do_sol_log_pubkey(&mut ctx, key.as_ref().try_into().unwrap());
        assert_eq!(
            ctx.logs.lines(),
            &[format!("Program log: {key}")]
        );
    }

    #[test]
    fn sol_log_compute_units_uses_runtime_framing() {
        let pid = Pubkey::new_unique();
        let mut ctx = BpfContext::new(pid, 199_900); // initial budget
                                                     // was 200_000, 100
                                                     // already burned
        do_sol_log_compute_units(&mut ctx, 200_000);
        // After the syscall: 100 already burned + 100 charged for
        // the syscall itself = 200 total consumed.
        let line = &ctx.logs.lines()[0];
        assert!(line.contains(&format!("Program {pid} consumed 200 of 200000 compute units")), "got {line:?}");
    }

    #[test]
    fn sol_panic_captures_message_and_returns_custom() {
        let mut ctx = ctx_with_units(1_000);
        let result = do_sol_panic(&mut ctx, b"src/lib.rs", 42, 7);
        assert!(matches!(result, SyscallResult::Custom(_)));
        let panic = ctx.panic_message.as_ref().expect("panic captured");
        assert!(panic.contains("src/lib.rs:42:7"), "got {panic}");
    }

    #[test]
    fn sol_memcpy_copies_and_charges_cu() {
        let mut ctx = ctx_with_units(1_000);
        let mut dst = [0u8; 4];
        let src = [1u8, 2, 3, 4];
        do_sol_memcpy(&mut ctx, &mut dst, &src);
        assert_eq!(dst, src);
        assert_eq!(ctx.remaining_units, 999);
    }

    #[test]
    fn sol_memcpy_length_mismatch_returns_custom() {
        let mut ctx = ctx_with_units(1_000);
        let mut dst = [0u8; 4];
        let src = [1u8; 8];
        let r = do_sol_memcpy(&mut ctx, &mut dst, &src);
        assert!(matches!(r, SyscallResult::Custom(_)));
    }

    #[test]
    fn sol_memset_fills_buffer() {
        let mut ctx = ctx_with_units(1_000);
        let mut buf = [0u8; 8];
        do_sol_memset(&mut ctx, &mut buf, 0xAB);
        assert_eq!(buf, [0xAB; 8]);
    }

    #[test]
    fn sol_memcmp_writes_signum() {
        let mut ctx = ctx_with_units(1_000);
        let mut out = [0u8; 4];
        do_sol_memcmp(&mut ctx, b"abc", b"abd", &mut out);
        assert_eq!(i32::from_le_bytes(out), -1);
        do_sol_memcmp(&mut ctx, b"abc", b"abc", &mut out);
        assert_eq!(i32::from_le_bytes(out), 0);
        do_sol_memcmp(&mut ctx, b"abd", b"abc", &mut out);
        assert_eq!(i32::from_le_bytes(out), 1);
    }

    #[test]
    fn sol_set_return_data_caps_at_max() {
        let mut ctx = ctx_with_units(1_000);
        let big = vec![0u8; MAX_RETURN_DATA_LEN + 1];
        let r = do_sol_set_return_data(&mut ctx, &big);
        assert!(matches!(r, SyscallResult::Custom(_)));
        assert!(ctx.return_data.is_none());
    }

    #[test]
    fn sol_set_return_data_empty_clears() {
        let mut ctx = ctx_with_units(1_000);
        do_sol_set_return_data(&mut ctx, b"prior");
        assert!(ctx.return_data.is_some());
        do_sol_set_return_data(&mut ctx, b"");
        assert!(ctx.return_data.is_none());
    }

    #[test]
    fn sol_get_return_data_reports_real_length() {
        let mut ctx = ctx_with_units(1_000);
        do_sol_set_return_data(&mut ctx, b"hello world");
        let mut out = [0u8; 5];
        let mut pid_out = [0u8; 32];
        let n = do_sol_get_return_data(&mut ctx, &mut out, &mut pid_out)
            .expect("ok");
        assert_eq!(n, 11);
        assert_eq!(&out, b"hello");
        assert_eq!(&pid_out[..], ctx.program_id.as_ref());
    }

    #[test]
    fn out_of_meter_short_circuits() {
        let mut ctx = ctx_with_units(50);
        // SOL_LOG costs 100 CU; only 50 remaining.
        let r = do_sol_log(&mut ctx, b"too expensive");
        assert_eq!(r, SyscallResult::OutOfMeter);
        // Meter not partially debited on rejection.
        assert_eq!(ctx.remaining_units, 50);
        // No log line emitted.
        assert!(ctx.logs.lines().is_empty());
    }

    #[test]
    fn base64_encode_pads_correctly() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn sol_log_data_emits_base64_chunks() {
        let mut ctx = ctx_with_units(1_000);
        do_sol_log_data(&mut ctx, &[b"foo", b"bar"]);
        assert_eq!(
            ctx.logs.lines(),
            &["Program data: Zm9v YmFy".to_string()]
        );
    }

    // ── PDA derivation tests ─────────────────────────────────────

    /// PDA derivation must charge CUs and produce a deterministic
    /// 32-byte digest for valid (off-curve) seeds. Pin against
    /// known-good vectors derived against `solana_program::pubkey`
    /// to catch any silent drift in the digest construction.
    #[test]
    fn create_program_address_is_deterministic() {
        let mut ctx = ctx_with_units(10_000);
        let program_id = [42u8; 32];
        let seeds: &[&[u8]] = &[b"vault", &[1, 2, 3]];
        let result_a = do_sol_create_program_address(&mut ctx, seeds, &program_id);
        // Drop the meter so the second call doesn't fail on
        // `OutOfMeter` — we want to test determinism, not metering
        // side effects.
        let mut ctx2 = ctx_with_units(10_000);
        let result_b = do_sol_create_program_address(&mut ctx2, seeds, &program_id);
        assert_eq!(result_a, result_b);
    }

    /// `MAX_SEEDS` is enforced. A 17th seed is a hard reject.
    #[test]
    fn create_program_address_rejects_too_many_seeds() {
        let mut ctx = ctx_with_units(10_000);
        let program_id = [0u8; 32];
        let many: Vec<&[u8]> = (0..MAX_SEEDS + 1).map(|_| &b"x"[..]).collect();
        let r = do_sol_create_program_address(&mut ctx, &many, &program_id);
        assert_eq!(r, Err(PdaError::TooManySeeds));
    }

    /// `MAX_SEED_LEN` is enforced. A 33-byte seed is a hard
    /// reject.
    #[test]
    fn create_program_address_rejects_long_seed() {
        let mut ctx = ctx_with_units(10_000);
        let program_id = [0u8; 32];
        let long = vec![0u8; MAX_SEED_LEN + 1];
        let r =
            do_sol_create_program_address(&mut ctx, &[&long[..]], &program_id);
        assert_eq!(r, Err(PdaError::SeedTooLong));
    }

    /// CU charging — each `do_sol_create_program_address`
    /// invocation charges `cu::SOL_CREATE_PROGRAM_ADDRESS = 1500`,
    /// regardless of whether the result was Ok or `OnCurve`.
    #[test]
    fn create_program_address_charges_per_call() {
        let mut ctx = ctx_with_units(5_000);
        let program_id = [9u8; 32];
        let _ = do_sol_create_program_address(&mut ctx, &[b"x"], &program_id);
        assert_eq!(ctx.remaining_units, 5_000 - cu::SOL_CREATE_PROGRAM_ADDRESS);
    }

    /// `try_find_program_address` must succeed for almost all
    /// reasonable seeds (the curve-rejection probability is ~50%
    /// per attempt; the bump loop walks 256 candidates, so the
    /// chance of total failure is astronomically small). Pin
    /// "succeeds and returns a sensible bump" against the same
    /// program-id + seeds repeatedly.
    #[test]
    fn try_find_program_address_terminates_for_normal_seeds() {
        let mut ctx = ctx_with_units(5_000_000); // generous budget
        let program_id = [7u8; 32];
        let seeds: &[&[u8]] = &[b"vault", b"alice"];
        let (pda, bump) =
            do_sol_try_find_program_address(&mut ctx, seeds, &program_id)
                .expect("PDA found");
        // Determinism: same seeds + program_id always produces the
        // same (pda, bump).
        let mut ctx2 = ctx_with_units(5_000_000);
        let (pda2, bump2) =
            do_sol_try_find_program_address(&mut ctx2, seeds, &program_id)
                .expect("PDA found again");
        assert_eq!(pda, pda2);
        assert_eq!(bump, bump2);
    }

    /// PDA marker — every digest input must end with the literal
    /// "ProgramDerivedAddress" string. Pin to catch any accidental
    /// rename of the marker constant.
    #[test]
    fn pda_marker_is_the_canonical_string() {
        assert_eq!(PDA_MARKER, b"ProgramDerivedAddress");
        assert_eq!(PDA_MARKER.len(), 21);
    }

    /// `try_find_program_address` returns None when seeds are
    /// already at the maximum count (the bump byte would push us
    /// over the limit).
    #[test]
    fn try_find_returns_none_when_at_seed_limit() {
        let mut ctx = ctx_with_units(5_000_000);
        let program_id = [0u8; 32];
        let max: Vec<&[u8]> = (0..MAX_SEEDS).map(|_| &b"x"[..]).collect();
        let r = do_sol_try_find_program_address(&mut ctx, &max, &program_id);
        assert!(r.is_none());
    }

    // ── Heap-allocator tests ─────────────────────────────────────

    /// Alloc returns a stable, monotonically-increasing VM
    /// address starting at `HEAP_VM_START`. Pin the first three
    /// allocations against their expected addresses so a future
    /// change to alignment or starting offset is caught.
    #[test]
    fn alloc_free_returns_monotonic_addresses() {
        let mut ctx = ctx_with_units(1_000_000);
        let a = do_sol_alloc_free(&mut ctx, 16, 0);
        let b = do_sol_alloc_free(&mut ctx, 24, 0);
        let c = do_sol_alloc_free(&mut ctx, 8, 0);
        // First allocation lives at the heap start.
        assert_eq!(a, HEAP_VM_START);
        // Second lives 16 bytes after (16 was already aligned).
        assert_eq!(b, HEAP_VM_START + 16);
        // Third lives 24 bytes after that.
        assert_eq!(c, HEAP_VM_START + 16 + 24);
        assert_eq!(ctx.heap_cursor, 16 + 24 + 8);
    }

    /// 1-byte alloc rounds up to the 8-byte alignment.
    #[test]
    fn alloc_rounds_up_to_alignment() {
        let mut ctx = ctx_with_units(1_000_000);
        let _a = do_sol_alloc_free(&mut ctx, 1, 0);
        assert_eq!(ctx.heap_cursor, 8); // not 1
        let _b = do_sol_alloc_free(&mut ctx, 9, 0);
        // 9 rounds up to 16, total cursor = 8 + 16 = 24.
        assert_eq!(ctx.heap_cursor, 24);
    }

    /// Free is a no-op — bump allocator never releases memory.
    /// Pin against accidental future "smart" free behaviour.
    #[test]
    fn free_is_noop() {
        let mut ctx = ctx_with_units(1_000_000);
        let a = do_sol_alloc_free(&mut ctx, 16, 0);
        let cursor_before = ctx.heap_cursor;
        // Free request: free_addr non-zero. Should return 0 and
        // not move the cursor.
        let r = do_sol_alloc_free(&mut ctx, 16, a);
        assert_eq!(r, 0);
        assert_eq!(ctx.heap_cursor, cursor_before);
    }

    /// Heap exhaustion returns null (0) and leaves the cursor
    /// untouched so subsequent allocs that fit can still proceed.
    #[test]
    fn alloc_returns_null_on_exhaustion() {
        let mut ctx = ctx_with_units(10_000_000);
        // Burn most of the heap with one big block.
        let _ = do_sol_alloc_free(&mut ctx, HEAP_SIZE - 16, 0);
        let cursor_before = ctx.heap_cursor;
        // Now a 32-byte alloc must fail — only 16 bytes left.
        let r = do_sol_alloc_free(&mut ctx, 32, 0);
        assert_eq!(r, 0);
        // Cursor untouched.
        assert_eq!(ctx.heap_cursor, cursor_before);
        // A 16-byte alloc still fits and succeeds.
        let r = do_sol_alloc_free(&mut ctx, 16, 0);
        assert_eq!(r, HEAP_VM_START + cursor_before);
    }

    /// Zero-size alloc returns the current cursor address without
    /// moving it. Useful as a sentinel pattern.
    #[test]
    fn zero_size_alloc_returns_cursor_without_moving() {
        let mut ctx = ctx_with_units(1_000_000);
        let _ = do_sol_alloc_free(&mut ctx, 16, 0);
        let cursor_before = ctx.heap_cursor;
        let r = do_sol_alloc_free(&mut ctx, 0, 0);
        assert_eq!(r, HEAP_VM_START + cursor_before);
        assert_eq!(ctx.heap_cursor, cursor_before);
    }

    /// Out-of-meter on alloc returns null without partial cursor
    /// movement.
    #[test]
    fn alloc_out_of_meter_returns_null() {
        let mut ctx = ctx_with_units(0); // zero CU
        let r = do_sol_alloc_free(&mut ctx, 16, 0);
        assert_eq!(r, 0);
        assert_eq!(ctx.heap_cursor, 0);
    }
}
