// hopper-svm TypeScript bindings.
//
// Wraps the `hopper_svm_ffi` shared library (built from the
// `crates/hopper-svm-ffi` crate) via koffi — the modern
// zero-build-step ffi for Node. Two layers:
//
// 1. The raw FFI surface (`koffi.func` declarations matching the
//    extern "C" functions in `crates/hopper-svm-ffi/src/lib.rs`).
//    These map 1:1 to C names; type-safety lives in the wrapper
//    classes below.
// 2. A higher-level idiomatic wrapper (`HopperSvm`,
//    `ExecutionResult`) that owns the handle, exposes a Promise-
//    free synchronous surface (matches the test harness's
//    in-process model — there's no I/O to await), and frees the
//    handle on `dispose()`.
//
// The shared library is loaded from the path in the
// `HOPPER_SVM_LIB_PATH` env var, falling back to a platform-
// specific default (`libhopper_svm_ffi.so` on Linux,
// `libhopper_svm_ffi.dylib` on macOS, `hopper_svm_ffi.dll` on
// Windows). For local development, build the FFI crate first:
//
//   cargo build --release -p hopper-svm-ffi --features bpf-execution
//
// then point `HOPPER_SVM_LIB_PATH` at the resulting artifact.

import * as koffi from "koffi";
import * as os from "os";
import * as path from "path";

// ------------------------------------------------------------------
// Shared-library loading
// ------------------------------------------------------------------

function defaultLibPath(): string {
  const platform = os.platform();
  switch (platform) {
    case "linux":
      return "libhopper_svm_ffi.so";
    case "darwin":
      return "libhopper_svm_ffi.dylib";
    case "win32":
      return "hopper_svm_ffi.dll";
    default:
      throw new Error(`hopper-svm: unsupported platform ${platform}`);
  }
}

const libPath = process.env.HOPPER_SVM_LIB_PATH ?? defaultLibPath();
const lib = koffi.load(libPath);

// ------------------------------------------------------------------
// FFI types — match the structs in src/lib.rs exactly
// ------------------------------------------------------------------

// HopperFfiString: { ptr: *mut u8, len: usize }
const HopperFfiString = koffi.struct("HopperFfiString", {
  ptr: "uint8_t *",
  len: "size_t",
});

// FfiAccountMeta: { pubkey_ptr: *const u8, is_signer: bool, is_writable: bool }
const FfiAccountMeta = koffi.struct("FfiAccountMeta", {
  pubkey_ptr: "const uint8_t *",
  is_signer: "bool",
  is_writable: "bool",
});

// FfiAccount: { address: [u8; 32], owner: [u8; 32], lamports: u64,
//               data_ptr: *const u8, data_len: usize, executable: bool,
//               rent_epoch: u64 }
const FfiAccount = koffi.struct("FfiAccount", {
  address: koffi.array("uint8_t", 32, "Array"),
  owner: koffi.array("uint8_t", 32, "Array"),
  lamports: "uint64_t",
  data_ptr: "const uint8_t *",
  data_len: "size_t",
  executable: "bool",
  rent_epoch: "uint64_t",
});

// ------------------------------------------------------------------
// FFI declarations
// ------------------------------------------------------------------

const ffi = {
  // Lifecycle.
  hopper_svm_new: lib.func("hopper_svm_new", "void *", []),
  hopper_svm_with_solana_runtime: lib.func("hopper_svm_with_solana_runtime", "void", ["void *"]),
  hopper_svm_set_compute_budget: lib.func("hopper_svm_set_compute_budget", "void", ["void *", "uint64_t"]),
  hopper_svm_free: lib.func("hopper_svm_free", "void", ["void *"]),

  // Account state.
  hopper_svm_set_account: lib.func("hopper_svm_set_account", "bool", [
    "void *",
    "const uint8_t *",
    "uint64_t",
    "const uint8_t *",
    "const uint8_t *",
    "size_t",
    "bool",
  ]),
  hopper_svm_get_lamports: lib.func("hopper_svm_get_lamports", "uint64_t", ["void *", "const uint8_t *"]),
  hopper_svm_get_account_data: lib.func("hopper_svm_get_account_data", "size_t", [
    "void *",
    "const uint8_t *",
    "uint8_t *",
    "size_t",
  ]),

  // Dispatch.
  hopper_svm_dispatch: lib.func("hopper_svm_dispatch", "void *", [
    "void *",
    "const uint8_t *",
    koffi.pointer(FfiAccountMeta),
    "size_t",
    "const uint8_t *",
    "size_t",
  ]),

  // Outcome accessors.
  hopper_svm_outcome_is_error: lib.func("hopper_svm_outcome_is_error", "bool", ["void *"]),
  hopper_svm_outcome_error_message: lib.func("hopper_svm_outcome_error_message", HopperFfiString, ["void *"]),
  hopper_svm_outcome_consumed_units: lib.func("hopper_svm_outcome_consumed_units", "uint64_t", ["void *"]),
  hopper_svm_outcome_transaction_fee_paid: lib.func(
    "hopper_svm_outcome_transaction_fee_paid",
    "uint64_t",
    ["void *"],
  ),
  hopper_svm_outcome_log_count: lib.func("hopper_svm_outcome_log_count", "size_t", ["void *"]),
  hopper_svm_outcome_log_at: lib.func("hopper_svm_outcome_log_at", HopperFfiString, ["void *", "size_t"]),
  hopper_svm_outcome_account_count: lib.func("hopper_svm_outcome_account_count", "size_t", ["void *"]),
  hopper_svm_outcome_account_at: lib.func("hopper_svm_outcome_account_at", FfiAccount, ["void *", "size_t"]),
  hopper_svm_outcome_return_data: lib.func("hopper_svm_outcome_return_data", "size_t", [
    "void *",
    "uint8_t *",
    "size_t",
  ]),
  hopper_svm_outcome_free: lib.func("hopper_svm_outcome_free", "void", ["void *"]),

  // String helpers.
  hopper_svm_string_free: lib.func("hopper_svm_string_free", "void", [HopperFfiString]),
  hopper_svm_ffi_version: lib.func("hopper_svm_ffi_version", "const uint8_t *", []),
};

// ------------------------------------------------------------------
// Helpers — string / bytes marshalling
// ------------------------------------------------------------------

function readFfiString(ffiStr: { ptr: Buffer | null; len: number }): string {
  if (ffiStr.len === 0 || !ffiStr.ptr) {
    return "";
  }
  // koffi gives us a Buffer; copy out before freeing.
  const buf = koffi.decode(ffiStr.ptr, "char", ffiStr.len) as string;
  // koffi.decode returns the string directly when length is provided.
  // Free the underlying allocation.
  ffi.hopper_svm_string_free(ffiStr);
  return buf;
}

function ensure32Bytes(name: string, bytes: Uint8Array): Uint8Array {
  if (bytes.length !== 32) {
    throw new Error(`${name}: expected 32-byte pubkey, got ${bytes.length}`);
  }
  return bytes;
}

// ------------------------------------------------------------------
// Pubkey
// ------------------------------------------------------------------

/**
 * 32-byte Solana pubkey. The TS wrapper accepts either a raw
 * `Uint8Array` or a `Pubkey` instance for any pubkey-shaped
 * argument.
 */
export class Pubkey {
  readonly bytes: Uint8Array;

  constructor(bytes: Uint8Array) {
    this.bytes = ensure32Bytes("Pubkey", bytes);
  }

  /** Build from a base58-encoded string. */
  static fromBase58(s: string): Pubkey {
    return new Pubkey(base58Decode(s));
  }

  /** Build a fresh random pubkey — useful for test fixtures. */
  static unique(): Pubkey {
    const bytes = new Uint8Array(32);
    for (let i = 0; i < 32; i++) bytes[i] = Math.floor(Math.random() * 256);
    return new Pubkey(bytes);
  }

  toBase58(): string {
    return base58Encode(this.bytes);
  }

  toString(): string {
    return this.toBase58();
  }

  equals(other: Pubkey): boolean {
    if (other.bytes.length !== this.bytes.length) return false;
    for (let i = 0; i < this.bytes.length; i++) {
      if (this.bytes[i] !== other.bytes[i]) return false;
    }
    return true;
  }
}

// ------------------------------------------------------------------
// Public API — HopperSvm + ExecutionResult
// ------------------------------------------------------------------

/** Account-meta shape for instruction dispatch. */
export interface AccountMeta {
  pubkey: Pubkey | Uint8Array;
  isSigner: boolean;
  isWritable: boolean;
}

/** Instruction shape — mirrors `solana_sdk::instruction::Instruction`. */
export interface Instruction {
  programId: Pubkey | Uint8Array;
  accounts: AccountMeta[];
  data: Uint8Array;
}

/** Account-state shape returned from dispatches. */
export interface KeyedAccount {
  address: Pubkey;
  owner: Pubkey;
  lamports: bigint;
  data: Uint8Array;
  executable: boolean;
  rentEpoch: bigint;
}

/**
 * In-process Hopper SVM instance. Construct with `new HopperSvm()`
 * (or `HopperSvm.withSolanaRuntime()` for the full validator
 * surface), seed accounts, dispatch instructions, and call
 * `dispose()` when finished.
 */
export class HopperSvm {
  private handle: Buffer | null;

  /** Construct a bare harness — only the system program registered. */
  constructor() {
    const handle = ffi.hopper_svm_new() as Buffer;
    if (!handle) {
      throw new Error("hopper-svm: hopper_svm_new returned null");
    }
    this.handle = handle;
  }

  /**
   * Construct a harness pre-loaded with the full Solana runtime:
   * System (default) + Compute Budget + ALT + Config + Stake +
   * Vote + SPL Token + Token-2022 + ATA. Mirrors the Rust
   * builder `HopperSvm::with_solana_runtime()`.
   */
  static withSolanaRuntime(): HopperSvm {
    const svm = new HopperSvm();
    ffi.hopper_svm_with_solana_runtime(svm.handle!);
    return svm;
  }

  /**
   * Set the harness's compute-unit budget. Subsequent dispatches
   * use this as the per-transaction CU limit.
   */
  setComputeBudget(units: bigint): void {
    this.requireOpen();
    ffi.hopper_svm_set_compute_budget(this.handle!, units);
  }

  /**
   * Seed an account into the harness's cached state. Replaces
   * any pre-existing account at the same address.
   */
  setAccount(opts: {
    address: Pubkey | Uint8Array;
    lamports: bigint;
    owner: Pubkey | Uint8Array;
    data?: Uint8Array;
    executable?: boolean;
  }): void {
    this.requireOpen();
    const address = pubkeyBytes(opts.address);
    const owner = pubkeyBytes(opts.owner);
    const data = opts.data ?? new Uint8Array(0);
    const ok = ffi.hopper_svm_set_account(
      this.handle!,
      address,
      opts.lamports,
      owner,
      data.length > 0 ? data : null,
      data.length,
      opts.executable ?? false,
    );
    if (!ok) {
      throw new Error("hopper-svm: setAccount failed (null pointer args)");
    }
  }

  /**
   * Read the lamport balance for a cached account. Returns
   * `null` if the account isn't seeded.
   */
  getLamports(address: Pubkey | Uint8Array): bigint | null {
    this.requireOpen();
    const value = ffi.hopper_svm_get_lamports(this.handle!, pubkeyBytes(address));
    // The FFI uses u64::MAX as a sentinel for "unknown". JS sees it
    // as 0xFFFF_FFFF_FFFF_FFFFn after the bigint widen.
    if (value === 0xFFFFFFFFFFFFFFFFn) return null;
    return value;
  }

  /**
   * Dispatch one instruction. Returns an `ExecutionResult` the
   * caller can query. After dispatch, the harness's cached
   * account state is replaced with the returned post-state.
   */
  dispatch(ix: Instruction): ExecutionResult {
    this.requireOpen();
    const programIdBytes = pubkeyBytes(ix.programId);
    // Build the FfiAccountMeta array — koffi handles the struct
    // packing if we pass plain JS objects.
    const metas = ix.accounts.map((m) => ({
      pubkey_ptr: pubkeyBytes(m.pubkey),
      is_signer: m.isSigner,
      is_writable: m.isWritable,
    }));
    const dataBuf = ix.data.length > 0 ? Buffer.from(ix.data) : null;
    const handle = ffi.hopper_svm_dispatch(
      this.handle!,
      programIdBytes,
      metas,
      metas.length,
      dataBuf,
      ix.data.length,
    ) as Buffer;
    if (!handle) {
      throw new Error("hopper-svm: dispatch returned null");
    }
    return new ExecutionResult(handle);
  }

  /** Free the underlying harness. Idempotent. */
  dispose(): void {
    if (this.handle !== null) {
      ffi.hopper_svm_free(this.handle);
      this.handle = null;
    }
  }

  private requireOpen(): void {
    if (this.handle === null) {
      throw new Error("hopper-svm: handle disposed");
    }
  }
}

/**
 * Captured result of a dispatched instruction. Free via
 * `dispose()` when finished — TypeScript GC won't reclaim the
 * underlying Rust allocation automatically.
 */
export class ExecutionResult {
  private handle: Buffer | null;

  /** @internal */
  constructor(handle: Buffer) {
    this.handle = handle;
  }

  /** True if the dispatch produced an error. */
  isError(): boolean {
    this.requireOpen();
    return ffi.hopper_svm_outcome_is_error(this.handle!);
  }

  /** True if the dispatch succeeded. */
  isSuccess(): boolean {
    return !this.isError();
  }

  /** Captured error message, or empty string on success. */
  errorMessage(): string {
    this.requireOpen();
    const s = ffi.hopper_svm_outcome_error_message(this.handle!) as {
      ptr: Buffer | null;
      len: number;
    };
    return readFfiString(s);
  }

  /** CUs consumed by the dispatch. */
  consumedUnits(): bigint {
    this.requireOpen();
    return ffi.hopper_svm_outcome_consumed_units(this.handle!);
  }

  /** Transaction fee paid, if dispatched as a full transaction. */
  transactionFeePaid(): bigint {
    this.requireOpen();
    return ffi.hopper_svm_outcome_transaction_fee_paid(this.handle!);
  }

  /** Captured logs, in emission order. */
  logs(): string[] {
    this.requireOpen();
    const n = Number(ffi.hopper_svm_outcome_log_count(this.handle!));
    const out: string[] = [];
    for (let i = 0; i < n; i++) {
      const s = ffi.hopper_svm_outcome_log_at(this.handle!, i) as {
        ptr: Buffer | null;
        len: number;
      };
      out.push(readFfiString(s));
    }
    return out;
  }

  /** Post-dispatch accounts. */
  accounts(): KeyedAccount[] {
    this.requireOpen();
    const n = Number(ffi.hopper_svm_outcome_account_count(this.handle!));
    const out: KeyedAccount[] = [];
    for (let i = 0; i < n; i++) {
      const a = ffi.hopper_svm_outcome_account_at(this.handle!, i) as {
        address: Uint8Array;
        owner: Uint8Array;
        lamports: bigint;
        data_ptr: Buffer | null;
        data_len: number;
        executable: boolean;
        rent_epoch: bigint;
      };
      const data =
        a.data_len > 0 && a.data_ptr
          ? new Uint8Array(koffi.decode(a.data_ptr, "uint8_t", a.data_len) as Buffer)
          : new Uint8Array(0);
      out.push({
        address: new Pubkey(new Uint8Array(a.address)),
        owner: new Pubkey(new Uint8Array(a.owner)),
        lamports: a.lamports,
        data,
        executable: a.executable,
        rentEpoch: a.rent_epoch,
      });
    }
    return out;
  }

  /** Read the program return data, if any. */
  returnData(): Uint8Array {
    this.requireOpen();
    // First call: probe the length.
    const probeLen = Number(
      ffi.hopper_svm_outcome_return_data(this.handle!, null, 0),
    );
    if (probeLen === 0) return new Uint8Array(0);
    const buf = Buffer.alloc(probeLen);
    ffi.hopper_svm_outcome_return_data(this.handle!, buf, probeLen);
    return new Uint8Array(buf);
  }

  /** Free the underlying outcome. Idempotent. */
  dispose(): void {
    if (this.handle !== null) {
      ffi.hopper_svm_outcome_free(this.handle);
      this.handle = null;
    }
  }

  private requireOpen(): void {
    if (this.handle === null) {
      throw new Error("hopper-svm: outcome disposed");
    }
  }
}

// ------------------------------------------------------------------
// Pubkey-byte coercion
// ------------------------------------------------------------------

function pubkeyBytes(pk: Pubkey | Uint8Array): Uint8Array {
  return ensure32Bytes("pubkey", pk instanceof Pubkey ? pk.bytes : pk);
}

// ------------------------------------------------------------------
// Base58 — minimal hand-written encoder/decoder so we don't pull
// `bs58` or another dep just for pubkey display.
// ------------------------------------------------------------------

const B58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

function base58Encode(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  let leading = 0;
  while (leading < bytes.length && bytes[leading] === 0) leading++;
  // Convert byte array (big-endian) to base58.
  const digits: number[] = [0];
  for (let i = leading; i < bytes.length; i++) {
    let carry = bytes[i];
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j] << 8;
      digits[j] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  let out = "";
  for (let i = 0; i < leading; i++) out += "1";
  for (let i = digits.length - 1; i >= 0; i--) out += B58_ALPHABET[digits[i]];
  return out;
}

function base58Decode(str: string): Uint8Array {
  if (str.length === 0) return new Uint8Array(0);
  let leading = 0;
  while (leading < str.length && str[leading] === "1") leading++;
  const bytes: number[] = [0];
  for (let i = leading; i < str.length; i++) {
    const v = B58_ALPHABET.indexOf(str[i]);
    if (v < 0) throw new Error(`base58Decode: invalid character ${str[i]}`);
    let carry = v;
    for (let j = 0; j < bytes.length; j++) {
      carry += bytes[j] * 58;
      bytes[j] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  const out = new Uint8Array(leading + bytes.length);
  for (let i = bytes.length - 1, j = leading; i >= 0; i--, j++) out[j] = bytes[i];
  return out;
}

// ------------------------------------------------------------------
// Version
// ------------------------------------------------------------------

/** FFI library version (semver string). */
export function ffiVersion(): string {
  const ptr = ffi.hopper_svm_ffi_version() as Buffer;
  // Null-terminated string.
  return koffi.decode(ptr, "char", -1) as string;
}
