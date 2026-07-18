# Hopper Loom (`hopper-topology`)

Hopper Loom is a host-side, profile-guided account-topology analyzer for
Solana programs. It consumes explicit correlated transaction classes and
compares current, vertical, horizontal, and hybrid placements using Solana's
account-level lock semantics.

The v0.1 library and JSON CLI are deliberately isolated from Hopper's on-chain
runtime. The JSON boundary can be produced by Hopper, another framework, an SVM
replay harness, or a carefully authored synthetic workload.

## What is exact

- Each transaction class retains its correlated logical reads and writes.
- For a candidate placement, Loom resolves those accesses to physical account
  identities.
- Two modeled transactions conflict exactly when one writes an account the
  other reads or writes.
- Weighted conflict probability is emitted as an integer rational, never a
  floating-point estimate.
- Candidate ordering, virtual-bucket routing, commitments, and Pareto
  dominance are deterministic.
- Cluster facts and project budgets are explicit input, not compiled-in
  assumptions.

`fixedAccountLocks` and `fixedMessageBytes` keep foreign/CPI overhead in
feasibility calculations. Because those fixed accounts do not carry identities
in v0.1, their conflicts are not included in the topology-controlled conflict
probability.

## Candidate model

- `current` preserves `currentAccount` groupings.
- `vertical` places each logical atom in its own account.
- `horizontal-N` preserves current groupings and routes explicitly shardable
  groups across `N` physical accounts.
- `hybrid-N` first separates atoms, then routes shardable atoms.

For a shardable atom, `sizeBytes` is bytes per fixed virtual bucket. For a
singleton atom it is the atom's total bytes. Horizontal duplication of a group
that mixes shardable and singleton atoms is rejected rather than guessed.

Fixed virtual buckets use rendezvous hashing. Adding the last physical shard
does not change any old shard's score, so a bucket stays on its old shard or
moves to the new one; it cannot jump between two existing shards. The profile's
bucket weights do not affect identity. They produce the reported single-bucket
collision floor, while actual candidate conflicts come from transaction-class
weights.

Loom emits every feasible candidate, every rejected candidate and reason, the
Pareto candidate IDs, and one named-policy choice. Policies are lexicographic:
`throughputFirst`, `capitalFirst`, or `minimalMigration`. There is no blended
score mixing probability, lamports, bytes, and account counts.

## Certificates and honest nonclaims

`Certified` means only that a complete confirmed-cluster profile produced the
committed, reproducible analysis. A complete deterministic replay receives
`ReplayValidated`; host and synthetic sources are downgraded. Any missing read,
write, CPI, remaining-account, or transaction-account coverage yields
`Incomplete`, and no feasible candidate yields `Infeasible`.

Loom does not claim:

- that byte-disjoint writes inside one account run concurrently (they do not);
- validator throughput, latency, scheduler ordering, priority fees, or future
  cluster limits;
- semantic correctness of a vertical split, sharded router, or migration;
- conflicts for foreign/fixed accounts whose identities are not modeled;
- that the routing-bucket floor applies to correlated multi-key transactions;
- formal verification or Solana Foundation endorsement.

Account data sizes are deterministic capacity calculations, not observed live
allocation. `migrationBytesUpperBound` conservatively treats every logical
byte as moved for every non-current plan.

## CLI

```text
hopper-topology validate profile.json
hopper-topology solve profile.json --out topology.plan.json
cat profile.json | hopper-topology solve - --compact
```

All input structs use Serde's `deny_unknown_fields`; misspelled or future fields
fail instead of silently changing the analysis.
