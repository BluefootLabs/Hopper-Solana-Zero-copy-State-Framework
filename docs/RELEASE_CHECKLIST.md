# Hopper Release Checklist

This checklist is the source of truth for what is published to crates.io and
what must stay private during a release cut.

## Non-Public Workspace Packages

The following package manifests are intentionally non-public and must keep
`publish = false` under `[package]`:

- `examples/hopper-vault/Cargo.toml` (`hopper-vault`)
- `examples/hopper-parity-vault/Cargo.toml` (`hopper-parity-vault`)
- `examples/hopper-escrow/Cargo.toml` (`hopper-escrow`)
- `examples/hopper-showcase/Cargo.toml` (`hopper-showcase`)
- `examples/hopper-treasury/Cargo.toml` (`hopper-treasury`)
- `examples/hopper-registry/Cargo.toml` (`hopper-registry`)
- `examples/hopper-migration/Cargo.toml` (`hopper-migration`)
- `examples/hopper-virtual-state/Cargo.toml` (`hopper-virtual-state`)
- `examples/hopper-token-2022-ata/Cargo.toml` (`hopper-token-2022-ata`)
- `examples/hopper-token-2022-vault/Cargo.toml` (`hopper-token-2022-vault`)
- `examples/hopper-token-2022-transfer-hook/Cargo.toml` (`hopper-token-2022-transfer-hook`)
- `examples/hopper-nft-mint/Cargo.toml` (`hopper-nft-mint`)
- `examples/hopper-proc-vault/Cargo.toml` (`hopper-proc-vault`)
- `examples/hopper-policy-vault/Cargo.toml` (`hopper-policy-vault`)
- `examples/quasar-port-20-min/Cargo.toml` (`hopper-quasar-port-20-min`)
- `examples/cross-program-read/program-a/Cargo.toml` (`hopper-xp-program-a`)
- `examples/cross-program-read/program-b/Cargo.toml` (`hopper-xp-program-b`)
- `tests/hopper-trybuild/Cargo.toml` (`hopper-trybuild`)
- `fuzz/Cargo.toml` (`hopper-fuzz`)

Do not publish example crates such as `hopper-vault`, `hopper-parity-vault`, or
`hopper-policy-vault` unless the release explicitly promotes them as public
starter templates and their manifests receive complete crates.io metadata first.

## Warning-free public lane

Before a launch or benchmark announcement, run the warning gate locally and in CI:

```sh
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo check -p hopper-quasar-port-20-min
cargo test -p hopper-quasar-port-20-min
cargo test -p hopper-framework --features proc-macros,metaplex --test metaplex_context_integration
powershell -ExecutionPolicy Bypass -File .\scripts\kani-runtime-tail.ps1
hopper lint
hopper profile elf target/deploy/<program>.so --html target/hopper-profile.html
```

On Unix-like CI runners, use `sh ./scripts/kani-runtime-tail.sh` for the same
proof lane. If PowerShell 7 is available, `pwsh ./scripts/kani-runtime-tail.ps1`
works too. On native Windows, the PowerShell runner delegates to WSL when
`cargo-kani` is not installed locally, because Kani's Cargo package currently
targets Unix-like hosts. The runtime proof lane covers bounded dynamic tails and
segment-borrow conflict invariants; the scripts fail fast when neither native
Kani nor WSL Kani is available.

Examples and CLI code should either compile warning-free or carry narrow, documented `allow(...)` attributes. Avoid broad crate-level allows on launch-facing examples; they make the release surface look unfinished.

## Published Package Metadata Gate

Every package intended for crates.io must have complete crates.io metadata in
`[package]` before publishing:

- `description`
- `license`
- `repository`
- `homepage`
- `documentation`
- `readme`
- `keywords`
- `categories` when a crates.io category applies

## crates.io Publication Order

Publish the public Hopper crates in this dependency order. Run
`cargo publish --dry-run -p <package>` before each real publish and wait for
crates.io indexing before publishing the next dependent crate.

1. `hopper-native`
2. `hopper-runtime`
3. `hopper-core`
4. `hopper-system`
5. `hopper-token`
6. `hopper-memo`
7. `hopper-metaplex`
8. `hopper-schema`
9. `hopper-macros-proc`
10. `hopper-macros`
11. `hopper-solana`
12. `hopper-associated-token`
13. `hopper-token-2022`
14. `hopper-finance`
15. `hopper-lending`
16. `hopper-staking`
17. `hopper-vesting`
18. `hopper-distribute`
19. `hopper-multisig`
20. `hopper-anchor`
21. `hopper-manager`
22. `hopper-sdk`
23. `hopper-cli`
24. `hopper-framework` (library crate name `hopper`)

`hopper-cli` is intentionally published after `hopper-manager` and
`hopper-schema` have indexed because it depends on both. The top-level
framework package publishes as `hopper-framework` because the crates.io
`hopper` package name is occupied by an unrelated crate; consumers should alias
it back to `hopper` in `Cargo.toml` with `hopper = { package = "hopper-framework", ... }`.
