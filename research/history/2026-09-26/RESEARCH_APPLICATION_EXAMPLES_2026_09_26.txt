# Application examples: September 26, 2026

This bounded review inspected public repository inventories and selected source
files or guides. It did not execute peer tests or audit whole repositories.
Exact pins and fetched-file hashes are in
[the source manifest](../audit/application-positioning-2026-09-26/sources.json).

**Subsequent implementation:** the coverage table below records the state at
review time. The [funded escrow replacement](../examples/hopper-escrow/README.md)
now implements classic SPL Token custody and settlement with layout v2. See
its current validation evidence; the old state-only evidence remains historical.

## What peers actually show

| Project | Reviewed examples | Lesson for Hopper |
|---|---|---|
| [Quasar b0de7db](https://github.com/blueshift-gg/quasar/tree/b0de7db4cd271654a2dcf78807dd865e98e0b339/examples) | Escrow, multisig, vault; read escrow take/refund, multisig transfer, and vault deposit handlers | Small recognizable workflows are easy to navigate. Escrow take/refund actually call token transfers and account closure; multisig checks approvals before a signed SOL transfer. Pair a simple application with its state, instructions, client, and tests. |
| [Pina 478ae3d](https://github.com/pina-rs/pina/tree/478ae3d2811ea7f7869d2a32e6095a9727d4f9d8/examples) | Inventory, escrow/vesting/staking-rewards/prop-AMM guides, and vesting/reward claim handlers | Vesting and rewards are useful product-level starting points. Claim handlers perform signed token transfers after entitlement and account checks. Guides warn that missing SBF artifacts skip E2E paths; peer test results were not independently reproduced here. |
| [Anchor v2 038c193](https://github.com/otter-sec/anchor/tree/038c193a9eedd5ffd4aaf831d501a74a0c90084e/bench/programs) | v2 vault manifest/handler, multisig dispatch, prop-AMM handler; v2 test inventory | Vault and multisig provide recognizable benchmark workloads. The prop-AMM handler updates oracle state and rotates authority, with an assembly update path; its name does not establish a complete AMM. Legacy fixtures under `tests/` must not be relabeled as v2 examples. |

The fetched Quasar tree currently contains three application directories;
older cached web listings can include comparison vault directories. Pina's
inventory also contains counter, profile, todo, privacy-pool, role-registry,
and framework feature demonstrations. Inventory presence alone is not a
production-readiness or feature-correctness claim.

Pina's vesting claim checks beneficiary and mint links, the stored PDA bump,
the clock/cliff, cumulative entitlement, and vault balance before a signed
`TransferChecked`. Its reward claim advances reward debt and clears pending
rewards before payout, releasing account guards before CPI. Both inspected
paths reject mint extensions. These are concrete patterns for a Hopper claim
example, not evidence that any token extension works automatically.

Source also matters more than a guide's limitations list: the inspected reward
claim includes a pause check and outstanding-liability accounting even though
the guide still lists pause administration and solvency policy as out of scope.
This review does not establish the completeness of those mechanisms.

## Hopper's current coverage

| Product direction | Available source | Gap that must remain visible |
|---|---|---|
| Trading and settlement | Cicada custody/intent settlement; segmented order storage | Cicada production route integration and independent audit remain unfinished. Order storage has no matching, collateral, or settlement. |
| Token claims and airdrops | Account validation, token CPI, vesting math, distribution math | Helpers are not a funded claim lifecycle. Eligibility, replay protection, custody, token release, and campaign recovery need a complete application and adversarial tests. |
| NFT/cNFT marketplaces | NFT mint reference; Token Metadata CPI helpers | No shipped Bubblegum adapter or validated marketplace. Asset-specific authorization, proof handling, and atomic payment/transfer logic remain integration work. |
| First successful program | SOL vault; delegated application quotas | The vault moves SOL. Quotas track application credits, not SPL tokens. Preserve each example's exact evidence scope. |

The example index incorrectly described `hopper-escrow` as token escrow with
SPL integration, while its README and implementation describe state and close
semantics only. The index is corrected in this change.

## Where to invest next

1. A compact funded token-claim example would connect existing math and CPI
   helpers to a recognizable product. Require actual custody, recipient/mint
   checks, repeat-claim rejection, insufficient-funds rollback, and exact token
   balance assertions before calling it complete.
2. A small token-escrow lifecycle would make the route from basic accounts to
   real settlement easier than starting with Cicada. It should cover funding,
   taking, cancellation, and closure, with matched negative tests.
3. A cNFT marketplace needs a separately scoped Bubblegum integration. Begin
   with verified transfer/proof handling and atomic payment behavior; do not
   market an adapter or marketplace before those paths exist and are tested.

These are development priorities, not implemented features. The immediate
change is application-led copy and a guide that links available code while
making the missing integration work explicit. No performance results changed.
