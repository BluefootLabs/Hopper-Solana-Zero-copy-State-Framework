# @hopper/svm

TypeScript bindings for `hopper-svm` — Hopper's in-process Solana
execution harness. Use it to test Solana programs (Hopper, Anchor,
or hand-rolled native) from Node without spinning up a validator.

## Setup

1. Build the FFI library from the workspace root:

   ```
   cargo build --release -p hopper-svm-ffi --features bpf-execution
   ```

   The artifact lands at `target/release/libhopper_svm_ffi.{so,dylib,dll}`.

2. Point the binding at it via the `HOPPER_SVM_LIB_PATH` env
   var, or copy the library next to your `node_modules` and let
   the platform default loader find it:

   ```bash
   export HOPPER_SVM_LIB_PATH=/path/to/target/release/libhopper_svm_ffi.so
   ```

3. Install the package:

   ```
   npm install @hopper/svm
   ```

## Quick start

```ts
import { HopperSvm, Pubkey } from "@hopper/svm";

const svm = HopperSvm.withSolanaRuntime();
try {
  // Seed a payer account.
  const payer = Pubkey.unique();
  svm.setAccount({
    address: payer,
    lamports: 10_000_000_000n,
    owner: new Pubkey(new Uint8Array(32)), // System program
  });

  // Dispatch a transfer.
  const recipient = Pubkey.unique();
  const ix = {
    programId: new Pubkey(new Uint8Array(32)), // System program ID
    accounts: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: recipient, isSigner: false, isWritable: true },
    ],
    data: new Uint8Array([2, 0, 0, 0, /* lamports LE */]),
  };
  const result = svm.dispatch(ix);
  try {
    console.log("Logs:", result.logs());
    console.log("Consumed:", result.consumedUnits());
  } finally {
    result.dispose();
  }
} finally {
  svm.dispose();
}
```

Each `HopperSvm` and `ExecutionResult` owns a Rust handle; call
`dispose()` (or use `try / finally` blocks as above) to release
the underlying allocation. Forgetting will leak memory inside the
Node runtime.

## Why this shape

The TS surface is intentionally synchronous — the harness runs
in-process so there's no I/O to await. Mirrors the Rust crate's
shape directly so test code reads consistently across language
boundaries.
