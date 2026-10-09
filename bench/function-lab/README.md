# Function lab

A deployable, allocation-free program for checking Hopper functions inside
Solana's VM and on public devnet. It computes results from instruction bytes;
the runner compares them with independent expected values. It is a test fixture,
not an application or custody example.

The default matrix contains 106 known-answer and rejection cases:

- Native and runtime SHA-256 and Keccak-256 with 0, 1, 2, 16, 17, 32
  and 64 input slices, including empty input.
- Prefix copies, forward and backward overlapping moves, fill, zero, slice
  comparison, and invalid memory ranges.
- Checked and saturating wire arithmetic and native multiplication, including
  overflow, underflow, division by zero and 32-bit carry boundaries.
- Epoch arithmetic through warmup transitions, maximum slots and epochs,
  saturation and zero-length schedule handling.
- Empty, 1-byte, 1,023-byte and 1,024-byte return data; 1,025-byte refusal.
- Raw and tagged receipt emission, including empty and binary payloads. The
  devnet runner compares the exact base64 log segments.
- Edwards and Ristretto operations, a Poseidon known-answer vector, BN254
  operations and invalid lengths, and top-level stack introspection.

Account tests exercise unaligned Pod lenses, duplicate-account borrow tracking,
signer/writable/owner admission, unchanged-balance checks and data fingerprints.
The devnet runner also compares live Clock and Rent results with RPC values,
decodes the actual EpochSchedule account independently, and reports disagreement
between that account, Clock and the RPC schedule. It checks the complete fixture account after every state test and
verifies the deployed ELF before and after execution.

## Run the compiled VM checks

```sh
python scripts/function-lab-cases.py --out target/function-lab/cases.json
cargo build-sbf --manifest-path bench/function-lab/program/Cargo.toml --arch v3 --sbf-out-dir target/function-lab/sbf
HOPPER_FUNCTION_LAB_SBF=target/function-lab/sbf/hopper_function_lab.so \
HOPPER_FUNCTION_LAB_CASES=target/function-lab/cases.json \
cargo test -p hopper-framework-verifier --test function_lab_sbf -- --ignored --nocapture
```

Use absolute ELF and case paths if launching the verifier from another directory.
Build with `--arch v0` to exercise the older SBF architecture separately.

To compile every deployable workspace package, including newly added programs:

```sh
python scripts/build-programs-sbf.py --arch v0 --out target/programs-v0
python scripts/build-programs-sbf.py --arch v3 --out target/programs-v3
```

Each invocation requires a fresh output directory, uses a fresh Cargo target,
and records source hashes, build logs and exact ELF hashes. Missing artifacts,
stack-frame diagnostics and source changes fail the gate. These are compile
checks; run each program's semantic tests separately.

## Run on devnet

Record the source before building. Deploy the resulting ELF to a fresh devnet
program, then run:

```sh
python scripts/test-borrowed-args-devnet.py snapshot --out target/function-lab/source.json
# Build and deploy this exact source before the next command.
python scripts/test-function-lab-devnet.py --program PROGRAM_ID \
  --payer DEVNET_KEYPAIR --hopper PATH_TO_HOPPER --elf PATH_TO_ELF \
  --source-snapshot target/function-lab/source.json --out target/function-lab/live
```

The runner uses only the public devnet endpoint and checks its genesis hash.
Keys and raw execution output stay under ignored `target/`. It neither changes
global Solana configuration nor signs mainnet transactions.

`simd-0321`, `simd-0449`, `static-syscalls` and `sha512-syscall` select separate
build variants. `--sha512` adds 14 hash cases to the live runner and case
generator. Verify each cluster gate before building/deploying those variants.
The default fixture caps its hash slice table at 64 to fit bounded stack
storage; that is a fixture limit, not the wrapper's 20,000-slice ABI limit.

`big-mod-exp` is deliberately separate. On October 7, 2026 the SIMD-0529 feature
account was absent on devnet; the pinned Agave 4.2.1 VM returns a failure stub.
Its vectors are available with the case generator's `--big-mod-exp` option for
a runtime that actually implements and enables the syscall. They are not
included in default passing counts or claimed as verified on public devnet.

`blake3-syscall` adds BLAKE3 probes; pair it with the case generator or devnet
runner's `--blake3` flag. Its feature account was also absent on October 7 and
Solana CLI 4.3 refused the initial BLAKE3-linked ELF during feature verification.
A probe overriding only that local feature selection deployed, but both native
and runtime BLAKE3 calls then finalized with `ProgramFailedToComplete` and an
unsupported-instruction log. The local VM supports the 14 BLAKE3 vectors; the
default devnet fixture excludes that symbol. Deployment alone is not execution
evidence, and a feature-account observation is not a substitute for either.

On October 7, devnet's EpochSchedule account and both runtime getters reported
8,192 slots per epoch, while `getEpochSchedule` returned 432,000 and Clock's
epoch agreed with the latter. The runner verifies Hopper's decoding and
arithmetic against the actual account, completes the other function checks,
then exits unsuccessfully if this cluster inconsistency remains. A receipt can
therefore have `functionChecksPassed: true` and `allPassed: false`. Use Clock
for the current epoch; do not silently replace on-chain schedule values with
RPC constants. See the [network baseline](../../docs/SOLANA_NETWORK_BASELINE.md).

This matrix covers the functions listed above. Token flows, CPI, account
lifecycle, dynamic tails, PDA initialization and application handlers have
separate fixtures. A successful function-lab run does not certify every Hopper
feature or every application program.
