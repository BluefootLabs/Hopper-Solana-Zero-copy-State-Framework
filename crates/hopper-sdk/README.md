# hopper-sdk

Off-chain companion crate for [Hopper](https://hopperzero.dev). Indexers,
explorers, wallets, and back-ends use this to consume Hopper programs without
running on-chain.

[![Crates.io](https://img.shields.io/crates/v/hopper-sdk.svg)](https://crates.io/crates/hopper-sdk)
[![Docs.rs](https://img.shields.io/docsrs/hopper-sdk)](https://docs.rs/hopper-sdk)

## What's here

- **Receipt decoder** — parse Hopper's 64-byte `StateReceipt` wire format
  into a structured value plus a human-readable narrative.
- **Reader** — segment-aware partial account readers that fetch only the
  fields you need from an account snapshot, with `LAYOUT_ID` fingerprint
  verification.
- **Fingerprint** — runtime layout-id verification helpers symmetric with
  Hopper's compile-time pinning.
- **Diff** — snapshot-to-snapshot field-level diff matching the on-chain
  diff engine.
- **Builder** (optional feature) — typed instruction + account builders
  derived from a `ProgramManifest`.

License: Apache-2.0.
