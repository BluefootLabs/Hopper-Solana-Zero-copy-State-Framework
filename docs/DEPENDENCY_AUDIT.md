# Dependency audit

This file records the dependency freshness decisions that should be easy to re-check before a release.

## Advisory policy

Run this from the workspace root before any public release:

```powershell
cargo audit --json --no-fetch
```

Every advisory must be classified before release:

- **SBF/on-chain direct**: fix, remove, or put behind an explicit non-default feature before release.
- **Host tooling**: may ship only with documented reachability, mitigation, and an owner for the upstream update.
- **Dev/test only**: may ship only when it is not part of the published on-chain framework path and the affected test lane is documented.

Do not add a RustSec ignore without a row in this file. The row must name the RustSec ID, dependency path, target lane, reachability, and retirement condition.

## Current decisions

- Solana host crates in `hopper-cli` resolve to Agave `2.3.13` in `Cargo.lock`, matching the local `solana-cli 2.3.13` toolchain used for devnet validation.
- `pinocchio` is kept on the current `0.11` line with `pinocchio-system 0.6` and `pinocchio-token 0.6`.
- `five8_const` stays on `0.1` because `solana-pubkey 2.4.0` requires `^0.1.3`; `five8_const 1.0.0` is not compatible with the current Solana crate line.
- `ureq 2`, `object 0.36`, and `gimli 0.31` remain pinned because they are host tooling dependencies and changing them does not improve on-chain safety or SBF output.

## Current RustSec ledger

Last checked: 2026-05-25 with `cargo audit --json --no-fetch`.

The current audit gate exits non-zero with 5 vulnerability advisories, 5 unmaintained advisories, and 1 unsound advisory. None of the listed advisories are direct dependencies of `hopper-runtime`, `hopper-systems`, or the default SBF authoring path. They are still release-tracked because host tooling, devnet runners, and Solana SDK compatibility crates are part of the public repository.

| Advisory | Package | Lane | Current dependency path | Reachability and mitigation | Retirement condition |
| --- | --- | --- | --- | --- | --- |
| `RUSTSEC-2024-0344` | `curve25519-dalek 3.2.0` | Host signing / Solana SDK | `ed25519-dalek 1.0.1` -> `solana-keypair` / `solana-signature` -> `hopper-cli`, devnet runner, Solana client stack | Not in Hopper's on-chain runtime path. Hopper does not implement custom scalar arithmetic or expose this as an on-chain signing oracle; affected use is through Solana host signing/client crates. Keep release notes honest that this is Solana SDK advisory debt, not a Hopper zero-copy runtime dependency. | Solana/Agave host crates move to patched `ed25519-dalek` / `curve25519-dalek`, or Hopper isolates CLI/devnet signing behind a separately audited tool crate. |
| `RUSTSEC-2022-0093` | `ed25519-dalek 1.0.1` | Host signing / Solana SDK | `solana-keypair` / `solana-signature` -> `hopper-cli`, devnet runner, Solana client stack | Not in the SBF runtime. Hopper uses Solana SDK keypair/signature APIs for host CLI/devnet workflows and must not expose APIs that accept attacker-controlled public keys for signing. | Solana/Agave host crates migrate to `ed25519-dalek >= 2`, or Hopper removes the affected host signing path. |
| `RUSTSEC-2026-0098`, `RUSTSEC-2026-0099`, `RUSTSEC-2026-0104` | `rustls-webpki 0.101.7` | Host RPC/TLS | `rustls 0.21` / `tungstenite` / `tokio-tungstenite` -> `solana-pubsub-client` -> `solana-client` -> `hopper-cli`, devnet runner | Host-only RPC/TLS dependency. No on-chain reachability and Hopper does not call CRL parsing APIs directly. The risk is confined to CLI/devnet network clients until Solana's client stack updates its TLS graph. | Solana client stack upgrades to patched `rustls-webpki >= 0.103.13` or Hopper removes the affected pubsub/TLS path. |
| `RUSTSEC-2025-0141` | `bincode 1.3.3` | Host CLI / Solana SDK / dev-test | Direct in `hopper-cli` for transaction decode plus Solana/Mollusk transitive paths | Unmaintained, not a known memory-safety vulnerability. Hopper CLI uses it only for explicit user-supplied transaction bytes in `tx simulate` / `tx submit`; on-chain Hopper code does not deserialize with `bincode`. | Replace direct CLI use with the Solana SDK's supported transaction codec or another maintained format, and inherit Solana SDK migration when available. |
| `RUSTSEC-2024-0388` | `derivative 2.2.0` | Build-time Solana ZK/Ark graph | `ark-*` / `light-poseidon` / `solana-poseidon` | Build-time proc-macro dependency through Solana ZK proof support. No direct Hopper runtime API depends on `derivative`. | Solana/Ark graph removes `derivative` or moves to a maintained derive helper. |
| `RUSTSEC-2025-0161` | `libsecp256k1 0.6.0` | Solana compatibility / dev-test SVM | `solana-secp256k1-recover` / `solana-program` and `agave-syscalls` / `mollusk-svm` | Hopper does not implement custom secp256k1 verification. The dependency is inherited from Solana compatibility and Mollusk test lanes. | Solana/Agave replaces `libsecp256k1` or Hopper gates the affected compatibility/dev-test lane separately. |
| `RUSTSEC-2025-0119` | `number_prefix 0.4.0` | Host CLI progress output | `indicatif` -> `solana-client` -> `hopper-cli`, devnet runner | Unmaintained formatting helper used through host progress/client tooling only. No SBF reachability. | Solana client stack or `indicatif` removes `number_prefix`. |
| `RUSTSEC-2024-0436` | `paste 1.0.15` | Build-time Solana ZK/Ark graph | `ark-*` / `light-poseidon` / `solana-bn254` / `solana-poseidon` | Build-time proc-macro dependency inherited through Solana ZK proof support. Hopper's own macro crates do not depend on `paste`. | Upstream Solana/Ark graph migrates to `pastey`, `with_builtin_macros`, or no paste-style helper. |
| `RUSTSEC-2026-0097` | `rand 0.7.3` | Host signing / Solana SDK / dev-test | `ed25519-dalek`, `libsecp256k1`, `solana-keypair` | Unsound only under the documented custom-logger + `thread_rng`/`rng` reentry conditions. Hopper does not install a custom logger that calls `rand::thread_rng`/`rand::rng`. Keep that as a CLI logging invariant. | Solana/Agave host crates leave `rand 0.7`, or Hopper removes the affected host signing/dev-test paths. |

## Re-check commands

```powershell
cargo audit --json --no-fetch
cargo tree --workspace -i curve25519-dalek@3.2.0 --locked --depth 5
cargo tree --workspace -i rustls-webpki@0.101.7 --locked --depth 4
cargo tree --workspace -i bincode@1.3.3 --locked --depth 4
cargo tree --workspace -i libsecp256k1@0.6.0 --locked --depth 4
cargo tree -p hopper-cli --depth 1
cargo tree -p hopper-runtime --depth 1
cargo search solana-client --limit 3
cargo search pinocchio --limit 3
cargo search pinocchio-system --limit 3
cargo search pinocchio-token --limit 3
cargo search five8_const --limit 3
```
