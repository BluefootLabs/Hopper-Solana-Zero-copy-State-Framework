# On-chain framework review: 2026-09-25

This review refreshes the specific source and network boundaries below. It is
not an independent security audit, an exhaustive review of every repository,
or a new execution benchmark of the peer frameworks.

## Alpenglow: testnet evidence, separate deployment assumptions

At **2026-09-25 06:23 UTC**, public RPC returned the Alpenglow feature account
`A1pengvuM6JEcyNuTnMqepBKhwHE3N6PmUrdATGawhJS` as Feature-owned and activated at
testnet slot **444620256**. The finalized observation was at slot **444825516**,
in epoch **1042**. The account was absent on devnet and mainnet-beta at the
observed slots below. Anza's [feature tracker](https://github.com/anza-xyz/agave/wiki/Feature-Gate-Tracker-Schedule)
agrees: Alpenglow is awaiting devnet activation and records testnet epoch 1042.
The feature activation slot is not a measurement of the first Votor-finalized
block, finality latency, or a claim that all Alpenglow-related features are active.

| Public endpoint | Finalized feature snapshot slot | Alpenglow feature |
| --- | ---: | --- |
| `https://api.devnet.solana.com` | 503855867 | Absent |
| `https://api.testnet.solana.com` | 444825516 | Active since 444620256 |
| `https://api.mainnet-beta.solana.com` | 450270673 | Absent |

The capture verifies the expected genesis hash, Feature program ownership,
non-executable account status, option encoding, and activation slot against the
observed finalized slot. These are **unauthenticated public RPC observations**,
not authenticated ledger proofs. All three endpoints reported Agave 4.3.0 and
feature-set 3383571666; software version alone is not activation evidence.

Anza's [v4.3 release schedule](https://github.com/anza-xyz/agave/wiki/v4.3-Release-Schedule)
lists September 28 as the tentative resumption of mainnet feature activation.
It does not establish a mainnet Alpenglow activation date. A community test
cluster or the [Alpenglow challenge's configured feature set](https://github.com/anza-xyz/alpenglow/blob/master/RULES.md)
also does not establish public-cluster activation.

The [pinned SIMD-0326](https://github.com/solana-foundation/solana-improvement-documents/blob/f1afd941b9fa5061ea80a5401feb72172121ebb5/proposals/0326-alpenglow.md)
covers Votor. Rotor, smart sampling, and lazy/asynchronous execution are outside
its scope. Its front matter still says `Review`, illustrating why proposal
labels cannot substitute for runtime observations.

The most direct on-chain compatibility consequence is documented in Anza's
[operator guide](https://docs.anza.xyz/consensus/alpenglow): Clock keeps its layout
and whole-second resolution, while `unix_timestamp` estimates the parent
block's end during transaction execution. `slot` remains the current slot.
Hopper programs should use explicit slot or approximate timestamp deadlines,
avoid fixed slot-to-seconds conversions, and avoid promising subsecond expiry
from Clock. Faster consensus does not remove account locks, execution compute
cost, signer validation, or application authorization requirements.

## Runtime features checked against source and public RPC

Feature IDs come from [Agave c9e429e](https://github.com/anza-xyz/agave/blob/c9e429ef8db5ba67bf3a02c9cd40e55f5164e99f/feature-set/src/lib.rs).
The feature-set file is unchanged from the preceding source pin. The table
reports the captured state, not a promise about later activation.

| Feature or gate | Devnet | Testnet | Mainnet-beta |
| --- | --- | --- | --- |
| 0321 instruction offset, 0339 CPI accounts, 0385 transaction v1 | Active | Active | Active |
| 0449 direct account pointers | Active | Active | Absent |
| 0459 syscall pointer restrictions | Active | Active | Active |
| 0460 virtual address adjustments | Active | Active | Unproven: System-owned account |
| Account-data direct mapping | Active | Active | Absent |
| sBPF v3 deployment/execution | Active | Active | Active |
| 0500 older-sBPF deployment restriction | Absent | Absent | Absent |
| 0512 SHA-512 | Active | Active | Absent |
| 0049 remaining-compute syscall | Absent | Absent | Absent |
| CPI nesting limit of eight | Absent | Absent | Absent |
| 0194 rent threshold; first two 0437 price stages | Active | Active | Active |
| Remaining three 0437 rent stages | Absent | Absent | Absent |
| Slot-time stages through 250 ms | Active | Active | Active |
| 200 ms slot-time stage | Active | Active | Absent |
| 0337 shred markers; 0357 validator admission gate | Active | Active | Active |
| Separate `alpenglow_fast_leader_handover` gate | Absent | Absent | Absent |

The separate handover key is
`FastLeaderHandover11111111111111111111111111`; the shred-marker key is
`disCA4efguFL6Wqa4pGdG7jpjC7C5uiKzKnhEBqchBe`. Treating these as the same feature
would produce an incorrect activation claim. The depth-eight key is
`6TkHkRmP7JZy1fdM6fg5uXn76wChQBWGokHBJzrLB3mj`.

For Hopper, maintain deployable defaults for the target cluster, test both sBPF
v0 and v3 where supported, use live rent calculations, and keep optional syscall
experiments separate from supported program behavior. A successful devnet test
does not demonstrate mainnet activation of direct pointers or SHA-512.

## Refreshed peer sources and useful engineering lessons

| Project and exact pin | Reviewed boundary | Consequence for Hopper |
| --- | --- | --- |
| [Pina 535c423](https://github.com/pina-rs/pina/commit/535c423f97a6155710e765192d314f192870f9b8) | Changes since 3f04169: account cursor identity, resize length agreement, token-balance reload and close/drain lint contracts | Runtime-resolved duplicate identity can avoid repeated key comparisons, but host fixtures and alias-order contracts matter. Reject inconsistent resize predictions in release builds. Post-CPI accounting must use observed balances and propagate refusal. |
| [Anchor v2 e271abf](https://github.com/otter-sec/anchor/commit/e271abf2977f32d59cf2af959b0d8bb4d687441d) | `anchor-next` changes since b3b47d1: serialization trait facade, event decoding derive, representation test and legacy IDL generics | Public program and client APIs should not require users to name transitive codec traits. Test derives, generic bounds, wire bytes, and decoding together. This refresh does not establish new mint-extension support in Hopper. |
| [Quasar b0de7db](https://github.com/blueshift-gg/quasar/tree/b0de7db4cd271654a2dcf78807dd865e98e0b339) | Default head unchanged | Preserve the earlier PDA/account-validation conclusions. Source-head verification is not a fresh performance measurement. |
| [Pinocchio adbd48d](https://github.com/anza-xyz/pinocchio/tree/adbd48d12229ffa30d6fb3d3a8ff777fdb053b80) | Default head unchanged | Preserve the earlier lazy-entrypoint and duplicate-account boundaries; lazy parsing by itself is not a novelty claim. |
| [Agave c9e429e](https://github.com/anza-xyz/agave/tree/c9e429ef8db5ba67bf3a02c9cd40e55f5164e99f) | Source-head diff, feature-set declarations, Alpenglow operational contract | Recent Votor work is consensus implementation work. Do not infer program execution semantics or nested-CPI evidence from release labels. |
| [SIMDs f1afd94](https://github.com/solana-foundation/solana-improvement-documents/tree/f1afd941b9fa5061ea80a5401feb72172121ebb5) | Latest change clarifies 0215 lattice-hash security; 0326 and previously reviewed 0449/0500/0512/0558 unchanged | Do not repurpose an accounts-state accumulator as a general proof of execution or claim every hash construction has the same security contract. The 0558 leader-info syscall remains a proposal, not a Hopper capability. |

Pina's [cursor implementation](https://github.com/pina-rs/pina/blob/535c423f97a6155710e765192d314f192870f9b8/crates/pina/src/traits.rs)
compares AccountView identity after the runtime deserializer resolves duplicate
slots. Its check looks forward: a readonly slot followed by a mutable alias is
allowed; a mutable slot with a later alias is rejected. Separate host headers
with the same address do not have that identity. Any Hopper optimization must
preserve its own alias and borrow rules and test runtime-shaped duplicates.

Pina's updated token-balance and drain lints inspect source usage; they do not
independently enforce a policy inside a deployed transaction. Their useful
application lesson is to propagate checked failures, reread affected accounts
after CPI, and account from actual deltas. Hopper can demonstrate those
invariants directly in on-chain examples while retaining its distinct byte
policy and CPI boundaries. This is an engineering opportunity, not evidence
that another framework cannot implement equivalent application checks.

## On-chain priorities supported by this review

1. Exercise a complete program flow with authority checks, bounded byte writes,
   replay or revision checks, and observed post-CPI state. Include adversarial
   refusal cases in compiled execution and devnet evidence.
2. Make the supported safe path concise: public API names, generated policy
   bounds, and examples should match the deployed code and avoid manual offsets
   where the type already describes the layout.
3. Measure execution overhead separately from consensus finality and network
   fees. Keep the dated comparison honest: Hopper's recorded counter increment
   is 349 CU; Quasar's pinned published row remains lower at 330 CU.
4. Keep Cicada useful as an on-chain settlement and policy exercise. Keep Grillo
   as a separate evidence/recomputation tool; it is not a dependency required
   for Hopper programs to enforce their own runtime rules.

These priorities are inferred from the reviewed source and runtime contracts;
they are not a Solana Foundation endorsement or a universal framework ranking.
The capture script, exact source heads, source diffs, checksums, public RPC
responses, and decoded observations are saved under
`target/hopper/onchain-2026-09-25/research/` for the release evidence archive.
