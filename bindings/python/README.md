# hopper-svm (Python)

Python bindings for `hopper-svm` — Hopper's in-process Solana
execution harness. Use it to test Solana programs (Hopper, Anchor,
or hand-rolled native) from Python without spinning up a validator.

## Setup

1. Build the FFI shared library from the workspace root:

   ```
   cargo build --release -p hopper-svm-ffi --features bpf-execution
   ```

   The artifact lands at
   `target/release/libhopper_svm_ffi.{so,dylib,dll}`.

2. Point Python at it via `HOPPER_SVM_LIB_PATH`, or install the
   library somewhere the platform's default loader will find it:

   ```bash
   export HOPPER_SVM_LIB_PATH=/path/to/target/release/libhopper_svm_ffi.so
   ```

3. Install the binding:

   ```
   pip install hopper-svm
   ```

## Quick start

```python
from hopper_svm import HopperSvm, Pubkey, Instruction, AccountMeta

with HopperSvm.with_solana_runtime() as svm:
    payer = Pubkey.unique()
    system_program = Pubkey(bytes(32))

    svm.set_account(
        address=payer,
        lamports=10_000_000_000,
        owner=system_program,
    )

    recipient = Pubkey.unique()
    transfer_amount = (1_000_000).to_bytes(8, "little")
    ix = Instruction(
        program_id=system_program,
        accounts=[
            AccountMeta(pubkey=payer, is_signer=True, is_writable=True),
            AccountMeta(pubkey=recipient, is_signer=False, is_writable=True),
        ],
        # System Transfer tag (2) + lamports LE
        data=b"\x02\x00\x00\x00" + transfer_amount,
    )

    with svm.dispatch(ix) as result:
        print("Logs:", result.logs())
        print("Consumed:", result.consumed_units())
        for acct in result.accounts():
            print(f"  {acct.address}: {acct.lamports}")
```

Both `HopperSvm` and `ExecutionResult` are context managers and
expose `close()` for explicit handle release. Garbage-collection
fallback exists (via `__del__`) but for high-throughput tests
prefer `with` / `close()` so handles are reclaimed deterministically.
