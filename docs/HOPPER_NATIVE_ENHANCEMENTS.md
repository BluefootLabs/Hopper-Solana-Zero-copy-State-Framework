# Hopper native execution

`hopper-native` owns loader parsing, duplicate-account resolution, eager and lazy
entrypoints, account views, syscalls, PDA helpers, and native CPI. The runtime
layer adds the account contracts and guarded operations used by authored programs.

The normal path is `no_std` with no heap allocation. Advanced applications can
choose entrypoint account ceilings, explicit raw APIs, or cluster-gated features.
The default scanning entrypoint remains usable without optional account-table
or static-syscall assumptions.

Checked CPI validates identities, privileges, and borrow compatibility before
invocation. Signed CPI provides authority only for PDAs derived by the caller's
program. Returning from CPI does not remove the application's responsibility to
validate balances and downstream behavior when its contract requires that.

Read [program architecture](ARCHITECTURE.md) for layer responsibilities and
[unsafe invariants](UNSAFE_INVARIANTS.md) before using raw pointers or unchecked
invocation. Performance work belongs in reproducible complete-program fixtures.

## Native/runtime 0.4.1 patch — September 26, 2026

`hopper-native` and `hopper-runtime` **0.4.1 are published**. The framework and
CLI remain 0.4.0; support packages keep their independent versions. Existing
lockfiles must update the native/runtime dependencies to pick up the patch:

```sh
cargo update -p hopper-native -p hopper-runtime
```

The patch rejects missing signers in specialized checked CPI, uses immediate
SVM aborts for no-allocation failures and panics, and validates the producer
and typed prefix of CPI return data. A nested callee's unforwarded return data
is rejected. Application-level value and outcome checks are still required.

[Validation evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-multisig-2026-09-26).
