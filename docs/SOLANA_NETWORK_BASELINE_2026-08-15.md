# Solana network baseline, reviewed 2026-08-17; corrected 2026-09-06

This file is a dated compatibility baseline, not a prediction. “Live” means the
Solana Foundation or the on-chain feature account confirms Mainnet activation.
Targets and schedules remain “upcoming” until that happens.

> **2026-09-19 correction:** transaction v1 activated on mainnet-beta on
> 2026-09-15, and rent fell to 5,080 lamports per byte on 2026-09-11. See
> [ARCHITECTURE.md](ARCHITECTURE.md).

> **2026-09-06 correction:** SIMD-0525 made the block and per-account CU
> ceilings slot-time-dependent. Mainnet is now in the 300 ms regime, so the
> SIMD-0286-scaled ceilings are **75M block / 30M per writable account**, not
> 100M/12M. SIMD-0437 step 1 also activated 2026-09-03, moving the live
> rent-exempt reserve coefficient from 6,960 to **6,333 lamports per byte**.
> See the
> [reverified refresh](ARCHITECTURE.md) for the full table and
> live feature observations.

## Confirmed Mainnet state

| Change | Status reverified 2026-09-06 | Hopper consequence |
|---|---|---|
| SIMD-0286 cost-limit scaling | **Live** since 2026-07-29, epoch 1009; composed with the current 300 ms slot regime | `hopper contention` derives the observed **75M block / 30M per-account** ceilings from the regime and gate. |
| Optimized Token Program / p-token (SIMD-0266) | **Live** | Existing token instructions remain compatible. Hopper must benchmark against the optimized program before publishing comparative CU claims. |
| BLS pubkey registration (SIMD-0387) | **Live** | Validator-facing; it does not activate Alpenglow or change Hopper program execution semantics. |

Agave composes its
[slot-time parameters](https://github.com/anza-xyz/agave/blob/v4.2.2/runtime/src/slot_params.rs)
with the SIMD-0286 feature. At 300 ms the base 18M-account/45M-block pair is
scaled by 100/60 to 30M/75M. The separate 100 MB block account-data delta
ceiling is unchanged. Hopper reports both compute ceilings and does not present
the old 100M headline as an instruction budget.

Agave v4.2.2 is the stable validator pin used for the 2026-09-03 correction.
Hopper's host-side Solana dependencies and forward SBF lane were aligned to
v4.2.1 at the original review.
The existence of a stable validator release still does not prove that every
feature implemented in its source is active on Mainnet.

## Proposed or scheduled work, not transaction-v1 activation

| Change | Current official status | Hopper position |
|---|---|---|
| 4,096-byte transactions (SIMD-0296 + SIMD-0385) | Both SIMDs remain **Review** at official snapshot `fc519fb3`; neither names an activated feature. Agave contains an `enable_tx_v1` feature id, but the feature tracker has no transaction-v1 activation entry. Finalized queries on 2026-08-17 found no feature account on devnet at slot 484,716,765 or testnet at slot 429,996,204. Mainnet activation was not confirmed. | Hopper's upgraded Agave 4.2.1 host stack still emits legacy transactions. Those remain capped at 1,232 bytes. Hopper must not advertise, submit, or silently assume the proposed 4,096-byte v1 envelope. |
| Account Data Direct Mapping | **Active on devnet/testnet; Pending Mainnet Beta Activation** in Anza's schedule, rechecked 2026-09-06. | Mainnet still uses the serialized/copy path. Model current full data length for test-cluster/upcoming first-write CoW guidance; never substitute a dynamic layout's minimum prefix or call the result a current Mainnet CU quote. |
| Reduced rent (SIMD-0437) | Step 1 live since 2026-09-03; four later gates absent in the 2026-09-06 finalized query | Read the live Rent sysvar/RPC. Current coefficient is 6,333 lamports per byte; do not bake later projected reductions into cost claims. |
| Reduced slot times (SIMD-0525) | Agave 4.2 staged rollout from 400ms toward 200ms | Treat latency as cluster state, not a framework guarantee. |
| Alpenglow (SIMD-0326) | Not activating in Agave 4.2; currently targeted for Agave 4.3 | No current program API change. Avoid equating BLS/VAT prerequisites with Alpenglow being live. |

## Transaction v1 details that affect Hopper

The proposed 4,096-byte limit belongs only to the v1 envelope. Legacy and v0
remain unchanged at 1,232 bytes. The reviewed proposal permits up to 12
signatures and 64 addresses/instructions, uses leading version byte 129, does
not support address lookup tables, and carries compute configuration in a
header mask rather than Compute Budget Program instructions. These are proposal
details, not live acceptance rules. Raw-transaction indexers would need a new
decoder after finalization and activation.

Hopper's readiness rule is:

1. Fail early when a Hopper-built legacy transaction exceeds 1,232 serialized
   bytes.
2. Keep legacy/v0 support after v1 activation.
3. Add v1 construction only after the SDK exposes the finalized type and the
   Mainnet feature is queryable/active.
4. Add golden wire fixtures and RPC/indexer decoding tests before claiming v1
   support.

The Agave 4.2 schedule's week-of-2026-08-17 window is a generic, tentative
feature-activation window. It is not a confirmed transaction-v1 activation
date. Presence of the `enable_tx_v1` id in Agave source is implementation
evidence only.

## Primary sources

- [Solana Foundation: 100M CU Blocks](https://solana.com/upgrades/100m-cu-blocks)
- [Solana Foundation: Larger Transaction Sizes](https://solana.com/upgrades/larger-transaction-sizes)
- [Solana Foundation: Agave 4.2 Release Overview](https://solana.com/upgrades/agave-4-2-release-overview)
- [SIMD-0296: Larger Transactions](https://github.com/solana-foundation/solana-improvement-documents/blob/fc519fb3d1ef0f7624b6232bda958438feba09ce/proposals/0296-larger-transactions.md)
- [SIMD-0385: Transaction v1](https://github.com/solana-foundation/solana-improvement-documents/blob/fc519fb3d1ef0f7624b6232bda958438feba09ce/proposals/0385-transaction-v1.md)
- [Anza: Agave 4.2 release schedule](https://github.com/anza-xyz/agave/wiki/v4.2-Release-Schedule)
- [Anza: feature-gate tracker](https://github.com/anza-xyz/agave/wiki/Feature-Gate-Tracker-Schedule)
- [Anza: transaction-v1 feature id in Agave v4.2.1](https://github.com/anza-xyz/agave/blob/v4.2.1/feature-set/src/lib.rs)
- [Solana Foundation: Optimized Token Program](https://solana.com/upgrades/p-token)
- [Anza: Agave releases](https://github.com/anza-xyz/agave/releases)
- [Anza: Agave changelog](https://github.com/anza-xyz/agave/blob/master/CHANGELOG.md)
