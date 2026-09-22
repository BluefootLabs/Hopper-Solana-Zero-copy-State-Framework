# Cicada

Cicada is a transport-neutral protected-execution intent program built as
Hopper's first production-shaped flagship example.

It is designed to accept execution from any delivery path:

- normal RPC submission;
- direct TPU;
- Jito bundles;
- BAM application scheduling;
- Cicada Edge / future private relays;
- any later scheduler that lands an ordinary Solana transaction.

The transport decides **how the transaction lands**. Cicada decides **what the
transaction is allowed to accomplish**.

## V1 vertical slice

The first slice implements:

- global initialization and emergency pause;
- directly initializable, column-oriented intent shards;
- immutable user execution constraints;
- optional solver allowlisting;
- atomic permissionless execution, plus bounded reservations for explicitly allowlisted solvers;
- owner cancellation and atomic input refund;
- runtime-variable route CPI through Hopper `DynCpi`;
- exact route-envelope commitments or trusted-program mode;
- isolated owner-bound, per-source-vault PDA signing authority;
- a global source-lease PDA preventing one funded vault from backing intents in multiple shards;
- measured source/destination token deltas;
- source/destination token-account and mint-policy immutability checks;
- safe SPL Token and screened Token-2022 support;
- exact-cell settlement writes under `strict_writes`;
- successful-instruction touch maps;
- final record reclamation with source-token authority returned to the owner.

## Deployment initialization authority

The singleton config is not first-caller-wins. `initialize_config` verifies the
currently executing program account and permits only its live deployment
authority to initialize it:

- current loader-v3 deployments pass the executable program plus its exact
  ProgramData account, whose `Some(upgrade_authority)` must equal the signing
  payer.

The source retains a loader-v4-format authority branch and host test as
compatibility modeling. Loader v4 was abandoned and its program address was
burned, so that branch is not a live deployment target or a claim of current
loader-v4 support. Legacy-loader programs and immutable loader-v3 ProgramData
cannot initialize Cicada. Deployment tooling must initialize the config before
revoking the loader-v3 upgrade authority. This is enforced on chain and removes
the public mempool race in which an arbitrary signer could otherwise seize the
singleton config.
The emergency authority must also be nonzero because V1 has no authority
rotation path that could repair a permanently unreachable pause key.

## Why the state is column-oriented

`IntentShard` stores twenty records in one account, but authority is split by
column:

```text
immutable user domain
  owners[]
  source_tokens[]
  vault_authorities[]
  refund_tokens[]
  destination_tokens[]
  mints[]
  max_inputs[] / min_outputs[]
  expiries[]
  allowed_executors[]
  route_programs[] / route_commitments[] / route_modes[]

executor domain
  statuses[]
  claimants[]
  claim_expiries[]
  settled_inputs[] / settled_outputs[]
  settlement_hashes[]
  revisions[]
```

For example, `execute_intent` publishes write access only to the executor
columns. `cells(slot; statuses, claimants, ...)` compiles each column's base,
stride, cell width, and count into a parametric policy tied to the decoded
`slot` argument. Hopper grants only the selected cell; a neighboring record in
the same statically declared column is refused before a mutable lease exists.

This is not merely a storage optimization. It means a compromised solver path
cannot safely acquire mutable access to the user's route, limits, vault,
expiry, or owner fields.


## Global source-vault uniqueness

Shard-local scans are not enough: the same token account could otherwise back
intents in two different shards. `create_intent` therefore initializes a small
marker PDA at:

```text
[b"cicada-source", source_token_address]
```

The marker binds the source account to the shard, slot, sequence, and owner. A
second intent using that vault cannot initialize its marker. Reclaim checks the
marker slot and sequence against the live record, so a stale marker cannot release
a reused shard slot. After settlement or cancellation, only the intent owner may
reclaim the record. Because anyone can credit a token account, reclaim permits a
post-finalization balance and returns that balance with the original source
account when it restores authority from the Cicada PDA to the user. It verifies
the authority change from account data, clears the record, and closes the marker
back to that user.

## Isolated signer capability

Cicada deliberately does **not** sign arbitrary route CPIs with the global
configuration PDA.

Each source token account has an authority PDA bound to both the intent owner and the source account:

```text
[b"cicada-vault", owner_address, source_token_address]
```

The owner prepares and funds a token account that is still controlled by the
owner. `create_intent` rejects a source with a delegate, delegated balance, close
authority, frozen state, or unsupported Token-2022 extension, then atomically
moves its token authority to this PDA through the canonical token program. Any
later create failure rolls that authority change back with the instruction.
Binding the PDA to the owner prevents an abandoned or later-refilled vault from
being adopted by a different user after the previous record is reclaimed.

A route therefore receives signer authority over one isolated source vault,
not every Cicada vault. The route-account validator also refuses:

- Cicada-owned state;
- the shard or config account;
- writable access to the user's refund account;
- another token account owned by the same vault authority;
- signer escalation for any PDA other than the committed vault authority;
- writable access to the vault-authority PDA itself.

## Route policies

### Exact envelope

`ROUTE_MODE_EXACT` commits to:

```text
route program
route instruction bytes
ordered account addresses
ordered writable/signer flags
account duplicates
```

Changing one account, privilege, duplicate position, or data byte changes the
route commitment. The commitment identifies the call envelope, not the target
program's deployed bytecode; an upgradeable route program can still change
behavior at the same address. A future production design should optionally bind
activation to a ledger-authenticated binary/deployment record that Grillo can
consume. Current Grillo checks supplied identity consistency and does not
establish ledger provenance.

Host clients can compute this wire value directly with
`compute_route_commitment_records` and `RouteCommitmentAccount`; both use the
same allocation-free core as the on-chain adapter. The helper accepts writable
and signer booleans rather than raw flag bytes, preserves order and duplicates,
and rejects more than 32 route records or more than 512 instruction-data bytes
because Cicada cannot execute those envelopes. Golden vectors pin the empty,
single-account, 8/9-account chunk boundary, duplicate, and reordered cases.

The host helper also refuses writable duplicates and conflicting duplicate
privileges, using execution's own alias rule. Read-only duplicates with
identical signer flags remain ordered, commitment-significant records. This
lets clients catch those unusable routes before publishing an intent. It is
a structural check; execution still verifies actual account ownership,
custody, mint policy, privileges, and token deltas.

Ordered duplicate accounts remain supported only when every occurrence is
read-only and uses identical signer flags. Solana unions privileges across
duplicate Pubkeys during CPI, while Hopper's safe deduplicated CPI tier rejects
repeated writable metas. Cicada therefore rejects both conflicting aliases and
all writable aliases before CPI instead of accepting an envelope that is unsafe
or cannot execute.

### Trusted program

`ROUTE_MODE_PROGRAM` fixes the route program while allowing the solver to
choose its instruction and accounts. Its commitment argument must be all zero,
so clients cannot disagree about an unused field. This mode remains bounded by:

- per-vault signer isolation;
- protected Cicada accounts;
- immutable token-account policy bytes;
- maximum input;
- minimum output;
- actual balance deltas;
- atomic refund of unused input.

It should be used only when the intent creator trusts the selected route
program's behavior.


## Claims without permissionless griefing

A claim is an optional reservation for an intent that already names an
`allowed_executor`. Only that executor may acquire the lease. Permissionless
intents cannot be pre-claimed: any solver executes them atomically from
`STATUS_OPEN`. This removes the repeat-claim censorship vector and maps cleanly
to BAM, Jito bundles, direct TPU, and normal RPC submission.

An allowlisted executor may also execute directly from `STATUS_OPEN`; taking a
lease is useful only when its off-chain workflow needs a short reservation.

## Token settlement guarantees

Before route CPI, Cicada records:

- source amount;
- destination amount;
- source and destination lamports plus each account's wrapped-SOL status;
- a hash of every source token-account byte except `amount`;
- a hash of every destination token-account byte except `amount`;
- hashes of every byte in both mint accounts, including `supply`.

The route may duplicate either committed mint only as read-only. Cicada rejects
any writable input-mint or output-mint alias before CPI, preventing a route
from hiding a supply-neutral mint/burn sequence behind an unchanged end state.

After CPI it requires:

```text
source token policy hash unchanged
destination token policy hash unchanged
input mint policy hash unchanged
output mint policy hash unchanged
spent = pre_source - post_source
received = post_destination - pre_destination
spent > 0
received > 0
spent <= max_input
received >= min_output
non-native source lamports >= their pre-route value
native source lamports >= pre-route lamports - spent
non-native destination lamports >= their pre-route value
native destination lamports >= pre-route lamports + received
```

The lamport floors prevent a signer-capable route from closing and recreating a
byte-identical token account while diverting its rent or excess SOL. Native SPL
accounts use amount-adjusted floors because canonical wrapped-SOL transfers
move the corresponding lamports with the token amount.

It then refunds the complete remaining source balance and refuses to settle
unless the source vault is empty.

That zero-balance postcondition is proved when the record becomes final. A
third party may deposit tokens after settlement or cancellation, but cannot use
that public credit path to pin the record. SPL `SetAuthority(AccountOwner)` is
balance-independent, so reclaim restores the original owner with any late
tokens still in the original source account and releases the source lease
without another transfer dependency.

All admitted token accounts must be exactly initialized, not frozen, and have
no separate close authority. This removes an additional actor that could delete
an empty refund or destination account and block the lifecycle. Cicada
walks the complete Token-2022 TLV envelope, rejects malformed, duplicate,
wrong-account-shape, unknown, and not-yet-reviewed extension types, and applies a
narrow explicit allowlist. V1 accepts base Token-2022 assets plus metadata-only
mint policy. It rejects transfer fees, permanent delegates, confidential
transfer, non-transferable tokens, transfer hooks, scaled UI amounts, pausable
tokens, permissioned burns, and other extensions that change transfer or
settlement semantics. A custody source also rejects `ImmutableOwner` and enabled
`CpiGuard` or `MemoTransfer`, because those policies can prevent authority
restoration or the mandatory refund CPI.

Writable-mint containment makes minting or burning during the route
unsupported, even when a balancing operation would restore the final supply.
The full mint hash remains defense in depth for persistent mint changes.
Token-2022 support is therefore fail closed: a new extension remains
unsupported until its wire layout and lifecycle effects are reviewed and added
deliberately.

Legacy SPL mints and token accounts must use their exact canonical 82-byte and
165-byte shapes. Token-2022 rejects the canonical 355-byte multisig collision
before applying any mint or token-account overlay. These shape gates prevent a
multisig body with attacker-selected signer bytes from being admitted as an
immutable refund or destination that canonical `TransferChecked` would later
refuse.

Cicada does not remove issuer authority risk between lifecycle instructions. A
legacy mint or freeze authority can change supply or freeze an account after
intent creation; a frozen source or refund cannot progress until the issuer
thaws it. During route CPI, writable-mint containment prevents the route from
changing either committed mint, while full mint hashing checks the end state as
defense in depth. Neither control applies between lifecycle instructions.
Deployments that require stronger asset trust should allowlist mints with
revoked authorities or add a creation-time mint-policy commitment.

## Mutation-contract boundary

The Hopper manifest declares Cicada-owned state writes and fixed
source/refund/destination account roles. A generic route may also write its own
dynamic remaining accounts, so `execute_intent` is intentionally not a complete
description of every downstream program effect.

That boundary is also why the current fail-closed Solana IDL v0.1 exporter
refuses Cicada: Hopper's u16-prefixed bounded `route_data` and the dynamic
remaining-account contract cannot be represented losslessly. Use the Hopper
manifest and Hopper-aware generated clients; do not advertise a Solana IDL for
this program.

The security statement for V1 is narrower and explicit:

> Cicada byte-governs its own shared state, isolates its signer capability,
> protects committed token-account and mint policy, and verifies the user's economic
> result. It does not claim to describe every internal state change made by the
> selected route program.

A later Grillo validator/RPC integration should capture the full transaction
account envelope and attribute observed downstream effects separately.

## Compiled lifecycle proof

The current SVM suite runs 23 compiled tests against three repository-built
ELFs: Cicada, the deliberately hostile route/token fixture, and a separate
canonical route fixture that never mutates token-owned data directly. Mollusk
also registers its vendored canonical SPL Token and Token-2022 processors.

The suite proves authorized initialization, atomic custody adoption, create,
claim, two-leg route execution, unused-input refund, cancellation, and reclaim.
Both legacy SPL Token and extension-free Token-2022 complete the real
execute/refund/reclaim path through their canonical processors. Compiled
negative cases cover unauthorized and finalized-deployment initialization,
noncanonical token-account shapes, token-policy mutation, writable-mint
delegation, underpayment, output without input, protected-account delegation,
stored-bump tampering, and complete SVM rollback. Conflicting privileges on
duplicate route Pubkeys are also rejected. Real canonical transfers also prove
that post-final dust cannot block reclaim after settlement or cancellation:
authority returns to the owner with the dust still in the original source.
The native-account matrix uses each processor's canonical wrapped-SOL mint and
proves exact source debit, destination credit, and refund lamport coupling. A
modeled close/reinitialize shortfall is rejected and rolled back atomically.
Another 25 host tests cover policy parsing, manifest behavior, client/on-chain
route-commitment parity, and refusal to reuse the terminal `u64::MAX` revision.

CI sets `HOPPER_REQUIRE_CICADA_SBF=1`, so a missing Cicada or route ELF fails
the job instead of turning compiled coverage into a skipped success. These are
deterministic SVM proofs, not a substitute for devnet evidence or an external
security audit.

## Current deployment-cost diagnostic

The 2026-09-06 build from the current working tree produced two isolated
rebuilds that matched `target/deploy/hopper_cicada.so`, three byte-identical
copies total, under `cargo-build-sbf 4.1.0` / platform-tools 1.54
(`sha256:7ee1247f704b6feb42cfc499b9bcdb30b79b4f83bc4de599cbe389b685c2defb`).
Strict release verification found the expected interface commitment and all
three layout anchors. The tree was dirty, so this is a reproducible diagnostic,
not a clean-commit release attestation.

At Mainnet slot 444,767,908, a fresh default loader-v3 allocation for that
artifact locks **1.053057573 SOL** in the Program and ProgramData accounts.
One Config plus one 20-slot Shard brings the minimally usable instance to
**1.112891757 SOL**. Each live SourceLease adds **0.001621248 SOL**, refundable
on reclaim. Transaction and optional priority fees are extra.

The stock Solana CLI temporarily funds its Buffer at the ProgramData reserve
(1.052018961 SOL for this build) and loader v3 recycles that balance into
ProgramData during a successful deploy. Do not add it again to the permanent
total. Re-run `hopper deploy --dry-run --no-build -p hopper-cicada --cluster
mainnet-beta` at deployment time; rent is live cluster state, not a constant.

## Build and test

```bash
cargo build-sbf --manifest-path examples/hopper-cicada/Cargo.toml -- --locked
cargo build-sbf --manifest-path examples/hopper-cicada-route-fixture/Cargo.toml -- --locked
cargo build-sbf --manifest-path examples/hopper-cicada-canonical-route-fixture/Cargo.toml -- --locked
HOPPER_REQUIRE_CICADA_SBF=1 cargo test -p hopper-cicada
cargo test -p hopper-cicada-canonical-route-fixture
hopper lint --project examples/hopper-cicada --deny-escapes
```

The detailed threat model, account lifecycle, and prioritized next work are in
[ARCHITECTURE.md](ARCHITECTURE.md).

The program is a production-shaped vertical slice, not an audited mainnet
release. Before custody or significant value, archive devnet execution evidence
against deployed canonical token programs and complete an independent audit
focused on initialization, route delegation, and token settlement.
