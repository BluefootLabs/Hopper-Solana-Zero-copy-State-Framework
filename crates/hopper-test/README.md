# hopper-test

Reusable in-process SVM harness for testing Hopper programs without a
validator. Workspace-internal (`publish = false`).

Wraps [`mollusk_svm`](https://crates.io/crates/mollusk-svm) so example and
integration tests can load a compiled Hopper `.so`, seed program-owned
accounts with a valid Hopper header (discriminator, version, layout id,
schema epoch), fire instructions, and read back lamports, account data, and
measured compute-unit cost.

This is the harness behind the compiled-SBF end-to-end suites — e.g.
[`examples/hopper-sentinel`](../../examples/hopper-sentinel)'s refusal
proofs and [`examples/hopper-smoke`](../../examples/hopper-smoke)'s
feature matrix — where the CU figures it reports have matched the deployed
devnet transactions exactly. For host-level tests that drive generated
dispatchers against live `AccountView` memory without an SBF VM, see
`hopper-svm` instead; the two harnesses are complementary tiers of the
same fidelity ladder (host bridge → compiled SBF → devnet).
