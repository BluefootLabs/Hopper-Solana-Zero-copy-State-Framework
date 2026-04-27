//! `hopper-svm-ffi` — C-ABI wrapper around `hopper-svm`.
//!
//! ## Why this crate exists
//!
//! Tier 4 of the Hopper SVM roadmap is "TypeScript / Python
//! language bindings". Rather than maintaining two separate
//! reimplementations of the harness against `napi-rs` (Node) and
//! `pyo3` (Python), we ship one extern-"C" surface here and let
//! both host languages bind through it via their canonical FFI
//! tooling:
//!
//! - **Node / TypeScript** — `koffi` (the modern node-ffi
//!   replacement, zero-build-step) loads `libhopper_svm_ffi.so`
//!   directly. Type-safe TS wrappers live in
//!   `bindings/typescript/`.
//! - **Python** — `cffi` or `ctypes` loads the same `.so`. Type-
//!   stubs + a Pythonic API live in `bindings/python/`. A future
//!   pass swaps `cffi` for `pyo3` if we need richer dataclass
//!   shaping; the C surface stays the same.
//!
//! ## Surface design
//!
//! Two opaque handle types (`HopperSvmHandle`,
//! `ExecutionResultHandle`) expose the underlying Rust types as
//! `*mut c_void` to host languages. Constructors return owning
//! handles; the destructor `*_free` releases the underlying
//! allocation. Operations take handles + plain-old-data
//! parameters (32-byte pubkeys as pointers, byte buffers as
//! `(ptr, len)` pairs, lamports as `u64`).
//!
//! ## Account-state model
//!
//! `HopperSvm` itself is stateless w.r.t. account state — every
//! call to `process_instruction` takes the account list as a
//! parameter and returns the post-state. The FFI keeps an
//! internal `Vec<KeyedAccount>` aligned to that model: host
//! callers seed accounts via [`hopper_svm_set_account`], the FFI
//! caches them, and [`hopper_svm_dispatch`] feeds the cache into
//! `process_instruction`. After dispatch, the FFI replaces its
//! cached state with the returned `resulting_accounts`, so the
//! next dispatch picks up where the previous one left off.
//!
//! ## ABI stability
//!
//! Every public function uses `extern "C"` and `#[no_mangle]`.
//! Structs that cross the FFI boundary (`FfiAccountMeta`,
//! `FfiAccount`) are `#[repr(C)]` with explicit field ordering.
//! Adding a new field to a struct is a breaking change at this
//! layer; we'll add new dedicated functions instead so existing
//! host-language wrappers keep working.
//!
//! ## Memory ownership
//!
//! - Strings returned by the FFI (e.g. `hopper_svm_outcome_log_at`)
//!   are owned by Rust. Host languages must call
//!   `hopper_svm_string_free` to release them. The
//!   `HopperFfiString` struct carries the pointer + length so
//!   freeing knows the exact buffer size — it's NOT a
//!   null-terminated C string (logs may contain interior nulls).
//! - Byte buffers passed INTO the FFI (`hopper_svm_set_account`'s
//!   `data_ptr`) are read-once during the call and copied into
//!   Hopper-owned memory. The caller can free them immediately
//!   on return.
//! - Handles returned by `*_new` / `dispatch` are owned by the
//!   host. Forgetting to call `*_free` leaks the underlying
//!   allocation; the FFI shouldn't crash but tests will leak
//!   memory under the language runtime.

#![allow(clippy::missing_safety_doc)] // Each fn carries a /// SAFETY block.

use hopper_svm::account::KeyedAccount;
use hopper_svm::error::HopperSvmError;
use hopper_svm::HopperSvm;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use std::ffi::c_void;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Handles
// ---------------------------------------------------------------------------

/// Opaque pointer to a Hopper SVM instance + its cached account
/// state. Created via [`hopper_svm_new`], freed via
/// [`hopper_svm_free`].
pub type HopperSvmHandle = *mut c_void;

/// Opaque pointer to a captured execution result.
pub type ExecutionResultHandle = *mut c_void;

/// Internal — state behind `HopperSvmHandle`. The FFI keeps the
/// account cache here so successive dispatches see consistent
/// state without the host language having to re-pass everything
/// each time.
struct FfiHarness {
    svm: HopperSvm,
    accounts: Vec<KeyedAccount>,
}

/// Internal — what the result handle actually points at.
struct FfiOutcome {
    error_message: Option<String>,
    consumed_units: u64,
    transaction_fee_paid: u64,
    logs: Vec<String>,
    post_accounts: Vec<KeyedAccount>,
    return_data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// String marshalling
// ---------------------------------------------------------------------------

/// FFI-safe owned string. Carries a UTF-8 byte buffer + length,
/// owned by the FFI side. Host languages should copy out the
/// content immediately and call [`hopper_svm_string_free`] to
/// release the underlying allocation.
///
/// Why not a null-terminated C string: log lines can contain
/// interior null bytes (raw byte data via `sol_log_data`), and
/// `koffi` / `cffi` both handle `(ptr, len)` shapes natively.
#[repr(C)]
pub struct HopperFfiString {
    /// UTF-8 bytes. NOT null-terminated. May be null if `len == 0`.
    pub ptr: *mut u8,
    /// Byte length. Use this together with `ptr` to materialise
    /// the string in the host language.
    pub len: usize,
}

impl HopperFfiString {
    fn from_string(s: String) -> Self {
        if s.is_empty() {
            return Self {
                ptr: std::ptr::null_mut(),
                len: 0,
            };
        }
        let bytes = s.into_bytes();
        let mut boxed = bytes.into_boxed_slice();
        let len = boxed.len();
        let ptr = boxed.as_mut_ptr();
        // Forget the box — the host now owns the allocation, must
        // call `hopper_svm_string_free` to reclaim it.
        std::mem::forget(boxed);
        Self { ptr, len }
    }
}

/// Release the bytes backing a [`HopperFfiString`]. The host
/// language must call this for every non-empty string returned
/// by the FFI to avoid leaks. Calling on a `len == 0` string is
/// a no-op.
///
/// SAFETY: `s` must be a `HopperFfiString` produced by this
/// crate. Calling with arbitrary bytes is undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_string_free(s: HopperFfiString) {
    if s.ptr.is_null() || s.len == 0 {
        return;
    }
    let _ = Box::from_raw(std::slice::from_raw_parts_mut(s.ptr, s.len));
}

// ---------------------------------------------------------------------------
// HopperSvm lifecycle
// ---------------------------------------------------------------------------

/// Construct a fresh handle with the bare default configuration
/// (system program registered, no SPL programs, no compute
/// budget program). Use [`hopper_svm_with_solana_runtime`] to
/// register the full validator-side surface in one call.
///
/// Returns null on out-of-memory (extremely unlikely).
#[no_mangle]
pub extern "C" fn hopper_svm_new() -> HopperSvmHandle {
    let harness = FfiHarness {
        svm: HopperSvm::new(),
        accounts: Vec::new(),
    };
    Box::into_raw(Box::new(Mutex::new(harness))) as HopperSvmHandle
}

/// Replace the harness's program registry with the full
/// validator-side runtime: System (default) + Compute Budget +
/// ALT + Config + Stake + Vote + SPL Token + Token-2022 + ATA.
/// Mirrors `HopperSvm::with_solana_runtime()` from the Rust API.
///
/// SAFETY: `handle` must be a non-null handle produced by
/// [`hopper_svm_new`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_with_solana_runtime(handle: HopperSvmHandle) {
    if handle.is_null() {
        return;
    }
    let mutex = &*(handle as *mut Mutex<FfiHarness>);
    let mut guard = mutex.lock().expect("hopper-svm-ffi: mutex poisoned");
    let owned = std::mem::replace(&mut guard.svm, HopperSvm::new());
    guard.svm = owned.with_solana_runtime();
}

/// Set the harness's compute-unit budget. Subsequent dispatches
/// use this as the per-transaction CU limit.
///
/// SAFETY: `handle` must be a non-null handle produced by
/// [`hopper_svm_new`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_set_compute_budget(handle: HopperSvmHandle, units: u64) {
    if handle.is_null() {
        return;
    }
    let mutex = &*(handle as *mut Mutex<FfiHarness>);
    let guard = mutex.lock().expect("hopper-svm-ffi: mutex poisoned");
    guard.svm.set_compute_budget(units);
}

/// Free a HopperSvm handle. The host must call this exactly once
/// per handle returned by [`hopper_svm_new`]. Subsequent calls on
/// the same handle (or on a null handle) are no-ops.
///
/// SAFETY: `handle` must be a handle produced by
/// [`hopper_svm_new`] and not previously freed.
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_free(handle: HopperSvmHandle) {
    if handle.is_null() {
        return;
    }
    let _ = Box::from_raw(handle as *mut Mutex<FfiHarness>);
}

// ---------------------------------------------------------------------------
// Account state
// ---------------------------------------------------------------------------

/// Seed an account into the harness's cached state — used by
/// tests to set up fixture state before dispatching an
/// instruction. Replaces any pre-existing account at the same
/// address.
///
/// Returns `true` on success, `false` on null-pointer args.
///
/// SAFETY: pointer params must be valid for `len` bytes:
/// - `address_ptr` — 32 bytes (pubkey)
/// - `owner_ptr` — 32 bytes (pubkey)
/// - `data_ptr` — `data_len` bytes (account data; copied)
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_set_account(
    handle: HopperSvmHandle,
    address_ptr: *const u8,
    lamports: u64,
    owner_ptr: *const u8,
    data_ptr: *const u8,
    data_len: usize,
    executable: bool,
) -> bool {
    if handle.is_null() || address_ptr.is_null() || owner_ptr.is_null() {
        return false;
    }
    let address = Pubkey::new_from_array(read_array_32(address_ptr));
    let owner = Pubkey::new_from_array(read_array_32(owner_ptr));
    let data = if data_len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(data_ptr, data_len).to_vec()
    };
    let account = KeyedAccount::new(address, lamports, owner, data, executable);
    let mutex = &*(handle as *mut Mutex<FfiHarness>);
    let mut guard = mutex.lock().expect("hopper-svm-ffi: mutex poisoned");
    if let Some(slot) = guard.accounts.iter_mut().find(|a| a.address == address) {
        *slot = account;
    } else {
        guard.accounts.push(account);
    }
    true
}

/// Read the lamport balance for a cached account. Returns
/// `u64::MAX` as a sentinel if the account isn't seeded. Host
/// languages should treat that as "unknown".
///
/// SAFETY: `address_ptr` must be valid for 32 bytes.
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_get_lamports(
    handle: HopperSvmHandle,
    address_ptr: *const u8,
) -> u64 {
    if handle.is_null() || address_ptr.is_null() {
        return u64::MAX;
    }
    let address = Pubkey::new_from_array(read_array_32(address_ptr));
    let mutex = &*(handle as *mut Mutex<FfiHarness>);
    let guard = mutex.lock().expect("hopper-svm-ffi: mutex poisoned");
    guard
        .accounts
        .iter()
        .find(|a| a.address == address)
        .map(|a| a.lamports)
        .unwrap_or(u64::MAX)
}

/// Read a cached account's data into a host-language buffer.
/// Returns the byte length actually written (which may be less
/// than `out_len` if the account's data is smaller). Returns
/// `usize::MAX` if the account isn't seeded.
///
/// SAFETY: `address_ptr` must be valid for 32 bytes;
/// `out_ptr` must be valid for `out_len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_get_account_data(
    handle: HopperSvmHandle,
    address_ptr: *const u8,
    out_ptr: *mut u8,
    out_len: usize,
) -> usize {
    if handle.is_null() || address_ptr.is_null() {
        return usize::MAX;
    }
    let address = Pubkey::new_from_array(read_array_32(address_ptr));
    let mutex = &*(handle as *mut Mutex<FfiHarness>);
    let guard = mutex.lock().expect("hopper-svm-ffi: mutex poisoned");
    let account = match guard.accounts.iter().find(|a| a.address == address) {
        Some(a) => a,
        None => return usize::MAX,
    };
    let n = account.data.len().min(out_len);
    if n > 0 && !out_ptr.is_null() {
        std::ptr::copy_nonoverlapping(account.data.as_ptr(), out_ptr, n);
    }
    n
}

// ---------------------------------------------------------------------------
// Instruction dispatch
// ---------------------------------------------------------------------------

/// Wire-shape `AccountMeta` for the FFI. Host languages
/// construct an array of these to describe an instruction's
/// account list.
#[repr(C)]
pub struct FfiAccountMeta {
    /// 32-byte pubkey. Reads through this pointer; the underlying
    /// bytes can be released by the host once the dispatch call
    /// returns.
    pub pubkey_ptr: *const u8,
    pub is_signer: bool,
    pub is_writable: bool,
}

/// Dispatch a single instruction through the harness, using the
/// FFI's cached account state. Returns an owning
/// [`ExecutionResultHandle`] that the host language can query
/// for logs, CU consumption, and post-state. Free with
/// [`hopper_svm_outcome_free`] when done.
///
/// On success the handle is non-null; on failure (null handle,
/// malformed pointers) returns null.
///
/// After dispatch, the harness's cached account state is
/// replaced with the returned `resulting_accounts` so the next
/// call sees the updated state. Host languages can re-read
/// post-dispatch state via [`hopper_svm_get_lamports`] /
/// [`hopper_svm_get_account_data`].
///
/// SAFETY: pointers as documented:
/// - `program_id_ptr` — 32 bytes
/// - `accounts_ptr` — array of `accounts_len` `FfiAccountMeta`s,
///   each with a valid 32-byte `pubkey_ptr`
/// - `data_ptr` — `data_len` bytes (instruction data)
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_dispatch(
    handle: HopperSvmHandle,
    program_id_ptr: *const u8,
    accounts_ptr: *const FfiAccountMeta,
    accounts_len: usize,
    data_ptr: *const u8,
    data_len: usize,
) -> ExecutionResultHandle {
    if handle.is_null() || program_id_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let program_id = Pubkey::new_from_array(read_array_32(program_id_ptr));
    let metas: Vec<AccountMeta> = if accounts_len == 0 {
        Vec::new()
    } else {
        if accounts_ptr.is_null() {
            return std::ptr::null_mut();
        }
        let raw = std::slice::from_raw_parts(accounts_ptr, accounts_len);
        raw.iter()
            .map(|m| AccountMeta {
                pubkey: Pubkey::new_from_array(read_array_32(m.pubkey_ptr)),
                is_signer: m.is_signer,
                is_writable: m.is_writable,
            })
            .collect()
    };
    let data = if data_len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(data_ptr, data_len).to_vec()
    };
    let ix = Instruction {
        program_id,
        accounts: metas,
        data,
    };
    let mutex = &*(handle as *mut Mutex<FfiHarness>);
    let mut guard = mutex.lock().expect("hopper-svm-ffi: mutex poisoned");
    let result = guard.svm.process_instruction(&ix, &guard.accounts);
    // Cache the post-state.
    guard.accounts = result.outcome.resulting_accounts.clone();
    let ffi = FfiOutcome {
        error_message: result.outcome.error.as_ref().map(error_message),
        consumed_units: result.outcome.compute_units_consumed,
        transaction_fee_paid: result.transaction_fee_paid,
        logs: result.logs.clone(),
        post_accounts: result.outcome.resulting_accounts.clone(),
        return_data: result.outcome.return_data.clone(),
    };
    Box::into_raw(Box::new(ffi)) as ExecutionResultHandle
}

/// `true` if the dispatch produced an error, `false` if it
/// succeeded. Read the message via
/// [`hopper_svm_outcome_error_message`].
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_is_error(handle: ExecutionResultHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let outcome = &*(handle as *mut FfiOutcome);
    outcome.error_message.is_some()
}

/// Captured error message, or empty if the dispatch succeeded.
/// The returned [`HopperFfiString`] must be freed via
/// [`hopper_svm_string_free`].
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_error_message(
    handle: ExecutionResultHandle,
) -> HopperFfiString {
    if handle.is_null() {
        return HopperFfiString {
            ptr: std::ptr::null_mut(),
            len: 0,
        };
    }
    let outcome = &*(handle as *mut FfiOutcome);
    HopperFfiString::from_string(outcome.error_message.clone().unwrap_or_default())
}

/// Compute units consumed by the dispatch (across all inner
/// instructions and the outer call).
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_consumed_units(handle: ExecutionResultHandle) -> u64 {
    if handle.is_null() {
        return 0;
    }
    let outcome = &*(handle as *mut FfiOutcome);
    outcome.consumed_units
}

/// Transaction fee paid (if the dispatch went through
/// `process_transaction`; always 0 for plain
/// `process_instruction` calls).
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_transaction_fee_paid(
    handle: ExecutionResultHandle,
) -> u64 {
    if handle.is_null() {
        return 0;
    }
    let outcome = &*(handle as *mut FfiOutcome);
    outcome.transaction_fee_paid
}

/// Number of log lines captured during dispatch. Use
/// [`hopper_svm_outcome_log_at`] to read each.
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_log_count(handle: ExecutionResultHandle) -> usize {
    if handle.is_null() {
        return 0;
    }
    let outcome = &*(handle as *mut FfiOutcome);
    outcome.logs.len()
}

/// Read a captured log line by index (0-based). Returns an
/// empty string on out-of-range index. Caller must
/// [`hopper_svm_string_free`] the returned bytes.
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_log_at(
    handle: ExecutionResultHandle,
    index: usize,
) -> HopperFfiString {
    if handle.is_null() {
        return HopperFfiString {
            ptr: std::ptr::null_mut(),
            len: 0,
        };
    }
    let outcome = &*(handle as *mut FfiOutcome);
    HopperFfiString::from_string(outcome.logs.get(index).cloned().unwrap_or_default())
}

/// Number of resulting accounts post-dispatch. Use
/// [`hopper_svm_outcome_account_at`] to read each.
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_account_count(handle: ExecutionResultHandle) -> usize {
    if handle.is_null() {
        return 0;
    }
    let outcome = &*(handle as *mut FfiOutcome);
    outcome.post_accounts.len()
}

/// FFI shape for a post-dispatch account. The `data_ptr` field
/// references Hopper-owned memory inside the [`FfiOutcome`] —
/// it remains valid until the outcome handle is freed. Host
/// languages should copy out the data immediately rather than
/// holding the pointer beyond the FfiOutcome's lifetime.
#[repr(C)]
pub struct FfiAccount {
    pub address: [u8; 32],
    pub owner: [u8; 32],
    pub lamports: u64,
    pub data_ptr: *const u8,
    pub data_len: usize,
    pub executable: bool,
    pub rent_epoch: u64,
}

/// Read the post-dispatch account at `index`. Out-of-range
/// returns a zeroed [`FfiAccount`]. The data pointer remains
/// valid until [`hopper_svm_outcome_free`] is called on the
/// handle.
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`].
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_account_at(
    handle: ExecutionResultHandle,
    index: usize,
) -> FfiAccount {
    let zero = FfiAccount {
        address: [0u8; 32],
        owner: [0u8; 32],
        lamports: 0,
        data_ptr: std::ptr::null(),
        data_len: 0,
        executable: false,
        rent_epoch: 0,
    };
    if handle.is_null() {
        return zero;
    }
    let outcome = &*(handle as *mut FfiOutcome);
    let acct = match outcome.post_accounts.get(index) {
        Some(a) => a,
        None => return zero,
    };
    FfiAccount {
        address: acct.address.to_bytes(),
        owner: acct.owner.to_bytes(),
        lamports: acct.lamports,
        data_ptr: acct.data.as_ptr(),
        data_len: acct.data.len(),
        executable: acct.executable,
        rent_epoch: acct.rent_epoch,
    }
}

/// Read the program return data, if the dispatched program
/// called `sol_set_return_data`. Returns the byte length the
/// program set; the FFI fills `out_ptr` up to `out_len`. If
/// the program didn't set return data, returns `0`.
///
/// SAFETY: `handle` must be a non-null handle from
/// [`hopper_svm_dispatch`]; `out_ptr` must be valid for
/// `out_len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_return_data(
    handle: ExecutionResultHandle,
    out_ptr: *mut u8,
    out_len: usize,
) -> usize {
    if handle.is_null() {
        return 0;
    }
    let outcome = &*(handle as *mut FfiOutcome);
    if outcome.return_data.is_empty() {
        return 0;
    }
    let n = outcome.return_data.len().min(out_len);
    if n > 0 && !out_ptr.is_null() {
        std::ptr::copy_nonoverlapping(outcome.return_data.as_ptr(), out_ptr, n);
    }
    outcome.return_data.len()
}

/// Free an outcome handle. Each
/// [`hopper_svm_dispatch`] result must be freed exactly once.
///
/// SAFETY: `handle` must be a handle from
/// [`hopper_svm_dispatch`] not previously freed.
#[no_mangle]
pub unsafe extern "C" fn hopper_svm_outcome_free(handle: ExecutionResultHandle) {
    if handle.is_null() {
        return;
    }
    let _ = Box::from_raw(handle as *mut FfiOutcome);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format an [`HopperSvmError`] for the FFI surface. Mirrors the
/// `Display` impl one-shot.
fn error_message(err: &HopperSvmError) -> String {
    err.to_string()
}

/// Read 32 bytes through a raw pointer into an owned array.
/// SAFETY: `ptr` must be valid for at least 32 bytes.
unsafe fn read_array_32(ptr: *const u8) -> [u8; 32] {
    let mut buf = [0u8; 32];
    if !ptr.is_null() {
        std::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), 32);
    }
    buf
}

/// Crate version. Useful for host-language wrappers that want to
/// assert they're loading a compatible build of the FFI lib.
/// Returns a static, null-terminated C string.
#[no_mangle]
pub extern "C" fn hopper_svm_ffi_version() -> *const u8 {
    // `concat!` makes sure the string is null-terminated at
    // compile time.
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a string through `from_string` + `string_free`
    /// without UB. Pin the byte length transit too.
    #[test]
    fn ffi_string_round_trip() {
        let msg = "hello hopper".to_string();
        let s = HopperFfiString::from_string(msg.clone());
        assert_eq!(s.len, msg.len());
        unsafe {
            // Verify the bytes round-trip before free.
            let view = std::slice::from_raw_parts(s.ptr, s.len);
            assert_eq!(view, msg.as_bytes());
            hopper_svm_string_free(s);
        }
    }

    /// Empty-string handling: should not allocate, ptr stays
    /// null, free is a no-op.
    #[test]
    fn ffi_string_empty_does_not_allocate() {
        let s = HopperFfiString::from_string(String::new());
        assert!(s.ptr.is_null());
        assert_eq!(s.len, 0);
        unsafe { hopper_svm_string_free(s) };
    }

    /// Lifecycle smoke: new -> with_solana_runtime -> free should
    /// not crash. Doesn't validate inner state — that's the
    /// `hopper-svm` crate's responsibility — but pins the FFI
    /// constructor / destructor edges.
    #[test]
    fn handle_lifecycle_does_not_crash() {
        let h = hopper_svm_new();
        assert!(!h.is_null());
        unsafe {
            hopper_svm_with_solana_runtime(h);
            hopper_svm_set_compute_budget(h, 200_000);
            hopper_svm_free(h);
        }
    }

    /// Reading lamports for an unseeded address returns the
    /// `u64::MAX` "unknown" sentinel.
    #[test]
    fn get_lamports_unknown_returns_sentinel() {
        let h = hopper_svm_new();
        let pk = [42u8; 32];
        unsafe {
            let l = hopper_svm_get_lamports(h, pk.as_ptr());
            assert_eq!(l, u64::MAX);
            hopper_svm_free(h);
        }
    }

    /// Set + get account round-trips lamports + data.
    #[test]
    fn set_and_get_account_round_trip() {
        let h = hopper_svm_new();
        let addr = [7u8; 32];
        let owner = [9u8; 32];
        let data = b"hopper-svm-ffi";
        unsafe {
            let ok = hopper_svm_set_account(
                h,
                addr.as_ptr(),
                12_345,
                owner.as_ptr(),
                data.as_ptr(),
                data.len(),
                false,
            );
            assert!(ok);
            let l = hopper_svm_get_lamports(h, addr.as_ptr());
            assert_eq!(l, 12_345);
            let mut buf = [0u8; 32];
            let n = hopper_svm_get_account_data(h, addr.as_ptr(), buf.as_mut_ptr(), buf.len());
            assert_eq!(n, data.len());
            assert_eq!(&buf[..n], data);
            hopper_svm_free(h);
        }
    }

    /// Replacing an existing account at the same address
    /// updates lamports + data in-place rather than appending.
    #[test]
    fn set_account_replaces_existing() {
        let h = hopper_svm_new();
        let addr = [7u8; 32];
        let owner = [9u8; 32];
        unsafe {
            hopper_svm_set_account(
                h,
                addr.as_ptr(),
                100,
                owner.as_ptr(),
                std::ptr::null(),
                0,
                false,
            );
            hopper_svm_set_account(
                h,
                addr.as_ptr(),
                200,
                owner.as_ptr(),
                std::ptr::null(),
                0,
                false,
            );
            assert_eq!(hopper_svm_get_lamports(h, addr.as_ptr()), 200);
            hopper_svm_free(h);
        }
    }

    /// Version string is null-terminated and parseable.
    #[test]
    fn ffi_version_is_well_formed() {
        let ptr = hopper_svm_ffi_version();
        assert!(!ptr.is_null());
        let mut len = 0;
        unsafe {
            while *ptr.add(len) != 0 {
                len += 1;
                if len > 64 {
                    panic!("version string too long");
                }
            }
            let bytes = std::slice::from_raw_parts(ptr, len);
            let s = std::str::from_utf8(bytes).expect("utf-8");
            assert!(s.contains('.'), "expected semver-shaped version, got {s}");
        }
    }
}
