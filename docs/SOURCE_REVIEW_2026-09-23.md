# Source and runtime review, 2026-09-23

This is an internal review of the named paths and changes below, not a claim
that every line of Hopper or each competitor has received an independent audit.
The framework's useful distinction remains direct account state combined with
declared byte authority, enforcement before tracked mutable access, and
inspectable effect evidence. Benchmarks do not establish universal fastest status.

## Primary source refresh

| Project and source pin | Reviewed scope | Consequence for Hopper |
|---|---|---|
| [Pina `29b91a0`](https://github.com/pina-rs/pina/commit/29b91a0ae6f1ca14f2292d0d24bac9afae466a0e) | Compact account creation, stored-bump trait, mismatch rejection and initialization cleanup | A validated derivation and the byte eventually stored by initialization are different invariants. Hopper's explicit supplied/stored-bump paths must not imply canonicality merely from ownership. |
| [Anchor v2 `b3b47d1`](https://github.com/otter-sec/anchor/commit/b3b47d1fb9575218207c9f08f896b04750369611) | `spl-v2/src/token_interface.rs` and the mint-space regression | Interface mint initialization now honors supplied space and rejects undersized allocations. Hopper's token interface readers and constraints are not equivalent to this mint-initialization API; do not advertise unsupported automatic extension initialization. |
| [Quasar `b0de7db`](https://github.com/blueshift-gg/quasar/tree/b0de7db4cd271654a2dcf78807dd865e98e0b339) | Refreshed source head against the preceding account-validation/PDA review | Head unchanged. SHA-only verification of an already selected bump is distinct from proving that a bump is canonical. The dated comparison still records Quasar's lower counter increment CU. |
| [Anza Pinocchio `adbd48d`](https://github.com/anza-xyz/pinocchio/tree/adbd48d12229ffa30d6fb3d3a8ff777fdb053b80) | `sdk/Cargo.toml`, `sdk/src/entrypoint/lazy.rs`, entrypoint boundary | The lazy entrypoint exposes duplicate-account handling to its caller. Hopper's count-exact and typed paths retain their own validation responsibilities; a lazy API alone is not a novel performance claim. |
| [Agave `c17c596`](https://github.com/anza-xyz/agave/tree/c17c5962f7e96fe72d37fefb2b6702b7447746a3) | Feature-set keys and runtime callback boundaries | Source support must be checked against cluster activation. Top-level execution callbacks alone cannot supply complete nested CPI entry/exit evidence. |
| [SIMDs `8b157e1`](https://github.com/solana-foundation/solana-improvement-documents/tree/8b157e1def5fb3b3779f0935ec71cd7cae271207) | 0449, 0500, 0512 and latest 0558 draft changes | Proposal labels can lag live activation. The leader-info syscall's draft pointer validation and cost are proposals, not a live Hopper capability. |

## Corrections and improvements

The PDA review covered the literal macro parser/search, generated seeds and
bump checks, generated validation/binding/gathering, runtime helper contracts,
CLI seed derivation, and the new SBF fixture and SDK oracle.

Bare `bump` and `seeds_fn` now use canonical, curve-checked search for typed
accounts as well as unchecked accounts. The previous matching-address search
could accept a lower off-curve bump for an already owned account. Ownership
does not establish a unique canonical address. This correction intentionally
tightens acceptance for applications that used bare `bump` with noncanonical
addresses. Explicit or stored bumps remain the way to verify a selected bump.

Direct required fields with bare `bump` retain the result returned by their
validator during binding. The public per-field validation API still runs all
checks, and field/error order is retained. Optional fields, typed seed helpers
and nested-context gathering still derive separately. The fixture compares
valid results and refusals; its explicit double-validation mode is a synthetic
comparison, not a historical before-binary measurement.

The compiled fixture also exposed a generated lifetime error for a
`seeds_fn` helper returning an array by value. Expansion now binds the helper
result before borrowing its seed slices, keeping it alive through derivation.

`canonical_pda!` accepts an explicit base58 program literal and byte-string
seed literals. It checks seed limits, searches descending bumps and checks the
curve on the build host, then emits only address bytes and the bump. It does
not discover a program ID through the filesystem or environment. The macro
does not replace account ownership, layout or privilege constraints.

The CLI's PDA search now enforces the same seed domain. `feature-gate` now
validates finalized RPC context, exact response cardinality, account ownership,
encoding and activation data. Reports retain network and slot identity;
`--require` provides an exit status for deployment prerequisites. These remain
RPC observations and do not authenticate a ledger.

## Observed network state

The [raw responses and observations](../audit/network-features-2026-09-23/README.md)
record devnet slot 503127420, testnet slot 444225296 and mainnet slot 449816747.

| Feature | Devnet | Testnet | Mainnet |
|---|---|---|---|
| 0321, 0339, transaction v1 (0385), 0459, SBPF v3 | Active | Active | Active |
| Direct account pointers (0449) | Active | Active | Absent |
| SHA-512 (0512) | Active | Active | Absent |
| Account-data direct mapping | Active | Active | Absent |
| Virtual address changes (0460) | Active | Active | Unproven: System-owned account |
| Disable old SBF deployments (0500), remaining-CU syscall (0049) | Absent | Absent | Absent |
| Rent threshold change (0194), first two 0437 rent stages | Active | Active | Active |
| Remaining three 0437 rent stages | Absent | Absent | Absent |

Hopper's CLI already supports explicit transaction-v1 sending. Legacy/v0
messages retain their 1,232-byte limit; activation does not upgrade a client
envelope automatically. The 0558 leader-info draft is not included as an
activated feature in this observation.

## Cicada and Grillo

The Cicada review follows execution from access and custody checks through
remaining-account validation, route-envelope hashing, CPI, token/mint policy
snapshots, observed deltas, refund and exact-cell settlement. Exact-route mode
does not bind an upgradeable route's executable bytes. A loader-state policy
must account for program/ProgramData linkage, deployment slot and execution
cache visibility; hashing arbitrary caller-supplied bytes would not close it.
Canonical token-processor fixtures are useful regression tests, but are not
a third-party AMM integration. Both remain planned acceptance milestones.

The Grillo review covers v0.2 frame identity, quotas, aliases, child context,
privileges, rollback and commitment binding. `ReplayClaimed` and provider
signature fields remain untrusted labels. Mollusk 0.15.1's inspection callback
surrounds top-level processing; it does not itself capture every nested CPI's
entry/exit state. A producer must refuse incomplete nested evidence or add a
runtime capture mechanism before claiming authenticated replay. Signing an
unverified caller frame would authenticate its sender, not prove execution.

The review also found that textual provenance labels were outside the frame's
existing resource quotas. The binder now bounds RPC and replay-source labels
to 4,096 UTF-8 bytes at every depth before encoding or cloning the frame.

Cicada's bounded custody and settlement, and Grillo's offline effect checking,
remain useful work with distinct deliverables. Neither requires a universal
speed or never-before-done claim to justify continued development.
