# hopper-sdk

Off-chain companion crate for [Hopper](https://hopperzero.dev). Indexers,
explorers, wallets, and backends use this to consume Hopper programs without
running on-chain.

## What's here

- **Receipt decoder** - parse Hopper's 72-byte `StateReceipt` wire format,
  with 64-byte legacy receipt support, into a structured value
  (`DecodedReceipt`) plus a human-readable narrative (`Narrator`, behind the
  default `narrate` feature).
- **Reader** - segment-aware partial account readers that fetch only the
  fields you need from an account snapshot, with `LAYOUT_ID` fingerprint
  verification.
- **Fingerprint** - runtime layout-id verification helpers symmetric with
  Hopper's compile-time pinning.
- **Diff** - snapshot-to-snapshot field-level diff matching the on-chain
  diff engine.
- **Builder** (the `builder` feature, on by default) - typed instruction and
  account builders derived from a `ProgramManifest`.

Default features are `std`, `narrate`, and `builder`. Build with
`default-features = false` to drop the narrative and builder modules.

Docs: <https://docs.rs/crate/hopper-sdk>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
