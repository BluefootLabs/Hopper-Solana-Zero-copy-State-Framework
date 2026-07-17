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
second intent using that vault cannot initialize its marker. After settlement or
cancellation, only the intent owner may reclaim the record. Reclaim first returns
the empty source token account's owner authority from the Cicada PDA to the user,
then clears the record and closes the marker back to that user.

## Isolated signer capability

Cicada deliberately does **not** sign arbitrary route CPIs with the global
configuration PDA.

Each source token account has an authority PDA bound to both the intent owner and the source account:

```text
[b"cicada-vault", owner_address, source_token_address]
```

The owner prepares a token account whose token authority is this PDA, then funds it before creating the intent. Binding the PDA to the owner prevents an abandoned or later-refilled vault from being adopted by a different user after the previous record is reclaimed.

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
behavior at the same address. A production deployment should optionally bind
activation to a Grillo-verified binary/deployment commitment.

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
- a hash of every source token-account byte except `amount`;
- a hash of every destination token-account byte except `amount`;
- hashes of both mint accounts excluding only the mutable `supply` field.

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
```

It then refunds the complete remaining source balance and refuses to settle
unless the source vault is empty.

Token-2022 mints are screened with Hopper's DeFi-safe extension policy. V1
rejects transfer fees, permanent delegates, confidential transfer,
non-transferable tokens, and transfer hooks because those extensions violate
the amount-only settlement model or require additional refund semantics.

## Mutation-contract boundary

The Hopper manifest fully describes Cicada-owned state writes and the declared
source/refund/destination account surfaces. A generic route may also write its
own dynamic remaining accounts, so `execute_intent` is intentionally not a
complete description of every downstream program effect.

The security statement for V1 is narrower and explicit:

> Cicada byte-governs its own shared state, isolates its signer capability,
> protects committed token-account and mint policy, and verifies the user's economic
> result. It does not claim to describe every internal state change made by the
> selected route program.

A later Grillo validator/RPC integration should capture the full transaction
account envelope and attribute observed downstream effects separately.

## Compiled lifecycle proof

The SVM suite loads two real SBF ELFs: Cicada and a deliberately adversarial
route/token fixture. It proves the initialize → create → claim → execute →
reclaim path, including custody restoration and sentinel-protected source-lease
close. Separate hostile routes attempt to mutate token policy and to credit
output without spending input; Cicada refuses both and the SVM rolls every
account back.

## Build and test

```bash
cargo build-sbf -- -p hopper-cicada
cargo build-sbf -- -p hopper-cicada-route-fixture
cargo test -p hopper-cicada
hopper lint --project examples/hopper-cicada --deny-escapes
```

The detailed threat model, account lifecycle, and prioritized next work are in
[ARCHITECTURE.md](ARCHITECTURE.md).

The program is a first vertical slice, not yet an audited mainnet release.
Before custody or significant value, add fuzzed account-envelope tests, devnet
execution evidence against the canonical SPL programs, and an external audit
focused on route delegation and token settlement.
