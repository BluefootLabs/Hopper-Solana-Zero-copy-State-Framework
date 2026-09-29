# Devnet release evidence

Hopper's devnet release lane is a transaction test and an artifact identity
check. An executable program account alone is not enough evidence because it
may contain an older build.

The lane uses only `https://api.devnet.solana.com` and requires the canonical
devnet genesis hash. The example release-capture lanes require:

- a clean, committed source tree;
- a fresh program id controlled by an explicit devnet-only signer;
- the JSON deployment receipt with its program id and transaction signature;
- a local release ELF built from that source commit;
- a finalized transaction receipt written to `HOPPER_DEVNET_RECEIPT`;
- the loader ProgramData address, deployment slot, and upgrade authority;
- an on-chain program dump whose SHA-256 exactly matches the local ELF; and
- identical program metadata and bytes before and after the transaction test.

## Receipt contract

The example release runners write `hopper.devnet-evidence.v1` JSON. Common
fields are:

- `example` identifies the exact example lane;
- `commitment` is `finalized`;
- `cluster.genesis_hash`, `cluster.node_version`, and
  `cluster.feature_set` identify the RPC view;
- `program_id`, or `programs` for a multi-program lane, identifies the tested
  deployment; and
- `transactions` contains every signature and its finalized slot, including
  expected rejection cases.

Example-specific state fields record the assertions made by that runner. They
are evidence for those named behaviors only. They are not evidence that every
framework feature has been exercised on devnet.

## Canonical PDA fixture, 2026-09-23

The [focused PDA archive](../audit/canonical-pda-2026-09-23/README.md) uses
`hopper.canonical-pda-devnet.v1`, with its own explicit assertion scope.
Program `8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH` was deployed from
source `3d18085` at slot 503214552. All 25 transactions finalized with the
expected success or error and unchanged complete account snapshots. The
8,288-byte SBF v0 ELF matched on-chain dumps before and after execution.

The constant check used 71 CU; runtime canonical search used 1,074 CU.
Generated unchecked binding used 1,061 CU. Typed modes were validated in the
local SBF matrix, not this live run. The archive retains final loader metadata
but does not assert that upgrade authority remained unchanged throughout.

## Artifact capture

Run the capture script immediately before and after the test. The output
directory must be a new ignored directory, normally below
`target/hopper/devnet-evidence/`.

```powershell
pwsh scripts/capture-devnet-program-evidence.ps1 `
  -Phase Before `
  -Example hopper-migration `
  -ProgramId <FRESH_PROGRAM_ID> `
  -LocalElf target/hopper/release/hopper_migration.so `
  -KeypairPath target/hopper/devnet-release/payer.json `
  -DeploymentReceiptPath target/hopper/devnet-release/hopper-migration-deploy.json `
  -OutputDirectory target/hopper/devnet-evidence/hopper-migration `
  -SolanaCli D:\path\to\agave-v4.2.1\solana.exe

The script expects `solana-cli 4.2.1` by default; pass
`-ExpectedSolanaVersion 'solana-cli 2.3.13'` (or whichever pinned line you
deploy with) to run on another Agave CLI line. It reads `lastDeployedSlot`
from `program show`, or `lastDeploySlot` as solana-cli 2.x spells it. It also
requires a clean tree at both phases, so run the harness from a clean
worktree (`git worktree add C:\hwt <commit>`) while development continues in
the main checkout, and keep `HOPPER_DEVNET_RECEIPT` outside the capture
output directory, which the After phase copies the receipt into.

$env:HOPPER_DEVNET = '1'
$env:HOPPER_REQUIRE_DEVNET = '1'
$env:HOPPER_DEVNET_RECEIPT = 'target/hopper/devnet-evidence/hopper-migration-receipt.json'
$env:HOPPER_KEYPAIR = 'target/hopper/devnet-release/payer.json'
$env:HOPPER_MIGRATION_PROGRAM_ID = '<FRESH_PROGRAM_ID>'
cargo test -p hopper-migration --test devnet --locked -- --nocapture

pwsh scripts/capture-devnet-program-evidence.ps1 `
  -Phase After `
  -Example hopper-migration `
  -ProgramId <FRESH_PROGRAM_ID> `
  -LocalElf target/hopper/release/hopper_migration.so `
  -KeypairPath target/hopper/devnet-release/payer.json `
  -DeploymentReceiptPath target/hopper/devnet-release/hopper-migration-deploy.json `
  -OutputDirectory target/hopper/devnet-evidence/hopper-migration `
  -ReceiptPath target/hopper/devnet-evidence/hopper-migration-receipt.json `
  -SolanaCli D:\path\to\agave-v4.2.1\solana.exe
```

The final bundle contains both on-chain dumps, both `program show` records,
the local release ELF, the finalized receipt, a provenance record,
`SHA256SUMS`, and a hash of that checksum list. A release evidence archive is
valid only when it is bound to the final source commit and retained as a
content-addressed release asset.

## Runtime policy record: 2026-09-22

The later clean-source rerun at `ad1e209daacc5fbee5fa1a9da58b6df561d6badd`
passed the same 12 transactions using a newly rebuilt, byte-identical ELF.
Its [complete snapshot archive](../audit/devnet-evidence-2026-09-22/runtime-gate-ad1e209/README.md)
retains the before/after account JSON in addition to transaction responses and
receipt hashes. All six expected refusals left those snapshots unchanged.

The policy optimization was deployed separately as
`BPNYrNXCPJwV3k8txPVYWjbqTswkLxRGF1d2DzcGTHAX` and tested from clean source
`fefc94bc898cf1ceaae55c7b5abf0fceebac3044`. Twelve transactions finalized:
two account creations, four successful policy/lifetime cases, and six exact
policy refusals. Every refusal left the observed account snapshots unchanged.
Local and deployed ELF bytes matched before and after capture. The
[public receipt and transactions](../audit/devnet-evidence-2026-09-22/runtime-gate/README.md)
record the program, slots, errors, CU, and snapshot hashes. This is focused
runtime evidence; it does not replace a Cicada lifecycle or full release run.

## Earlier example lanes

Round 4 completed on 2026-09-22 UTC at source commit `03adc6d`: escrow
`ADjgLVeqM4t64bFuR5JKYhYw2JdsNcBWuQ3judJn7s4N` (deployment slot
502,250,553; five finalized transactions) and devnet-audit
`F42uSNm8WgnMoKWuKKtqEiKmeeayF9iVSD7NExzoDuJ8` (slot 502,250,352;
15 finalized transactions). Both bundles record a clean source tree and
matching local/before/after ELF hashes. The audit lane's three rejection
cases retained identical account snapshots. The complete public records
are in [the round-4 archive](../audit/devnet-evidence-2026-09-22/README.md).
This proves `03adc6d`, not the later gate-dispatch optimization.

Archived under
[`audit/devnet-evidence-2026-09-19/`](../audit/devnet-evidence-2026-09-19/)
without the ELF copies; every ELF hash is in each lane's `SHA256SUMS` and
`provenance.json`, and each bundle's `BUNDLE.SHA256` hashes that list. All
lanes used `https://api.devnet.solana.com`, genesis
`EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG`, node version 4.3.0-rc.0,
finalized commitment, and the devnet-only signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` as payer and upgrade
authority. The harnesses ran from a clean worktree at the commit each
`provenance.json` names, with `solana-cli 2.3.13` rather than the 4.2.1 the
scripts default to. Devnet had SIMD-0449 direct account pointers, Account
Data Direct Mapping, and SIMD-0460 active during the run, so these
transactions also executed Hopper's input parser under that configuration.

| Lane | Program id | Deployed slot | Transactions (finalized) | Result |
| --- | --- | --- | --- | --- |
| hopper-migration | `HeY6UKYJZiutNTZcUChgexf5qReSWFfHzsA5ehoT86fU` | 501,067,677 | initV1, migrateV1ToV2, depositV2 | pass, dump matches local ELF before and after |
| hopper-escrow | `7kDb5zSywMQZFH3uboVMtdgrczTYWFiDjxsiQD2zU5Jr` | 501,067,873 | make, wrong-maker-cancel (rejected, snapshot unchanged), authorized-cancel-close, funding and cleanup | pass |
| hopper-orderbook | `65u5AZMrqDU9UMdXhoSwUhHpF9GpfBh2g96AhSmgknoJ` | 501,068,074 | initBook, postBid | pass |
| hopper-compact-vault | `8ViiHe4yWWStzhrDsAAoKENDVHFVVn17ERYknXdwvUPc` | 501,068,324 | create-and-initialize, wrong-authority-deposit (rejected, snapshot unchanged), authorized-deposit, funding and cleanup | pass |
| hopper-token-2022-vault | `FJn8z851vZkpyiZnoAvfSEYYXG1mm4AG7txJEpr4bg8u` | 501,095,212 | initialize, prepare, mint, sweep (TransferChecked) | pass |
| cross-program-read | A `2h5zat7pKjUHH3jkgsuPGqQ2N2hyVb1cCmvZ5FGCnn8J`, B `EyMprn3Ur5ix47hei5iTztPx48FjJJ7EzH4AkMw6Fqei` | see receipt | program_a:init, program_a:deposit, program_b:read, program_b:min | pass, both dumps match local ELFs |
| sentinel authority gate | program `7N2pyj1zhn6HSt6A553xJaM5CLJdcLjZXvw9KtSQNmFx`, buffer `7faSVbUUco4dba21xBZgssCeV1TtN11owVWgXPcPmTQa` | 501,068,155 | none (read-only review) | WIDENED, exit 2 |
| hopper-devnet-audit | `EB6SZ7qTGerTuptmHjt6aGpZc1tpZDUPwiTBYpsbhs8M` | 501,098,319 | 15: initialize, three rejected negative cases with unchanged snapshots (wrong authority, too few remaining signers, read-only state mutation), rename, add-member, increment-segment, substrate, proof, token-policy, field-capability, and remaining-signer probes, read-audit, funding and cleanup | pass |
| hopper-escrow, round 2 (2026-09-21, commit `34d51dc`) | `J45ZZmwT54eoNX2uCbvQS6cjQCvs4aT4422pogDSvYWW` | 502,072,801 | make, wrong-maker-cancel (rejected, snapshot unchanged), authorized-cancel-close, funding and cleanup (slots 502,074,390 to 502,074,725) | pass, dump matches local ELF before and after; the build carries the live-rent `init`, the one-sha256 PDA checks, and the trimmed entry path |
| hopper-devnet-audit, round 2 (2026-09-21, commit `34d51dc`) | `D7LSG9suG3URCPMVVG2awRnjhSDMXkHHvbymr751xwVa` | 502,072,572 | the same 15 transactions as the first lane (slots 502,074,882 to 502,075,861) | pass; see `audit/devnet-evidence-2026-09-21/` |
| metadata records and transaction v1 (2026-09-21) | sentinel `7N2pyj1zhn6HSt6A553xJaM5CLJdcLjZXvw9KtSQNmFx` | none (no deployment) | `publish-security` inline Initialize (slot 502,011,159); `publish-manifest` Allocate, two Writes, Initialize (502,011,247 to 502,011,260); `tx send --v1` SPL Memo, version 1 envelope (502,011,279) | pass, both records read back and decoded; see `audit/devnet-evidence-2026-09-21/metadata-and-txv1/` |
| framework-comparison macro counter, round 3 (2026-09-21, commit `a91462c`) | `F4Um7PWsnZfN7y8WFzu1aPYJwqGduJTa4zuCGY9EUqMy` | 502,112,265 | wrong unsigned PDA (rejected at the creation CPI, `PrivilegeEscalation`), signing non-PDA (rejected, `InvalidSeeds`), initialize (1,572 CU), increment (368), re-initialize (rejected, `AccountAlreadyInitialized`), pre-funded initialize with a zero-delta payer (1,570), increment (slots 502,112,352 to 502,112,508) | pass, dump matches local ELF before and after, every CU equal to the Mollusk row; see `audit/devnet-evidence-2026-09-21/counter/` |
| hopper-escrow, round 4 (2026-09-21, commit `03adc6d`, the inline write-gate liveness test) | `ADjgLVeqM4t64bFuR5JKYhYw2JdsNcBWuQ3judJn7s4N` | 502,250,553 | make, wrong-maker-cancel (rejected, snapshot unchanged), authorized-cancel-close, funding and cleanup (slots 502,251,906 to 502,252,170) | pass, dump matches local ELF before and after; bundle `audit/devnet-evidence-2026-09-21/escrow-round4/` (SHA256SUMS hash `89dd56c6…`) |
| hopper-devnet-audit, round 4 (2026-09-21, commit `03adc6d`) | `F42uSNm8WgnMoKWuKKtqEiKmeeayF9iVSD7NExzoDuJ8` | 502,250,352 | the same 15 transactions as the first lane (slots 502,252,325 to 502,253,444) | pass; bundle `audit/devnet-evidence-2026-09-21/devnet-audit-round4/` (SHA256SUMS hash `04c60dcd…`) |
| hopper-devnet-audit, round 5 (2026-09-27, commit `c5317a6`: sysvar reads through `sol_get_sysvar`, the rent cache, the count-exact surplus refusal) | `79JhXff3YmDreZvM4QFeJUg4M6RBkvQ7tPn3s8CwepaK` | 505,024,230 | the same 15 transactions (slots 505,024,481 to 505,024,685), built and run from a clean worktree | pass, dump matches local ELF before and after; bundle `audit/devnet-evidence-2026-09-27/devnet-audit-round5/` (SHA256SUMS hash `624969b7…`) |
| framework-comparison macro counter, round 5 (2026-09-27, commit `c5317a6`) | `6LjqFgiBazYXDYadHeeXq8ff85MiaPDLiDd4J62aexCq` | 505,024,761 | the round-three steps (wrong unsigned PDA `PrivilegeEscalation`, signing non-PDA `InvalidSeeds`, initialize 1,542 CU, increment 348, re-initialize refused, pre-funded initialize 1,540, increment) plus `increment` with a surplus account, refused before the walk with `Custom(45057)` at 14 CU (slots 505,024,790 to 505,024,913) | pass, dump matches local ELF before and after, every CU equal to the Mollusk row for the commit; see `audit/devnet-evidence-2026-09-27/counter-round5/` |
| hopper-escrow, round 5 (2026-09-27, commit `c5317a6`) | `Bo9bpNC2SZyAiR8aJpX7QPaWEy1H7CHAtuEVJZbhAvg4` | see deploy receipt | none: the deployment landed, but the example's `devnet` test target was retired on 2026-09-26 in favor of `scripts/test-token-escrow-devnet.py`, which this round did not run | deployed, not exercised; the payer held 0.40 SOL afterwards |
| hopper-devnet-audit, round 7 (2026-09-28, commit `8c7db32`, the distinct-mutable-roles refusal in every context) | `98isXe2Hbhv6m95WQoj3h8QivCUt4zyLJsqd2hgbif3Y` | 505,055,197 | the same 15 transactions (slots 505,055,432 to 505,055,645), built and run from a clean worktree | pass, dump matches local ELF before and after; bundle `audit/devnet-evidence-2026-09-28/devnet-audit-round7/` (SHA256SUMS hash `0f035b56…`) |
| hopper-escrow, round 7 (2026-09-28, commit `8c7db32`) | `6zF1RrQ1kq4izFKBM5yGJj3GAz3qgPUscwUVmcocPjMJ` | see deploy receipt | `scripts/test-token-escrow-devnet.py`: setup, make refusals, both makes, reinitialize, stale-quote, unsigned-take, wrong-cancel-maker, cancel-trailing-data, wrong-vault-authority, and wrong-token-program refusals finalized with exact states; the runner then stopped at `wrong-payment-recipient`, which the program now refuses at bind with `Custom(45058)` (the taker's payment account in the maker's receiving role is one account in two mutable roles) where the runner expected the later `InvalidAccountData` | partial: the program refused earlier than the runner expected; the runner's expectation is updated and the full lane is re-run in round 8 |
| hopper-escrow, round 8 (2026-09-28, commit `fcc66e9`: the token builder parity batch) | `9vnEXTpz7TC2ugC5LEDLdJ6i8bBNEradv996hUwFSfPw` | 505,152,698 | `scripts/test-token-escrow-devnet.py` in full: 33 transactions (slots 505,152,759 to 505,153,308), the setup, both makes, ten refusals with unchanged snapshots including `wrong-payment-recipient` refused at bind with `Custom(45058)` at 241 CU, the two donations, `take` (8,707 CU) and `cancel` (4,122 CU) | pass, dump matches local ELF before and after, every expected state verified; bundle `audit/devnet-evidence-2026-09-28/escrow-round8/` (SHA256SUMS hash `6efd6e2e…`) |
| hopper-token-lab, round 8 (2026-09-28, commit `244b72c`: every SPL Token and Token-2022 builder) | `417akw6B2CcuTZFpePrZaSR5oyX3riBPtHmdePkjrAYJ` | 505,156,238 | `scripts/test-token-lab-devnet.py`: 42 transactions (slots 505,156,293 to 505,157,222); per program a plain mint, two immutable-owner accounts sized by `GetAccountDataSize`, `MintToChecked`, a `Batch` of two `TransferChecked`s (one token CPI, 2,472 CU on SPL Token and 5,575 on Token-2022), the UI-amount round trip, `WithdrawExcessLamports`, `InitializeMultisig2`; `UnwrapLamports` on a native account (4,475 CU); the eight plannable Token-2022 extensions alone and seven together (508-byte mint, 29,294 CU), the interest-plus-scaled pair refused by `MintPlan` at 439 CU; pause and resume, a multiplier update, and the scaled UI amounts `2.469134` and `3.703701` round-tripping to the raw amount | pass, dump matches local ELF before and after, every expected state verified; bundle `audit/devnet-evidence-2026-09-28/token-lab-round8/` (SHA256SUMS hash `3be86470…`) |
| hopper-tail-lab, round 10 (2026-09-28, commit `2ec0930`: value rules, in-place tail setters) | `2dihUAuNMBu23gnsRAx7qtwRgXtKSptEDWGotc3NjFoD` | 505,403,690 | `scripts/test-tail-lab-devnet.py`: 15 transactions (slots 505,403,705 to 505,403,903); `init_note` 2,185 CU, three `add_reviewer` in place at 628, 645, and 662 CU, a fifth reviewer refused, `rewrite_note` shorter at 1,016 CU, to its longest at 1,331, and with multi-byte text; an empty body `Custom(6700)`, a rewrite by another signer refused by `has_one`; `init_blob`, `write_blob` to the longest payload, and a tag outside the layout's `#[check]` rule refused with `Custom(6702)` on a write (374 CU, account unchanged) and at creation (no account left) | pass, every account equal to the runner's model byte for byte, dump matches local ELF (24,376 bytes) before and after; bundle `audit/devnet-evidence-2026-09-28/tail-lab-round10/` |
| hopper-token-lab, round 10 (2026-09-28, commit `2ec0930`: token metadata and token group) | `4MWp9iQM1qxLrz9sYj4jdf9BU4waEo58MEPs7R4j7m28` | 505,404,217 | `scripts/test-token-lab-devnet.py`: 49 transactions (slots 505,404,225 to 505,405,058); the round-eight lanes again, then a mint that is its own metadata account (`create_metadata_mint` 17,783 CU, 362 bytes, funded for exactly its size), a key set (12,144 CU) and replaced with multi-byte text, the token renamed, the key removed strictly and idempotently, the update authority given up (32,837 CU for the five CPIs), an update without an authority refused, `Emit` returning the stored entry each time; a group of at most three (`create_group` 15,425 CU) and a member of it (`create_group_member` 16,630 CU), the group's size reading 1 afterwards | pass, Token-2022 on devnet accepts the metadata and group interfaces, every TLV entry equal to the expected bytes, dump matches local ELF (78,136 bytes) before and after; bundle `audit/devnet-evidence-2026-09-28/token-lab-round10/` |
| framework-comparison macro counter, round 9 (2026-09-28, commit `2f25a78`: `init` stores the bump, in-place tail setters) | `Ax5Ntu8G7wrJ5D1tVK5qRnCdengWBhSCzL5h9hVjRqHV` | 505,186,917 | the round-six steps (slots 505,186,933 to 505,187,070): the two refusals, `initialize` 1,517 CU with the bump byte written by `init` (255 at offset 16), `increment` 348 CU through `bump = stored`, re-initialize refused, the pre-funded initialize 1,515 CU, the surplus-account refusal at 14 CU, the alias refusal at 99 CU | pass, dump matches local ELF (8,352 bytes) before and after, every CU equal to the Mollusk row for the commit; bundle `audit/devnet-evidence-2026-09-28/counter-round9/` |
| framework-comparison macro counter, round 6 (2026-09-27, commit `d527a3a`: one account in two mutable roles refused at bind) | `6yNZne9rJuXVKDwV1wFQv1zvK7msMgvFmPNM42Juu54e` | 505,032,516 | the round-five steps (initialize 1,544 CU, increment 348, surplus account `Custom(45057)` at 14 CU) plus `initialize` with the payer as both `authority` and `counter`, refused at bind with `Custom(45058)` at 99 CU before any signer, PDA, or rent check (slots 505,032,571 to 505,032,754) | pass, dump matches local ELF before and after, every CU equal to the Mollusk row for the commit; see `audit/devnet-evidence-2026-09-27/counter-round6/` |

The sentinel row is the ledger-bound upgrade review: v1 was built from the
committed source and deployed; v2 was built from the same source with one
line changed (`mut(paused, revision)` widened to include `fee_bps` and
`has_one = admin` removed) and written to a loader Buffer, never applied.
`hopper verify sentinel-v2.manifest.json --authority-baseline
sentinel-v1.manifest.json --baseline-program 7N2pyj... --candidate-buffer
7faSVb... --cluster devnet` bound both manifests to their on-chain
commitments and reported `write_range_widened` (gains `fee_bps`) on
`honest_pause` and `unpause`. The buffer is left in place so the review can be
re-run. The removed `has_one` was not reported in this run because the
manifests predate the `instructions` key on contexts; the shared-context
case is pinned in `grillo-manifest` tests since commit `1d3d136`.

The devnet-audit lane first failed to deploy: both the CLI and the devnet
loader rejected the artifact with `Unresolved symbol
(sol_remaining_compute_units)`. That syscall is SIMD-0049, which is
Withdrawn; its feature gate has never been activated on mainnet-beta,
devnet, or testnet, while every local validator enables it. The
compute-budget helpers that read it are now compiled for on-chain targets
only under the `remaining-compute-units-syscall` feature, the audit probe no
longer uses it, and the lane passed on the rebuilt artifact. The audit
receipt records the program's final state (counter 1, one substrate pass,
two remaining-signer checks, one proof, token-policy, and field-capability
check each, label `hopper-live`, one member).

Lanes found and fixed five harness defects on the way, all recorded in the
changelog: receipts that named their transaction list `signatures`, wrong
signers funded below the rent-exempt floor in three runners, a `1`-lamport
state expectation, a stale `MissingAccount` expectation where the program
returns `NotEnoughAccountKeys`, and the undeployable syscall reference.
