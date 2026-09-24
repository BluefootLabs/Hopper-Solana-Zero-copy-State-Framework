# Source and API refinement: 2026-09-24

Status: 0.3.1 release candidate under validation after the published 0.3.0 release.
This is a focused review of the boundaries below, not an independent audit of
Hopper or a claim that every line of every competitor has been audited.

## Sources reviewed

| Project and exact pin | Reviewed boundary | Consequence |
| --- | --- | --- |
| [Pina 3f04169](https://github.com/pina-rs/pina/commit/3f04169a2999fed45a93543a99223939ef93b512) | PDA macro's new `assert_stored_bump` and provenance-lint contract | Reuse a bump from the same validated account. Provenance, selected-address validity, canonicality, and the byte eventually persisted are distinct. |
| [Anchor v2 b3b47d1](https://github.com/otter-sec/anchor/commit/b3b47d1fb9575218207c9f08f896b04750369611) | `anchor-next` interface mint creation, minimum space check, supplied allocation | A reader API is not mint initialization. Hopper adds an explicit extension plan, including the exact-size requirement imposed by Token-2022. This branch pin is distinct from default-branch `97e2d91`. |
| [Quasar b0de7db](https://github.com/blueshift-gg/quasar/tree/b0de7db4cd271654a2dcf78807dd865e98e0b339) | Default head unchanged from the preceding PDA/account review | SHA-only verification of a selected bump does not establish canonicality. Quasar still has the lower increment in the dated comparison; no new peer benchmark is claimed here. |
| [Pinocchio adbd48d](https://github.com/anza-xyz/pinocchio/tree/adbd48d12229ffa30d6fb3d3a8ff777fdb053b80) | Lazy entrypoint and its duplicate-account return type | Its caller manages duplicate markers. Hopper's resolving lazy parser and typed validators retain their own responsibilities. A lazy API alone is not novel. |
| [Agave 9a06a94](https://github.com/anza-xyz/agave/tree/9a06a9426251a6c68547b9c52a75b5d713b0119c) | Feature-set keys and `invoke_context` compared with c17c596 | These two source files are unchanged across those pins. Cluster activation still needs observation; top-level callbacks alone are insufficient for complete nested CPI evidence. |
| [SIMDs 8b157e1](https://github.com/solana-foundation/solana-improvement-documents/tree/8b157e1def5fb3b3779f0935ec71cd7cae271207) | 0449, 0500, 0512, and 0558 | Document status and live activation differ. 0558 remains a leader-info draft; its proposed pointer checks and charges are not a live Hopper API. |

The Token-2022 wire/size oracle is `spl-token-2022-interface = 3.1.1`.
Compiled execution uses the token ELFs packaged by `mollusk-svm-programs-token
0.15.0`; public devnet validation is a separate evidence lane.

## Changes and their contracts

- `MintPlan` pairs the exact allocation with six explicitly configured,
  fixed-size mint extensions. It rejects duplicates, invalid nullable addresses,
  out-of-range fee rates, and mismatched allocation. It initializes extensions
  before the base mint and supports live-rent funding, prefunding and PDA signers.
  It does not initialize every extension or populate metadata behind a pointer.
- Every generated extension constraint now checks Token-2022 ownership before
  inspecting TLV bytes. Raw byte readers still require caller-supplied ownership
  and authority checks. The compiled regression presents identical bytes under
  different owners.
- Direct required binding retains the bump produced by validation for supplied,
  stored and seed-helper paths, extending the existing bare-bump optimization.
  Optional and composite gather paths remain separate. Retention does not turn
  selected-bump verification into canonical search.
- The PDA fixture checks creation, persistence of the validated bump,
  reinitialization refusal, corrupted stored bytes, and matching noncanonical
  selected bumps. `#[bump]` is an offset marker, not an automatic field initializer.

## Current cluster observations

The [raw and decoded observations](../audit/framework-refinement-2026-09-24/)
use public RPC at finalized commitment and verify genesis and Feature ownership.
They are observations from those endpoints, not authenticated ledger proofs.

| Feature | Devnet | Testnet | Mainnet-beta |
| --- | --- | --- | --- |
| 0321 instruction offset, 0339 CPI accounts, 0385 transaction v1 | Active | Active | Active |
| 0449 direct account pointers | Active | Active | Absent |
| 0459 syscall pointer restrictions | Active | Active | Active |
| 0460 virtual address adjustments | Active | Active | Unproven: returned account is not Feature-owned |
| sBPF v3 deployment/execution | Active | Active | Active |
| 0500 older-sBPF deployment restriction | Absent | Absent | Absent |
| 0512 SHA-512 | Active | Active | Absent |
| 0049 remaining-compute syscall | Absent | Absent | Absent |
| Account-data direct mapping | Active | Active | Absent |
| 0194 rent threshold and first two 0437 price stages | Active | Active | Active |
| Remaining three 0437 price stages | Absent | Absent | Absent |

0558 remains Draft with no feature key in the reviewed proposal. A prototype
must not introduce its syscall into a default deployable program.

## Documentation and product narrative

The Token-2022 guide previously used a nonexistent constant path, included an
undefined setter in its example, and attributed constraints to the vault example
that its source does not declare. The replacement uses compiled API examples
and separates mint creation from readers, authority checks, and extension policy.

Architecture/model pages described the legacy `hopper:v1` hash as the universal
fingerprint, required append-only migrations despite shipped transformations,
and claimed no global runtime state despite scoped borrow/policy registries.
The updated pages describe the current proc-macro `hopper:wire:v2` descriptor,
separate body/account sizing, and the actual migration and runtime contracts.
Nested user types and aliases still require versioning review; the macro does
not recursively inspect arbitrary type definitions.

The homepage now moves from a program example to validation, state control,
applications, effect inspection, dated measurements, and installation. It
retains workload-specific CU and binary-size evidence and whole-account Solana
locking. It does not claim universal speed, safety, fee reductions, or novelty.

## Cicada and Grillo

Cicada remains useful as an adversarial settlement application for byte-range
policies, route binding, authority checks, and rollback. Its existing compiled
SPL/Token-2022 evidence does not establish an audited production route adapter
or authenticated third-party artifact policy.

Grillo remains useful as a separate recomputation boundary over supplied
snapshots and acquisition evidence. Caller labels are not authenticated.
Complete invocation-bound replay needs nested CPI entry/exit capture, pinned
executable identities, and explicit completeness handling; top-level runtime
callbacks cannot substitute for it.

The strongest follow-up opportunities are completing that replay evidence,
versioned transitive layout-identity support, and extending mint plans only as
each additional extension's size, ordering, and processor behavior are tested.
These are engineering targets, not claims of a first-ever Solana capability.

## Validation record

Canonical interface comparisons pass for all 64 supported extension subsets
and all emitted instruction formats. Five compiled mint tests and three PDA
tests pass on SBF v0 and v3, including a compute-exhaustion rollback after
successful creation and extension CPIs. The rebuilt 154,160-byte Cicada v0
artifact passes all 23 compiled lifecycle tests. These are diagnostic builds
before final release capture; artifact records will identify their exact source.

The initial full host run identified two stale tests: one inspected the former
unfused validator body, and the documentation-surface guard rejected a raw
account signature in the new guide. The validator assertion now follows the
fused implementation; the guide uses typed signer inputs. Both corrected
targets pass, and the guide's three complete examples compile.

The unsafe-contract scan covers 308 Rust files across 29 public packages.
Final clean-source regressions, devnet, website checks, and registry publication
remain release gates. The confirmed registry framework release remains 0.3.0.
