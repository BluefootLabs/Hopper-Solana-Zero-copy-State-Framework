# Solana network compatibility

Alpenglow was rechecked **2026-10-01 01:28 UTC (September 30 locally)** through finalized public RPC,
with expected genesis, Feature ownership, and activation encoding checks.
These are observations, not authenticated ledger proofs or permanent constants.

## Alpenglow is active on devnet and testnet in this capture

The feature account `A1pengvuM6JEcyNuTnMqepBKhwHE3N6PmUrdATGawhJS` reports:

| Cluster | Finalized snapshot slot | Alpenglow gate |
|---|---:|---|
| Devnet | 506103299 | Active since 504144000 |
| Testnet | 447094610 | Active since 444620256 |
| Mainnet-beta | 452139219 | Absent |

Anza's [feature tracker](https://github.com/anza-xyz/agave/wiki/Feature-Gate-Tracker-Schedule)
records testnet epoch 1042 and devnet epoch 1167, with mainnet activation pending.
An activation slot is not a measured first Votor-finalized block or a latency
benchmark. Validator versions alone do not establish activation.

The [pinned SIMD-0326](https://github.com/solana-foundation/solana-improvement-documents/blob/4b643ca8746742183a469681765e694b385bb315/proposals/0326-alpenglow.md)
covers Votor. Rotor, smart sampling, and lazy/asynchronous execution are outside
its scope. Its `Review` label illustrates why proposal status and deployed
features must be checked separately.

## Program implications

Anza's [Alpenglow operator guide](https://docs.anza.xyz/consensus/alpenglow)
preserves Clock's layout and whole-second resolution. During execution,
`unix_timestamp` estimates when the parent block ended; `slot` still names the
current slot. Use approximate time or explicit slot deadlines. Do not convert
slots to seconds with a fixed constant or promise subsecond expiry from Clock.

Consensus finality does not remove account locks, compute charges, or application
authorization requirements. Hopper byte policies continue to govern tracked
mutation inside the program; they do not change the scheduler's account locks.

## Other runtime observations from September 30

The same finalized capture checked 22 feature accounts per cluster, with source
keys pinned to Agave `b1912476edbb6907f647ff36e6be7808c18f5c2e`.

| Feature | Devnet | Testnet | Mainnet-beta |
|---|---|---|---|
| 0321 instruction offset, 0339 CPI account limit, 0385 transaction v1 | Active | Active | Active |
| 0449 direct account pointers | Active | Active | Absent |
| 0459 syscall pointer restrictions and sBPF v3 | Active | Active | Active |
| 0460 virtual address adjustments | Active | Active | Unproven: System-owned account |
| Account-data direct mapping and 0512 SHA-512 | Active | Active | Absent |
| 0500 older-sBPF deployment restriction and 0049 remaining-compute syscall | Absent | Absent | Absent |
| 0194 rent threshold and first two 0437 price stages | Active | Active | Active |
| Remaining three 0437 rent stages | Absent | Absent | Absent |
| Slot-time stages through 250 ms | Active | Active | Active |
| 200 ms slot-time stage | Active | Active | Absent |

Keep deployable defaults compatible with the actual target cluster. Query live
rent. A devnet test of direct account pointers or SHA-512 does not demonstrate
mainnet availability. The reviewed 0558 leader-info syscall remains a proposal,
not a shipped Hopper capability. The SHA-512 syscall gate is
`s512oDwgx8hjMnaQjXfqqrZroVj4HvC6TkN3iSSWXCh`; Hopper binds `sol_sha512`
(`hopper::hash::sha512`) only under the `sha512-syscall` cargo feature, because
a program that references the symbol fails to load where the gate is inactive.

The [release record](https://hopperzero.dev/docs/release-status) separates these
network observations from Hopper's own compiled and devnet tests. Query the
actual target cluster again before enabling a gated feature.

The CPI depth-eight gate was absent in the separate September 25 capture; it
was not included in this 22-feature refresh.

## What is scheduled next (checked 2026-09-28)

Anza's feature-gate tracker lists six gates pending mainnet-beta activation
and none pending on devnet or testnet: Alpenglow
(`A1pengvuM6JEcyNuTnMqepBKhwHE3N6PmUrdATGawhJS`), the 200 ms slot stage
(`iBRLjhJnkmDZgNoZRDMW11d8ZV7HvsL3vAyRjZB5npW`), the SHA-512 syscall
(`s512oDwgx8hjMnaQjXfqqrZroVj4HvC6TkN3iSSWXCh`), virtual address space
adjustments (`7VgiehxNxu53KdxgLspGQY8myE6f7UokaWa4jsGcaSz`), account-data
direct mapping (`CR3dVN2Yoo95Y96kLSTaziWDAQT2MNEpiWh5cqVq2pNE`), and direct
account pointers in the program input
(`ptr9umikaeAS7ZBBp2fsfRhie16F1V2jCKA2y6gXNAK`). Hopper's account parser has
both input layouts, so the pointer-table activation changes nothing for a
deployed program; the SHA-512 wrapper stays behind its cargo feature until the
gate lands.

Agave's published schedule puts v4.4 on testnet from 2026-09-28, on devnet from
2026-10-05, and recommends it for mainnet on 2026-11-02. The Solana Foundation's
upgrade page for sBPFv3 programs expects SIMD-0500 to activate in November 2026,
after which a deployment, upgrade, or finalization of a program built for sBPF
v0, v1, or v2 is refused while already deployed programs keep running. Hopper's
toolchain builds v3 today (`cargo build-sbf --arch v3`, `scripts/attest-sbf-release.py
--arch v3`), and the comparison bench carries sBPFv3 rows next to the v0 rows so
the v3 numbers are measured rather than assumed; build and deploy new programs
as v3 before that date.

Proposals that were in Draft or Review on 2026-09-28 and are not shipped
Hopper capabilities: SIMD-0670 (a second `invoke_signed` syscall for ABIv1),
SIMD-0568 (deprecating precompiles in favor of syscalls), SIMD-0646
(retiring the legacy and v0 transaction formats; `hopper tx send --v1` already
builds the surviving format), SIMD-0596 (a 96-account lock limit for
transaction v1), SIMD-0582 (early instruction-trace overflow detection),
SIMD-0648 (unbounded loader-v3 instruction data), SIMD-0645 (SVM JIT
intrinsics), SIMD-0376 (relaxed signature verification), and SIMD-0558 (a
leader-info syscall). The rent repricing (SIMD-0437) has its first two stages
active on every cluster and the remaining three planned for a later Agave line;
the 250 ms stage is active on all three clusters, and the 200 ms stage is active on devnet and testnet only.
