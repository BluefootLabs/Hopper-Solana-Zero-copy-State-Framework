# Why Hopper

Hopper is a zero-copy Solana program framework for developers who want direct
state access and explicit validation, mutation, and upgrade contracts. Start
with Rust account declarations and handlers. Add byte-range policies, generated
clients, migrations, and release inspection when your program needs them.

## Start with the program

Typed accounts validate ownership and layout before handler access. Account
constraints express signer, address, authority, and PDA requirements. Headered
layouts include version and layout identity; compact layouts use a smaller
discriminator and size contract, with their fingerprint carried in metadata.

You choose the access level. Whole-layout loads, field and segment access,
external-account adapters, and raw escapes have different guarantees. The
[memory-access guide](https://hopperzero.dev/docs/memory-access) explains those boundaries.

## Control what can change

`strict_writes` authorizes mutable acquisition of declared ranges on
Hopper-tracked paths. Disjoint segment borrows can coexist inside one invocation.
Solana still locks entire writable accounts; these policies do not create
sub-account scheduling or an automatic fee discount.

Opt-in touch maps record the ranges acquired. Grillo can then recompute
`changed ⊆ acquired ⊆ authorized` from a manifest and supplied snapshots.
The [effect-verification guide](https://hopperzero.dev/docs/effect-verification) explains PASS,
VIOLATION, INCONCLUSIVE, and the evidence that each verdict requires.

Before an upgrade, compare declared authority between releases. Hopper's
release-interface commitment binds declarations to an ELF. It does not prove
arbitrary handler behavior, and Grillo does not yet authenticate ledger replay.

## Keep useful framework ergonomics

Hopper provides checked CPI, token helpers, generated clients in six languages,
bounded dynamic fields, account-byte collections, and typed migrations with
explicit grow or shrink policies. Rent funding uses live values. Applications
still define transformation rules and business invariants.

The 0.3.1 refinement adds an exact-size `MintPlan` for legacy base mints and
six fixed-size Token-2022 mint extensions. The plan initializes extensions
before the base mint, accepts signer or PDA creation, and preserves excess
prefunding. It does not initialize every extension or populate metadata behind
a pointer. See the [mint guide](https://hopperzero.dev/docs/token-2022) and [release status](https://hopperzero.dev/docs/release-status).

## Learn from the other frameworks

The [September 24 source review](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/SOURCE_REVIEW_2026-09-24.md)
records exact pins and bounded source scopes. Its conclusions apply to those
scopes, rather than every API or line in each project.

| Reviewed area | What matters when choosing or writing an API |
|---|---|
| Pina stored-bump provenance | Validate the selected address, prove canonicality when required, and explicitly persist the validated bump. Ownership alone establishes none of those three. |
| Anchor v2 interface mint allocation | Reading extensions and initializing them are distinct APIs. Hopper's new plan covers six explicit extensions and enforces the processor's exact-size contract. |
| Quasar PDA and counter paths | Retaining a validated bump avoids repeated work. Quasar's pinned published increment is still lower than Hopper's refreshed result. |
| Anza Pinocchio lazy entrypoint | Duplicate-account handling and typed validation remain caller responsibilities at the substrate boundary. A lazy parser alone is not a novelty claim. |
| Agave feature gates and callbacks | Check cluster activation separately from source support. Complete nested-CPI evidence needs more than a top-level execution callback. |
| SIMDs 0449, 0500, 0512, 0558 | Proposal labels and deployed features differ. The leader-info draft is not a shipped Hopper syscall. |

Anchor has a broad existing ecosystem. Pinocchio offers a small substrate for
manual program construction. Pina and Quasar provide other framework designs
worth evaluating against your account model, validation needs, and tooling.

## Measure the workload you ship

The clean September 24 Pina-recipe refresh measures Hopper's macro counter at
**349 CU for increment**, down from 358, with an **8,312-byte ELF**, down from
8,376. Initialization remains 1,549 CU. The rebuilt Pinocchio reference
reproduces the pinned published cross-check exactly.

Quasar's published increment is 330 CU. Hopper's substrate hello is the
smallest executable in that pinned hello table, while its macro hello uses
more CU than Quasar and Anchor. Account layouts, initialization bump policies,
and checks differ. The [benchmark guide](https://hopperzero.dev/docs/benchmarks) preserves the recipe,
source pins, and separate historical vault measurements.

## Use policy modes with their actual guarantees

`strict` records the program posture and binds typed contexts. The account
declarations and called APIs determine the checks. The token-policy flag does
not rewrite arbitrary CPI calls; strict token helpers perform their documented
checks. Ordinary Rust unsafe code still needs review.

`sealed` also emits `#[deny(unsafe_code)]` on handler bodies unless a handler
opts out with `unsafe_memory`. It does not ban unsafe implementations in
separately defined helpers or dependencies. `raw` permits manual context
handling; called accessors still enforce their own checks.

## Build on evidence

Cicada exercises settlement, route binding, authority checks, and rollback
against canonical token processors. It remains an application and regression
target, with production route integration and executable policy work ahead.
Grillo supplies a separate recomputation boundary; authenticated nested replay
is the next evidence problem to solve.

Choose Hopper for the contracts your program can use and test. The
[release record](https://hopperzero.dev/docs/release-status) separates published packages, host and
compiled tests, finalized devnet execution, and remaining independent review.
