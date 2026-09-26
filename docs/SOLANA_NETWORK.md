# Solana network compatibility

Alpenglow was rechecked **2026-09-26 08:41 UTC** through finalized public RPC,
with expected genesis, Feature ownership, and activation encoding checks.
These are observations, not authenticated ledger proofs or permanent constants.

## Alpenglow is active on devnet and testnet in this capture

The feature account `A1pengvuM6JEcyNuTnMqepBKhwHE3N6PmUrdATGawhJS` reports:

| Cluster | Finalized snapshot slot | Alpenglow gate |
|---|---:|---|
| Devnet | 504348469 | Active since 504144000 |
| Testnet | 445256318 | Active since 444620256 |
| Mainnet-beta | 450623655 | Absent |

Anza's [feature tracker](https://github.com/anza-xyz/agave/wiki/Feature-Gate-Tracker-Schedule)
records testnet epoch 1042 and devnet epoch 1167, with mainnet activation pending.
An activation slot is not a measured first Votor-finalized block or a latency
benchmark. Validator versions alone do not establish activation.

The [pinned SIMD-0326](https://github.com/solana-foundation/solana-improvement-documents/blob/f1afd941b9fa5061ea80a5401feb72172121ebb5/proposals/0326-alpenglow.md)
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

## Other runtime observations from September 25

The following table retains the **2026-09-25 06:23 UTC** capture; it was not all
requeried with the Alpenglow check.

| Feature | Devnet | Testnet | Mainnet-beta |
|---|---|---|---|
| 0321 instruction offset, 0339 CPI account limit, 0385 transaction v1 | Active | Active | Active |
| 0449 direct account pointers | Active | Active | Absent |
| 0459 syscall pointer restrictions and sBPF v3 | Active | Active | Active |
| 0460 virtual address adjustments | Active | Active | Unproven: System-owned account |
| Account-data direct mapping and 0512 SHA-512 | Active | Active | Absent |
| 0500 older-sBPF deployment restriction and 0049 remaining-compute syscall | Absent | Absent | Absent |
| CPI depth-eight gate | Absent | Absent | Absent |
| 0194 rent threshold and first two 0437 price stages | Active | Active | Active |
| Remaining three 0437 rent stages | Absent | Absent | Absent |
| Slot-time stages through 250 ms | Active | Active | Active |
| 200 ms slot-time stage | Active | Active | Absent |

Keep deployable defaults compatible with the actual target cluster. Query live
rent. A devnet test of direct account pointers or SHA-512 does not demonstrate
mainnet availability. The reviewed 0558 leader-info syscall remains a proposal,
not a shipped Hopper capability.

The [release record](https://hopperzero.dev/docs/release-status) separates these
network observations from Hopper's own compiled and devnet tests. Query the
actual target cluster again before enabling a gated feature.
