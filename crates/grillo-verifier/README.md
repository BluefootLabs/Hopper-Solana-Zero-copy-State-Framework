# grillo-verifier

A separately runnable offline effect verifier for Hopper's behavioral
contracts. Grillo is maintained in the Hopper workspace; "separate" describes
its recomputation boundary, not third-party organizational independence.

Grillo does not deploy to Solana and has no on-chain program account, so its
deployment principal is **0 SOL / not applicable**. It runs on an auditor,
indexer, CI worker, or replay host.

The stable v0.1 CLI takes an instruction's caller-supplied pre/post snapshots,
its emitted touch map (decoded from the `Program data:` log line), and the
program's mutation manifest
([`grillo-manifest`](https://crates.io/crates/grillo-manifest)), then recomputes

> **changed ⊆ acquired ⊆ authorized**

- `changed`: bytes that actually differ between the snapshots;
- `acquired`: bytes covered by a WRITE touch record (what the instruction
  told the runtime it was mutating);
- `authorized`: bytes the manifest's `writeRanges` permit.

Scope rules:

- **Acquired-but-unchanged is legal** (access is not modification); it is
  surfaced as a note on a scoped PASS, never a violation.
- Containment is judged per **byte against the union** of ranges, so a
  touch record coalesced under capacity pressure (the exact union of
  several individually-gated acquires) verifies identically to its parts.
- A **partial touch map** (overflowed/skipped) yields `INCONCLUSIVE`, never
  a false PASS. Rare by construction: the runtime coalesces exact unions
  under pressure, so overflow requires 33+ pairwise-unmergeable ranges in
  one instruction.
- The lamport dimension is checked when the contract is mutation-complete:
  an observed balance change on an undeclared account is a violation.
- Every PASS lists the exact account-data and lamport snapshot scope. Missing
  snapshots are never presented as transaction-complete evidence.

Violations carry byte-precise evidence (`UntrackedWrite`,
`UnauthorizedAcquisition`, `UnauthorizedLamportDelta`: account index,
offset, size).

See [`docs/EFFECT_ABI_V0_1.md`](../../docs/EFFECT_ABI_V0_1.md) for the
framework-neutral contract, invocation-parametric resolution rules, and the
limits of a v0.1 scoped PASS.

The library also implements the experimental strict
[`Effect ABI v0.2`](../../docs/EFFECT_ABI_V0_2.md). That path models data,
lamports, owner, length, presence, executable state, remaining-account grammar,
and nested CPI; binds a caller-supplied invocation frame to an exact artifact
or deployment identity; and commits the contract, frame, binding, and verdict.
It fails closed on impossible frame shape, account-index/pubkey conflicts,
future deployment slots, CPI context drift, invented child accounts, writable
privilege escalation, and runtime-informed resource limits. A v0.2 PASS
requires invocation-entry/exit snapshots. Transaction-wide snapshots are
inconclusive because sibling instructions could hide a forbidden mutation.

Binding is not authentication. The current verifier does not fetch a ledger
transaction, verify the claimed signature, prove who produced the frame, or
replay SVM execution. Every v0.2 result preserves the supplied provenance as
untrusted and reports authenticity as `Unauthenticated`.

Exercised end-to-end against the compiled-SBF
[`hopper-sentinel`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-sentinel) showcase: the honest
pause PASSes with exactly its declared ranges; the tampered handler's
refused write never reaches the local SVM snapshots. This is deterministic
test evidence, not public-cluster or independent-review evidence.

## The stable v0.1 `grillo` command

An indexer, auditor, or security desk can reproduce a
byte-precise verdict offline from a manifest and an evidence bundle. In the
v0.1 bundle, the snapshots, instruction name/payload, and touch map are
caller-supplied facts: Grillo recomputes containment but does not authenticate
their producer, verify a transaction signature, or bind them to a ledger
message or program deployment. Full supplied snapshot/lamport scope is also
required before a PASS can be described as transaction-complete. Build the
`grillo` binary from the Hopper workspace or install the exact 0.1.0 release
after it is indexed on crates.io:

```sh
cargo install grillo-verifier --version 0.1.0 --features cli --locked

grillo commit hopper.manifest.json             # per-instruction contract commitments
grillo verify hopper.manifest.json bundle.json # changed ⊆ acquired ⊆ authorized
```

Exit codes make it a CI gate: `0` scoped PASS, `2` VIOLATION, `3`
INCONCLUSIVE, `1` malformed input.

An evidence bundle is dependency-free JSON. It carries the post-discriminator
argument payload (hex, for parametric instructions), the program's emitted
touch-map blob (hex, as `hopper tx explain` prints it), and per-account
pre/post data (hex) with optional lamport pairs:

```json
{
  "instruction": "execute_intent",
  "argumentPayload": "0300",
  "touchMap": "7a0100...",
  "accounts": [ { "index": 2, "pre": "…", "post": "…" } ]
}
```

The verifier core (this crate without `--features cli`) stays pure
byte/interval arithmetic with a single dependency (`grillo-manifest`), so
embedding it in another tool never pulls serde. The bundle format and the
`grillo` binary live behind the `cli` feature.

## Package boundary

The core is no-network, no-RPC, and framework-neutral by design. It verifies
any producer that emits the corresponding contract, not only Hopper. The CLI
currently exposes the v0.1 JSON bundle workflow; v0.2 is a typed library API
while the ledger/replay producer is built. `grillo-manifest` must be indexed
before this package is verified or published.
