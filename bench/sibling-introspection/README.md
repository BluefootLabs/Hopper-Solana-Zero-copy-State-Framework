# Processed sibling instruction fixture

This no-allocation program makes real nested calls to itself and reads their
instruction data and account metadata. It checks the native and runtime APIs,
exact lengths, signer/writable flags, empty and missing instructions, reverse
sibling order, child exclusion, and a 1,300-byte CPI payload. Two cases deliberately
return `AccountDataTooSmall` for insufficient data or account buffers.

`sibling_introspection_sbf.rs` runs the compiled fixture in an SVM. The devnet
runner verifies finalized transactions, complete payer/program snapshots, and
the deployed ELF before and after. Host stubs cannot establish trace behavior.

```sh
cargo build-sbf --manifest-path bench/sibling-introspection/program/Cargo.toml
HOPPER_SIBLING_SBF=target/deploy/hopper_sibling_introspection_fixture.so \
  cargo test -p hopper-framework-verifier --test sibling_introspection_sbf -- --ignored --nocapture
```

The `baseline` feature excludes the new API so the same legacy-read cases can be
built in an isolated workspace with native 0.4.2 and runtime 0.4.3. The verifier's
`HOPPER_SIBLING_BASELINE=1` lane expects those versions to exhibit the four
regressions; it must not be used to validate a fixed release.
