# grillo-verifier

The independent **byte-diff verifier** for Hopper's behavioural contract:
given an instruction's pre/post account snapshots, its emitted touch map
(decoded from the `Program data:` log line), and the program's published
mutation manifest ([`grillo-manifest`](../grillo-manifest)), compute the
verdict

> **changed ⊆ acquired ⊆ authorized**

- `changed`: bytes that actually differ between the snapshots;
- `acquired`: bytes covered by a WRITE touch record (what the instruction
  told the runtime it was mutating);
- `authorized`: bytes the manifest's `writeRanges` permit.

Honest by construction:

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

Verified end-to-end against the deployed
[`hopper-sentinel`](../../examples/hopper-sentinel) showcase: the honest
pause PASSes with exactly its declared ranges; the tampered handler's
refused write never reaches the snapshots.

Not published to crates.io yet (`publish = false`); part of the Hopper
workspace.
