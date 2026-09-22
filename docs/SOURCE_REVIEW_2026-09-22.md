# Source review and next engineering targets, 2026-09-22

This review follows selected execution and evidence paths line by line.
It is not an exhaustive audit of every line in each repository. Source pins
below were checked against the public remotes on 2026-09-22; they are distinct
from the older pins behind published benchmark rows.

## Peer changes and what they imply for Hopper

| Source | Inspected path | Finding and Hopper response |
| --- | --- | --- |
| [Pina `1a3032c`](https://github.com/pina-rs/pina/tree/1a3032c7eda382e6580b80caad52ca21331b9bdb) | `crates/pina/src/traits.rs`, vesting/escrow account validation, and the delta from `b0106aa` | Pina removes redundant ATA derivations while retaining the check that pins a signing source vault. Its docs explain that the combined address-and-view loader pays once. Hopper should measure shared validation work and preserve the account relation each eliminated check proved. |
| [Anchor v2 `dbb5bae`](https://github.com/otter-sec/anchor/tree/dbb5bae417bfb7dd2ba67c1d1f4091c5216d7f4c) | `lang-v2/derive/src/pda.rs`, `parse.rs`, interface account-ID checks in `derive/src/lib.rs`, IDL conversion changes | Literal PDA precomputation already emits both address and canonical bump. New interface code rejects mismatched callee/account program IDs at compile time; current-to-legacy IDL conversion was removed. Hopper's supplied-bump `const_pda!` is narrower. Automatic canonical derivation remains an opportunity, not an invention claim. |
| [Quasar `b0de7db`](https://github.com/blueshift-gg/quasar/tree/b0de7db4cd271654a2dcf78807dd865e98e0b339) | Generated account parsing, validated-account PDA checks, and the previously reviewed comparison paths | The head is unchanged. Quasar combines account checks and uses SHA verification on validated accounts. Hopper has the same broad optimization opportunities; the counter table still records Quasar's lower increment CU and smaller macro artifact. |
| [Solana address implementation](https://docs.rs/solana-address/latest/src/solana_address/syscalls.rs.html) | Seed validation and bump derivation | The 16-seed limit includes the bump; each seed is at most 32 bytes. A hash-equivalent input outside that domain is not a valid signing seed list. |

## Fixes produced by the review

### One PDA seed domain across the runtime

`hopper-native/src/pda.rs` now validates seed count and per-seed length on
the SHA paths. Bump-appending helpers reserve the final slot. The const
helper rejects 16 base seeds instead of producing an address that cannot
be signed with that list. The runtime's older hash loop silently clamped
excess seeds; it now delegates to the fully inlined shared implementation.
The fallible canonical-bump check propagates malformed-seed errors on SBF
instead of going through the infallible search wrapper.

[The SBF fixture](../bench/pda-boundaries/README.md) drives 12 public paths
with dynamic input. Its seven vectors include invalid empty-seed padding
and a 33-byte seed with the same concatenated hash preimage as legal
seeds. Those inputs must return `InvalidSeeds` even when their hash matches.
The SDK supplies reference addresses and the test checks complete unchanged
account snapshots. No ownership, curve, or canonical-bump guarantee is
inferred from seed validation alone.

### Cicada catches unusable client envelopes before commitment

`compute_route_commitment_records` now applies execution's duplicate-meta
rule before hashing. Writable aliases and conflicting signer flags are
refused. Read-only duplicates with identical flags preserve their positions
and existing commitment bytes. A cross-product test covers every flag pair,
including duplicates separated by the eight-record hash chunk boundary.
Live ownership, mint, custody, and settlement checks remain execution work.

Cicada remains worth pursuing as a protected execution layer: bounded
per-vault signer authority, immutable user limits, observed token deltas,
and exact-cell settlement have uses independent of transaction delivery.
The next acceptance milestone should be a real route integration with
canonical token processors, adversarial rollback, and authenticated route
artifact policy. A transport label or a fast empty handler is not that milestone.

### Grillo refuses contradictory observation scope

The v0.1 bundle path previously admitted multiple snapshots for one account
index. Individually authorized but mutually incompatible pairs could receive
a scoped PASS, and duplicate pairs could inflate changed-byte counts. Bundle
verification now returns malformed input; the typed core returns INCONCLUSIVE.
Changed ranges are sorted by account and offset, as its API documentation
promises. Missing accounts remain explicitly outside the observation scope.

Grillo remains worth pursuing as an off-chain contract and effect checker.
Its immediate gap is authenticated input production. A replay adapter should
produce invocation-entry/exit snapshots, nested CPI boundaries, and artifact
identity for v0.2; acceptance must reject missing frames, mismatched binaries,
and transaction-wide snapshots that hide sibling writes. Existing verification
does not authenticate the caller-supplied evidence. No on-chain deployment or
SOL spending is needed for Grillo itself.

## Performance and publication discipline

The preceding gate-registration optimization reduced the macro counter ELF
from 10,056 to 8,304 bytes and the substrate counter from 8,256 to 6,728.
Those measurements and their compute tradeoffs remain in the
[earlier refresh](COMPETITIVE_REFRESH_2026-09-22.md). Seed-boundary changes
must be remeasured before treating those values as a later-source result.

The comparison driver now records source commit, clean-tree status, lockfile
SHA-256, and measured ELF hashes. `--require-clean` requires a build and refuses
`--skip-build`; every capture refuses source changes during the run. Historical
vault, current counter, and primitive measurements remain separate recipes.

Remaining performance targets are generated context setup, sharing parsing
with validation, and automatic canonical literal-PDA derivation. Each needs a
representative fixture, incorrect-input cases, and measured code-size/CU
tradeoffs. Hopper's useful distinction is the combination of direct state,
declared byte authority, runtime enforcement, and inspectable evidence.
The source and benchmark evidence do not establish universal fastest status.
