# Dependency audit

This file records the dependency freshness decisions that should be easy to re-check before a release.

## Current decisions

- Solana host crates in `hopper-cli` resolve to Agave `2.3.13` in `Cargo.lock`, matching the local `solana-cli 2.3.13` toolchain used for devnet validation.
- `pinocchio` is kept on the current `0.11` line with `pinocchio-system 0.6` and `pinocchio-token 0.6`.
- `five8_const` stays on `0.1` because `solana-pubkey 2.4.0` requires `^0.1.3`; `five8_const 1.0.0` is not compatible with the current Solana crate line.
- `ureq 2`, `object 0.36`, and `gimli 0.31` remain pinned because they are host tooling dependencies and changing them does not improve on-chain safety or SBF output.

## Re-check commands

```powershell
cargo tree -p hopper-cli --depth 1
cargo tree -p hopper-runtime --depth 1
cargo search solana-client --limit 3
cargo search pinocchio --limit 3
cargo search pinocchio-system --limit 3
cargo search pinocchio-token --limit 3
cargo search five8_const --limit 3
```
