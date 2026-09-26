# Hopper positioning and claim review — September 26, 2026

## Lead with the application

Hopper is a zero-copy Solana program framework for Rust developers building
trading, token-claim, and marketplace logic with on-chain state and permissions.
The first page should answer what someone can build, how they start, and why
direct state access and explicit write limits help their application.

The homepage sequence is: application promise, use cases, ordinary Rust
authoring, state controls, working quota and settlement examples, optional
inspection, dated measurements, and installation. Technical names belong next
to the code or in the linked guide. Keep the dark visual style; simplify the
language rather than hiding limits or inventing performance claims.

## What the source comparison supports

This is a bounded authoring and onboarding review, not a whole-repository audit.
Primary sources were fetched September 25, 2026; exact hashes are in the
[review archive](../audit/use-case-review-2026-09-26/README.md).

| Project and reviewed pin | Reviewed scope | Useful lesson for Hopper |
|---|---|---|
| [Quasar b0de7db](https://github.com/blueshift-gg/quasar/tree/b0de7db4cd271654a2dcf78807dd865e98e0b339) | README and escrow make handler | Make the account/handler workflow immediate. Named escrow values and an explicit token transfer show application intent. Hopper's named inputs and checked CPI provide its own authoring path, without implying identical APIs. |
| [Pina 478ae3d](https://github.com/pina-rs/pina/tree/478ae3d2811ea7f7869d2a32e6095a9727d4f9d8) | README and counter program | Show setup and validation in context. The counter pairs account assertions with explicit creation/initialization; syntax alone does not establish safety or performance parity. |
| [Anchor v2 038c193](https://github.com/otter-sec/anchor/tree/038c193a9eedd5ffd4aaf831d501a74a0c90084e/lang-v2) | README and account/lifecycle traits | Migration and extension points are part of DX. Hopper's input trait is narrower than a general account-wrapper/lifecycle extension model. Keep that gap explicit. |

The [Quasar documentation](https://quasar-lang.com/docs) foregrounds installation,
quickstart, migration, clients, and local testing. Hopper should make its own
first successful build easy to reach. This is a presentation observation,
not a benchmark result or endorsement of every competitor claim.

## Claims and their boundaries

| Customer-facing statement | Source or evidence | Required boundary |
|---|---|---|
| Build a SOL vault | Vault implementation and 0.4 finalized devnet evidence | Deposit performs a real System transfer; changing a balance field alone is not custody. |
| Give delegates application quotas | Byte-allowance program and fresh 40-transaction 0.4 devnet run | Application units, not SPL token custody. Handler checks identity/revision/limit; Hopper restricts tracked mutable access. |
| Work with account state in place | Runtime checked loaders and typed layouts | Layout choices and access paths have different checks; raw/unsafe paths need review. |
| Keep selected fields outside an instruction's write grant | Context cell accessors and strict-write policy tests | Applies to Hopper-tracked access; no sub-account scheduling or automatic fee reduction. |
| Upgrade account state | Typed migration API and dated tests | Application supplies transforms; growth/shrinking and rent handling are explicit. |
| Inspect permission changes | CLI manifest/authority tools and Grillo | Optional off-chain inspection; no proof of arbitrary handler behavior or authenticated ledger replay. |

## Corrections made during this review

- Replaced the quick-start's balance-only deposit sketch with the actual
  authority-checked SOL transfer and System Program account.
- Corrected stale 0.3.1 onboarding text to the published 0.4.0 line.
- Matched the generated manifest's compatible version requirement; an exact
  version pin is an application release choice.
- Changed the website's new-project steps to install, scaffold, and build.
  The dependency-add command is labeled for an existing Rust project.
- Replaced hero microbenchmarks with application and authoring benefits.
  The dated benchmark table remains available with its original scope.
- Fixed bounded retry handling for transient read-only devnet RPC failures.
  Transaction submissions and unknown methods are never replayed by that helper.

## Release and verification scope

The runtime remains the published 0.4.0 release. Repository and website
documentation can be corrected independently; immutable crates.io archives
retain their publication-time README text until a subsequent crate release.
Do not describe these documentation edits as newly uploaded crate archives.

Fresh devnet validation exercised the published byte-allowance ELF. It does not
rerun every framework feature or the peer benchmark matrix. The placement
compiler and broader wrapper extensions remain proposals. Cicada is an
integration workload; Grillo is optional evidence tooling. Neither establishes
universal speed, safety, or production-readiness leadership.

## Application-example follow-up

Lead with trading and settlement, token claims and airdrops, and NFT/cNFT
markets. Link each to the [application guide](APPLICATION_USE_CASES.md).
Keep the working SOL vault and delegated quotas as introductory examples,
rather than treating quotas as a token airdrop.

The [peer example review](RESEARCH_APPLICATION_EXAMPLES_2026_09_26.md)
distinguishes Quasar application examples, Pina application guides, and Anchor
v2 benchmark fixtures. It also identifies Hopper's state-only escrow label
error in the example index, now corrected. Funding and token release are
implementation requirements, not benefits that can be inferred from a name.

Claims and cNFT markets are application directions with explicit integration
gaps. They are not newly shipped templates, newly measured benchmarks, or a
new crates.io release.

## Funded escrow and governance follow-up

The replacement escrow now implements classic SPL Token funding, atomic exchange,
cancellation, surplus refund, and account closure. It uses layout v2; the earlier
state-only deployment and evidence do not validate this version. Link its current
README and evidence rather than carrying forward the old no-token description.

Governance is a first-class application direction. The Squads review identifies
proposal, membership, spending, and execution requirements; it does not establish
that Hopper outperforms Squads. The bounded multisig example's duplicate-member
threshold bug is fixed, but it remains a data example, not a complete DAO.
