"""Core Python bindings for hopper-svm.

This module wraps the ``hopper_svm_ffi`` shared library (built
from ``crates/hopper-svm-ffi``) through ``cffi``'s ABI mode. The
public API (:class:`HopperSvm`, :class:`ExecutionResult`,
:class:`Pubkey`) mirrors the Rust crate one-shot — same builders,
same accessors, same naming convention shifted to ``snake_case``.

Implementation notes
--------------------

- ``cffi`` ABI mode loads the shared library at import time via
  ``ffi.dlopen``. The C header is declared inline below, matching
  the ``extern "C"`` exports in ``crates/hopper-svm-ffi/src/lib.rs``
  exactly. Adding a new export to the FFI crate requires updating
  *both* the Rust source and the inline ``cdef`` block here.
- Strings returned by the FFI (logs, error messages) live behind
  ``HopperFfiString`` structs that own the byte allocation. We
  copy the bytes into Python ``str`` immediately and call
  ``hopper_svm_string_free`` to release the buffer — no Python-
  side reference to the FFI string survives.
- ``HopperSvm`` and ``ExecutionResult`` register cleanup via
  ``__del__`` AND expose explicit ``close()`` for deterministic
  release. Test code should prefer ``close()`` (or ``with``
  blocks — both classes are context managers) since Python's GC
  doesn't fire promptly enough to keep memory bounded under
  large test suites.
"""

from __future__ import annotations

import os
import platform
import secrets
from dataclasses import dataclass
from typing import Iterable, List, Optional, Sequence, Union

from cffi import FFI

# ----------------------------------------------------------------------
# CFFI definitions — must match crates/hopper-svm-ffi/src/lib.rs.
# ----------------------------------------------------------------------

ffi = FFI()
ffi.cdef(
    """
    typedef struct {
        uint8_t *ptr;
        size_t len;
    } HopperFfiString;

    typedef struct {
        const uint8_t *pubkey_ptr;
        bool is_signer;
        bool is_writable;
    } FfiAccountMeta;

    typedef struct {
        uint8_t address[32];
        uint8_t owner[32];
        uint64_t lamports;
        const uint8_t *data_ptr;
        size_t data_len;
        bool executable;
        uint64_t rent_epoch;
    } FfiAccount;

    void *hopper_svm_new(void);
    void hopper_svm_with_solana_runtime(void *handle);
    void hopper_svm_set_compute_budget(void *handle, uint64_t units);
    void hopper_svm_free(void *handle);

    bool hopper_svm_set_account(
        void *handle,
        const uint8_t *address_ptr,
        uint64_t lamports,
        const uint8_t *owner_ptr,
        const uint8_t *data_ptr,
        size_t data_len,
        bool executable);
    uint64_t hopper_svm_get_lamports(void *handle, const uint8_t *address_ptr);
    size_t hopper_svm_get_account_data(
        void *handle,
        const uint8_t *address_ptr,
        uint8_t *out_ptr,
        size_t out_len);

    void *hopper_svm_dispatch(
        void *handle,
        const uint8_t *program_id_ptr,
        const FfiAccountMeta *accounts_ptr,
        size_t accounts_len,
        const uint8_t *data_ptr,
        size_t data_len);

    bool hopper_svm_outcome_is_error(void *handle);
    HopperFfiString hopper_svm_outcome_error_message(void *handle);
    uint64_t hopper_svm_outcome_consumed_units(void *handle);
    uint64_t hopper_svm_outcome_transaction_fee_paid(void *handle);
    size_t hopper_svm_outcome_log_count(void *handle);
    HopperFfiString hopper_svm_outcome_log_at(void *handle, size_t index);
    size_t hopper_svm_outcome_account_count(void *handle);
    FfiAccount hopper_svm_outcome_account_at(void *handle, size_t index);
    size_t hopper_svm_outcome_return_data(void *handle, uint8_t *out_ptr, size_t out_len);
    void hopper_svm_outcome_free(void *handle);

    void hopper_svm_string_free(HopperFfiString s);
    const uint8_t *hopper_svm_ffi_version(void);
    """
)


def _default_lib_name() -> str:
    system = platform.system()
    if system == "Linux":
        return "libhopper_svm_ffi.so"
    if system == "Darwin":
        return "libhopper_svm_ffi.dylib"
    if system == "Windows":
        return "hopper_svm_ffi.dll"
    raise RuntimeError(f"hopper-svm: unsupported platform {system}")


def _load_library() -> "object":
    path = os.environ.get("HOPPER_SVM_LIB_PATH", _default_lib_name())
    return ffi.dlopen(path)


_lib = _load_library()


# ----------------------------------------------------------------------
# String marshalling
# ----------------------------------------------------------------------


def _read_ffi_string(s: "object") -> str:
    """Copy bytes out of a `HopperFfiString` and free the underlying buffer."""
    if s.len == 0 or s.ptr == ffi.NULL:
        return ""
    raw = ffi.buffer(s.ptr, s.len)[:]
    text = raw.decode("utf-8", errors="replace")
    _lib.hopper_svm_string_free(s)
    return text


# ----------------------------------------------------------------------
# Pubkey
# ----------------------------------------------------------------------


_BASE58_ALPHABET = (
    "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
)


def _base58_encode(b: bytes) -> str:
    """Tiny base58 encoder — avoids pulling `base58` as a dep."""
    if not b:
        return ""
    n = int.from_bytes(b, "big")
    out = ""
    while n > 0:
        n, rem = divmod(n, 58)
        out = _BASE58_ALPHABET[rem] + out
    # Preserve leading zeros as '1' chars.
    leading = 0
    for byte in b:
        if byte == 0:
            leading += 1
        else:
            break
    return "1" * leading + out


def _base58_decode(s: str) -> bytes:
    if not s:
        return b""
    leading = 0
    for ch in s:
        if ch == "1":
            leading += 1
        else:
            break
    n = 0
    for ch in s:
        idx = _BASE58_ALPHABET.find(ch)
        if idx < 0:
            raise ValueError(f"base58_decode: invalid character {ch!r}")
        n = n * 58 + idx
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n > 0 else b""
    return b"\x00" * leading + body


class Pubkey:
    """32-byte Solana pubkey."""

    __slots__ = ("bytes",)

    def __init__(self, raw: bytes) -> None:
        if len(raw) != 32:
            raise ValueError(f"Pubkey: expected 32 bytes, got {len(raw)}")
        self.bytes = bytes(raw)

    @classmethod
    def unique(cls) -> "Pubkey":
        """Random 32 bytes — useful for test fixtures."""
        return cls(secrets.token_bytes(32))

    @classmethod
    def from_base58(cls, s: str) -> "Pubkey":
        decoded = _base58_decode(s)
        if len(decoded) != 32:
            raise ValueError(
                f"Pubkey.from_base58: decoded {len(decoded)} bytes, expected 32"
            )
        return cls(decoded)

    def to_base58(self) -> str:
        return _base58_encode(self.bytes)

    def __str__(self) -> str:
        return self.to_base58()

    def __repr__(self) -> str:
        return f"Pubkey({self.to_base58()!r})"

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Pubkey):
            return False
        return self.bytes == other.bytes

    def __hash__(self) -> int:
        return hash(self.bytes)


PubkeyLike = Union[Pubkey, bytes]


def _to_pubkey_bytes(pk: PubkeyLike) -> bytes:
    if isinstance(pk, Pubkey):
        return pk.bytes
    if isinstance(pk, (bytes, bytearray)):
        if len(pk) != 32:
            raise ValueError(f"pubkey: expected 32 bytes, got {len(pk)}")
        return bytes(pk)
    raise TypeError(f"pubkey: expected Pubkey or 32-byte bytes, got {type(pk).__name__}")


# ----------------------------------------------------------------------
# Account / instruction shapes
# ----------------------------------------------------------------------


@dataclass
class AccountMeta:
    pubkey: PubkeyLike
    is_signer: bool
    is_writable: bool


@dataclass
class Instruction:
    program_id: PubkeyLike
    accounts: Sequence[AccountMeta]
    data: bytes = b""


@dataclass
class KeyedAccount:
    address: Pubkey
    owner: Pubkey
    lamports: int
    data: bytes
    executable: bool
    rent_epoch: int


# ----------------------------------------------------------------------
# HopperSvm
# ----------------------------------------------------------------------


class HopperSvm:
    """In-process Hopper SVM instance — the harness for tests.

    Construct with :py:meth:`HopperSvm` for a bare configuration
    (only the system program registered) or
    :py:meth:`with_solana_runtime` for the full validator-side
    surface (System + Compute Budget + ALT + Config + Stake +
    Vote + SPL Token + Token-2022 + ATA).

    Use as a context manager so the underlying handle is freed
    when the ``with`` block exits.
    """

    def __init__(self) -> None:
        handle = _lib.hopper_svm_new()
        if handle == ffi.NULL:
            raise RuntimeError("hopper-svm: hopper_svm_new returned NULL")
        self._handle: Optional[object] = handle

    @classmethod
    def with_solana_runtime(cls) -> "HopperSvm":
        svm = cls()
        _lib.hopper_svm_with_solana_runtime(svm._handle)
        return svm

    def __enter__(self) -> "HopperSvm":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def __del__(self) -> None:
        self.close()

    def close(self) -> None:
        """Free the underlying harness. Idempotent."""
        if self._handle is not None and self._handle != ffi.NULL:
            _lib.hopper_svm_free(self._handle)
            self._handle = None

    def set_compute_budget(self, units: int) -> None:
        """Set the per-transaction compute-unit limit."""
        self._require_open()
        _lib.hopper_svm_set_compute_budget(self._handle, units)

    def set_account(
        self,
        address: PubkeyLike,
        lamports: int,
        owner: PubkeyLike,
        data: bytes = b"",
        executable: bool = False,
    ) -> None:
        """Seed an account into the harness's cached state."""
        self._require_open()
        addr_bytes = _to_pubkey_bytes(address)
        owner_bytes = _to_pubkey_bytes(owner)
        data_buf = ffi.new("uint8_t[]", data) if data else ffi.NULL
        ok = _lib.hopper_svm_set_account(
            self._handle,
            ffi.new("uint8_t[]", addr_bytes),
            lamports,
            ffi.new("uint8_t[]", owner_bytes),
            data_buf,
            len(data),
            executable,
        )
        if not ok:
            raise RuntimeError("hopper-svm: set_account failed")

    def get_lamports(self, address: PubkeyLike) -> Optional[int]:
        """Read the lamport balance for a cached account.

        Returns ``None`` if the account isn't seeded.
        """
        self._require_open()
        value = _lib.hopper_svm_get_lamports(
            self._handle,
            ffi.new("uint8_t[]", _to_pubkey_bytes(address)),
        )
        # u64::MAX is the FFI sentinel for "unknown account".
        if value == 0xFFFFFFFFFFFFFFFF:
            return None
        return int(value)

    def dispatch(self, ix: Instruction) -> "ExecutionResult":
        """Dispatch a single instruction.

        Returns an :class:`ExecutionResult`; remember to call its
        ``close()`` when done.
        """
        self._require_open()
        program_id_buf = ffi.new("uint8_t[]", _to_pubkey_bytes(ix.program_id))

        # Pubkey buffers must outlive the FfiAccountMeta array we
        # construct below — keep references in `keepalive` so the
        # cffi GC doesn't reclaim them mid-call.
        keepalive: List[object] = []
        metas = ffi.new("FfiAccountMeta[]", len(ix.accounts))
        for i, m in enumerate(ix.accounts):
            buf = ffi.new("uint8_t[]", _to_pubkey_bytes(m.pubkey))
            keepalive.append(buf)
            metas[i].pubkey_ptr = buf
            metas[i].is_signer = m.is_signer
            metas[i].is_writable = m.is_writable

        data_buf = ffi.new("uint8_t[]", ix.data) if ix.data else ffi.NULL
        outcome_handle = _lib.hopper_svm_dispatch(
            self._handle,
            program_id_buf,
            metas,
            len(ix.accounts),
            data_buf,
            len(ix.data),
        )
        if outcome_handle == ffi.NULL:
            raise RuntimeError("hopper-svm: dispatch returned NULL")
        return ExecutionResult(outcome_handle)

    def _require_open(self) -> None:
        if self._handle is None or self._handle == ffi.NULL:
            raise RuntimeError("hopper-svm: handle closed")


# ----------------------------------------------------------------------
# ExecutionResult
# ----------------------------------------------------------------------


class ExecutionResult:
    """Captured result of a dispatched instruction."""

    def __init__(self, handle: object) -> None:
        self._handle: Optional[object] = handle

    def __enter__(self) -> "ExecutionResult":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def __del__(self) -> None:
        self.close()

    def close(self) -> None:
        """Free the underlying outcome. Idempotent."""
        if self._handle is not None and self._handle != ffi.NULL:
            _lib.hopper_svm_outcome_free(self._handle)
            self._handle = None

    def is_error(self) -> bool:
        self._require_open()
        return bool(_lib.hopper_svm_outcome_is_error(self._handle))

    def is_success(self) -> bool:
        return not self.is_error()

    def error_message(self) -> str:
        self._require_open()
        s = _lib.hopper_svm_outcome_error_message(self._handle)
        return _read_ffi_string(s)

    def consumed_units(self) -> int:
        self._require_open()
        return int(_lib.hopper_svm_outcome_consumed_units(self._handle))

    def transaction_fee_paid(self) -> int:
        self._require_open()
        return int(_lib.hopper_svm_outcome_transaction_fee_paid(self._handle))

    def logs(self) -> List[str]:
        self._require_open()
        n = int(_lib.hopper_svm_outcome_log_count(self._handle))
        return [
            _read_ffi_string(_lib.hopper_svm_outcome_log_at(self._handle, i))
            for i in range(n)
        ]

    def accounts(self) -> List[KeyedAccount]:
        self._require_open()
        n = int(_lib.hopper_svm_outcome_account_count(self._handle))
        out: List[KeyedAccount] = []
        for i in range(n):
            a = _lib.hopper_svm_outcome_account_at(self._handle, i)
            data = bytes(ffi.buffer(a.data_ptr, a.data_len)[:]) if a.data_len > 0 else b""
            out.append(
                KeyedAccount(
                    address=Pubkey(bytes(a.address)),
                    owner=Pubkey(bytes(a.owner)),
                    lamports=int(a.lamports),
                    data=data,
                    executable=bool(a.executable),
                    rent_epoch=int(a.rent_epoch),
                )
            )
        return out

    def return_data(self) -> bytes:
        self._require_open()
        # Probe length first.
        probe_len = int(_lib.hopper_svm_outcome_return_data(self._handle, ffi.NULL, 0))
        if probe_len == 0:
            return b""
        buf = ffi.new("uint8_t[]", probe_len)
        _lib.hopper_svm_outcome_return_data(self._handle, buf, probe_len)
        return bytes(ffi.buffer(buf, probe_len)[:])

    def _require_open(self) -> None:
        if self._handle is None or self._handle == ffi.NULL:
            raise RuntimeError("hopper-svm: outcome closed")


# ----------------------------------------------------------------------
# Module-level helpers
# ----------------------------------------------------------------------


def ffi_version() -> str:
    """Version string of the loaded `hopper_svm_ffi` shared library."""
    ptr = _lib.hopper_svm_ffi_version()
    return ffi.string(ffi.cast("char *", ptr)).decode("utf-8")
