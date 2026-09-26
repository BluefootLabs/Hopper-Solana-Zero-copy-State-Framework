# Dependency audit

This file records the dependency freshness decisions that should be easy to re-check before a release.

## Advisory policy

Run this from the workspace root before any public release. The installed
`cargo-audit` version does not accept Cargo's `--locked` flag; it scans
`Cargo.lock` by default:

```powershell
cargo audit --json --no-fetch
```

Every advisory must be classified before release:

- **SBF/on-chain direct**: fix, remove, or put behind an explicit non-default feature before release.
- **Host tooling**: may ship only with documented reachability, mitigation, and an owner for the upstream update.
- **Dev/test only**: may ship only when it is not part of the published on-chain framework path and the affected test lane is documented.

Do not add a RustSec ignore without a row in this file. The row must name the RustSec ID, dependency path, target lane, reachability, and retirement condition.

## Current decisions

- Solana host RPC/signing crates resolve to Agave `4.2.1`. Direct SDK types are
  pinned to the coherent versions selected by that release instead of loose
  same-major ranges: `solana-instruction 3.4.0`, `solana-pubkey 4.2.1`,
  `solana-transaction 4.1.4`, and `solana-system-interface 3.2.0`.
- The in-process validator lane uses `mollusk-svm 0.15.0` and Agave `4.2.1`.
  Keeping the client and validator graphs aligned prevents Cargo from selecting
  incompatible same-major Solana SDK leaf crates.
- `five8`, `five8_const`, and `five8_core` resolve together at `1.0.0`. This
  avoids the invalid `five8 1.0.0` to `five8_core 0.1.2` lock resolution that
  fails to compile `solana-keypair 3.1.2`.
- `ureq 2`, `object 0.36`, and `gimli 0.31` remain pinned because they are host tooling dependencies and changing them does not improve on-chain safety or SBF output.

## Current RustSec ledger

Last checked: 2026-08-16 with `cargo audit --json --no-fetch` against the local RustSec cache.

The audit gate exits successfully with **0 vulnerability advisories, 4
unmaintained informational advisories, and 0 unsound advisories**. None of the
remaining informational packages are dependencies of `hopper-runtime`,
`hopper-systems`, or the default deployable SBF authoring path. They are still
tracked because the host CLI and validator-class test harness are part of the
public repository.

### Vulnerabilities removed on 2026-08-16

The five former blockers all came from the Agave 2.3 host graph. They were not
reachable from a Hopper SBF program, but they were reachable in public CLI,
RPC, signing, or devnet workflows and therefore were fixed rather than ignored.

| Advisory | Previous package and path | Previous lane | Resolution |
| --- | --- | --- | --- |
| `RUSTSEC-2024-0344` | `curve25519-dalek 3.2.0` through `ed25519-dalek 1.0.1` -> `solana-keypair` / `solana-signature` -> `hopper-cli`, the cross-program devnet runner, and example dev dependencies | Host signing and RPC clients; no SBF reachability | Agave `4.2.1` selects `ed25519-dalek 2.2.0` and patched `curve25519-dalek 4.1.3`. The vulnerable 3.2 package is absent from `Cargo.lock`. |
| `RUSTSEC-2022-0093` | `ed25519-dalek 1.0.1` through `solana-keypair 2.2.3` and `solana-signature 2.3.0` | Host keypair loading, signing, CLI, and devnet runners; no SBF reachability | The signing graph now uses `solana-keypair 3.1.2`, `solana-signature 3.4.1`, and `ed25519-dalek 2.2.0`. The vulnerable 1.0 package is absent from `Cargo.lock`. |
| `RUSTSEC-2026-0098`, `RUSTSEC-2026-0099`, `RUSTSEC-2026-0104` | `rustls-webpki 0.101.7` through `rustls 0.21` / `tokio-tungstenite 0.20` -> `solana-pubsub-client 2.3.13` -> `solana-client` | Host RPC WebSocket TLS only; no SBF reachability. Hopper did not call CRL parsing directly, but RPC certificate handling remained exposed. | Agave `4.2.1` selects `rustls 0.23.43`, `tokio-tungstenite 0.28.0`, and patched `rustls-webpki 0.103.13`. The vulnerable 0.101 package is absent from `Cargo.lock`. |

The same graph refresh removed `rand 0.7.3` and its
`RUSTSEC-2026-0097` unsoundness warning, and replaced unmaintained
`number_prefix 0.4.0` with `unit-prefix 0.5.2`.

The 2026-08-15 refresh also found and immediately removed four newly disclosed
issues whose patched versions fit the existing dependency constraints:

| Advisory | Resolution |
| --- | --- |
| `RUSTSEC-2026-0204` (`crossbeam-epoch 0.9.18`) | Lockfile updated to `0.9.20`. |
| `RUSTSEC-2026-0185` (`quinn-proto 0.11.14`) | Lockfile updated to `0.11.15`. |
| `RUSTSEC-2026-0190` (`anyhow 1.0.102`) | Lockfile updated to `1.0.103`. |
| `RUSTSEC-2026-0221` (`event-listener 5.4.1`) | Lockfile updated to patched `5.4.2`; affected path was host-only `async-lock` / Solana QUIC client code. |

### Remaining informational advisories

| Advisory | Package | Lane | Current dependency path | Reachability and disposition | Retirement condition |
| --- | --- | --- | --- | --- | --- |
| `RUSTSEC-2025-0141` | `bincode 1.3.3` | Host CLI, Agave client, and dev/test harness | Direct `hopper-cli` transaction decoding plus `solana-client 4.2.1`, `solana-account 4.3.1`, and `mollusk-svm 0.15.0` | Unmaintained, with no published vulnerability. It is absent from Hopper's deployable runtime crates. Legacy transaction wire compatibility still requires upstream bincode surfaces in Agave 4.2. | Migrate Hopper's direct decoding and sizing to the supported wincode/versioned-transaction APIs, then remove the warning when Agave and Mollusk no longer require bincode. |
| `RUSTSEC-2024-0388` | `derivative 2.2.0` | Build-time host validator graph | `ark-* 0.4` -> `light-poseidon 0.2` / `solana-poseidon 4.0` -> `solana-syscalls 4.2.1` -> Mollusk and the host BPF loader | Proc macro used while compiling the validator-class test stack. It is not linked into Hopper SBF programs. | Agave's Poseidon/Ark 0.4 compatibility graph removes `derivative` or migrates to a maintained derive helper. |
| `RUSTSEC-2025-0161` | `libsecp256k1 0.7.2` | Host validator execution | `solana-syscalls 4.2.1` -> `mollusk-svm 0.15.0` / `solana-bpf-loader-program 4.2.1` | Used by the host SVM to emulate Solana secp256k1 behavior. Hopper does not expose or link this crate in its deployable runtime. | Agave replaces the syscall implementation with `k256` or another maintained implementation. |
| `RUSTSEC-2024-0436` | `paste 1.0.15` | Build-time host validator graph | `ark-* 0.4/0.5` -> `light-poseidon` / `solana-bn254` / `solana-poseidon` -> `solana-syscalls 4.2.1` | Proc macro inherited by the host validator and cryptography build graph. Hopper's macro crates do not depend on it. | Upstream Ark/Solana graph migrates to `pastey`, another maintained helper, or no paste-style macro. |

## Re-check commands

```powershell
cargo audit --json --no-fetch
cargo tree --workspace -i curve25519-dalek@4.1.3 --locked --depth 5
cargo tree --workspace -i ed25519-dalek@2.2.0 --locked --depth 5
cargo tree --workspace -i rustls-webpki@0.103.13 --locked --depth 5
cargo tree --workspace -i bincode@1.3.3 --locked --depth 4
cargo tree --workspace -i derivative@2.2.0 --locked --depth 6
cargo tree --workspace -i libsecp256k1@0.7.2 --locked --depth 6
cargo tree --workspace -i paste@1.0.15 --locked --depth 6
cargo tree -p hopper-cli --depth 1
cargo tree -p hopper-runtime --depth 1
cargo search solana-client --limit 3
cargo search five8_const --limit 3
```
