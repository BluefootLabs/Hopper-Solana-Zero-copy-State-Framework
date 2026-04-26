"""hopper-svm — Python bindings for the Hopper in-process Solana execution harness.

The :class:`HopperSvm` class wraps the ``hopper_svm_ffi`` shared
library via ``cffi``'s ABI mode (no compile step required). The
shared library is built from ``crates/hopper-svm-ffi`` in the
Hopper workspace::

    cargo build --release -p hopper-svm-ffi --features bpf-execution

By default the loader searches the standard system library path.
Override via the ``HOPPER_SVM_LIB_PATH`` environment variable to
point at a non-standard location (e.g. a ``target/release`` build
inside the workspace).

Usage
-----

::

    from hopper_svm import HopperSvm, Pubkey

    svm = HopperSvm.with_solana_runtime()
    try:
        payer = Pubkey.unique()
        system_program = Pubkey(bytes(32))
        svm.set_account(
            address=payer,
            lamports=10_000_000_000,
            owner=system_program,
        )
        # Dispatch instructions, inspect logs / consumed CUs / accounts.
    finally:
        svm.close()

Every :class:`HopperSvm` and :class:`ExecutionResult` owns a Rust
handle; release via ``close()`` (or use ``with`` blocks — both
classes are context managers) to avoid leaks in the Python
runtime.
"""

from .core import (
    HopperSvm,
    ExecutionResult,
    Pubkey,
    AccountMeta,
    Instruction,
    KeyedAccount,
    ffi_version,
)

__all__ = [
    "HopperSvm",
    "ExecutionResult",
    "Pubkey",
    "AccountMeta",
    "Instruction",
    "KeyedAccount",
    "ffi_version",
]
__version__ = "0.1.0"
