# hopper-multisig

M-of-N signer threshold checks for Hopper. Duplicate-signer prevention,
zero heap allocation, fixed stack footprint.

[![Crates.io](https://img.shields.io/crates/v/hopper-multisig.svg)](https://crates.io/crates/hopper-multisig)
[![Docs.rs](https://img.shields.io/docsrs/hopper-multisig)](https://docs.rs/hopper-multisig)

Part of the **[Hopper](https://hopperzero.dev)** framework.

Walks the program's `AccountInfo` slice, matches each entry against the
configured signer set, and rejects any duplicate keys before counting
toward the threshold. The check is constant-stack and bounded by the
declared signer-set size, not the account-list size.

```rust
use hopper_multisig::verify_threshold;

verify_threshold(accounts, &signer_set, threshold)?;
```

License: Apache-2.0.
