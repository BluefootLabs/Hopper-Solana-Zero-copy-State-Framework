# hopper-test

Reusable in-process SVM harness for testing Hopper programs without a
validator. Workspace-internal (`publish = false`).

Wraps [`mollusk_svm`](https://crates.io/crates/mollusk-svm) so example and
integration tests can load a compiled Hopper `.so`, seed program-owned
accounts with a valid Hopper header (discriminator, version, layout id,
schema epoch), fire instructions, and read back lamports, account data, and
measured compute-unit cost.

This is the harness behind the compiled-SBF end-to-end suites (e.g.
[`examples/hopper-sentinel`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-sentinel)'s refusal
proofs and [`examples/hopper-smoke`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-smoke)'s
feature matrix) where the CU figures it reports have matched the deployed
devnet transactions exactly. For host-level tests that drive generated
dispatchers against live `AccountView` memory without an SBF VM, see
`hopper-svm` instead; the two harnesses are complementary tiers of the
same fidelity ladder (host bridge → compiled SBF → devnet).

## Frames and fixtures

Call `capture_logs()` before `process`, then `frames()` for the CPI frame
tree the runtime logged: each `Frame` carries the program, its depth, the
compute units it consumed and the units it spent itself (consumed minus its
callees), success or the failure text, its own `Program log:` lines, and its
children. `Frame::invocations_of(program)` counts how often a program ran
in the subtree, which is how the token lab asserts that a `Batch` of two
transfers is one token-program invocation. `hopper_test::fixtures` builds
base mints, token accounts, funded wallets, and associated token addresses
for SPL Token or Token-2022 without depending on the token crates; add the
program itself through `mollusk_svm_programs_token` when it must execute.
