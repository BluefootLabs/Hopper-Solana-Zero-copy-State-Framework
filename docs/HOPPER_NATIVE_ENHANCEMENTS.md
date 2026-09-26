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
