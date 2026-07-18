# Solana Program Effect ABI v0.2

Status: implemented experimental profile. The semantics are framework-neutral.
Hopper is the reference producer and runtime, and Grillo is the reference
binder and verifier. This document describes what the current code proves. It
is not a claim of Solana Foundation adoption or a finalized network standard.

v0.2 is the fulfillment of the v0.1 roadmap. Where
[v0.1](EFFECT_ABI_V0_1.md) publishes an upper bound on one instruction's
authorized data-byte and lamport writes, v0.2 publishes the complete
account-state transition contract for an instruction, binds it to a concrete
deployment, and verifies a concrete invocation frame against it. v0.2 is a
separate strict contract model, not a widening of the v0.1 manifest structs. A
consumer opts into the v0.2 parser explicitly; a v0.1 manifest is never
reinterpreted as a v0.2 contract.

## What v0.2 adds over v0.1

- Full per-account transition dimensions: data bytes, lamport direction, owner,
  data length, logical presence, and the executable bit.
- Deployment binding: a contract names the loader and executable it is valid
  for, so it cannot be applied to a different program build.
- A deterministic remaining-account grammar for the variadic account suffix.
- A complete nested-call (CPI) envelope with per-child account bindings and
  call-count bounds.
- A concrete invocation frame plus a fail-closed binding step, so evidence can
  only be verified against the contract it cryptographically names.
- A four-domain commitment chain that ties the published contract, the observed
  frame, the binding, and the verdict together.

## Relationship to the runtime and to v0.1

v0.2 is a verification and publication model. It does not replace the Hopper
runtime write policy, which remains the enforced choke point described in v0.1.
The intended relationship is that the same authorization the runtime enforces
is what a v0.2 contract publishes, and a v0.2 verdict checks an observed
invocation against that published contract after the fact. Emitting a contract
without enforcing it is metadata compatibility, not conformance.

## Contract model

A contract is one `EffectContractV2` document. All v0.2 structures use strict
decoding: unknown JSON fields are rejected, and every authoritative field is
required rather than defaulted.

```text
EffectContractV2
  effectAbiVersion : "0.2"            exact string, else UnsupportedVersion
  deployment       : DeploymentBindingV2
  instructions     : [InstructionEffectContractV2]   non-empty
```

### Deployment binding

```text
DeploymentBindingV2 =
  ExactDeployment { programId, loaderId, executableDigest }   all [u8;32]
  ExactArtifact   { executableDigest }                        [u8;32]
```

`ExactDeployment` pins a specific address, loader, and deployed executable
digest. `ExactArtifact` pins only the build artifact digest and permits any
address whose loader-canonical executable bytes match it.

### Instruction contract

```text
InstructionEffectContractV2
  name               : string, unique within the contract
  discriminatorBytes : [u8], length 1..=32, globally prefix-free
  completeness       : ContractCompletenessV2
  accounts           : [AccountRoleContractV2]        fixed positional roles
  remainingAccounts  : RemainingAccountsContractV2    variadic suffix grammar
  cpi                : CpiPolicyV2
```

`discriminatorBytes` is the exact dispatcher prefix, not merely a one-byte tag.
Discriminators are validated to be prefix-free across the whole contract, so no
instruction's prefix shadows another. The fixed roles plus the fully expanded
remaining grammar must total at most 256 accounts.

### Completeness

```text
ContractCompletenessV2 { accounts, data, lamports, owner,
                         dataLength, presence, executable, cpi }   all bool
```

Each flag is the producer's explicit claim that the corresponding dimension is
completely modeled. A claim of complete `accounts` requires a closed remaining
grammar (`complete` and not `allowTrailing`). A claim of complete `cpi`
requires a closed CPI envelope. The verifier issues a PASS only when every
contract dimension is complete; otherwise the verdict is inconclusive for the
first incomplete dimension.

### Account role and transition policy

```text
AccountRoleContractV2
  name       : string, unique within its role list
  signer     : Required | Forbidden | Allowed
  writable   : Required | Forbidden | Allowed
  address    : Any | Exact{address} | ProgramId
  transition : TransitionPolicyV2

TransitionPolicyV2
  data       : { ranges: [ {offset:u32, size:u32} ] }   half-open [offset, offset+size)
  lamports   : Preserve | MayChange | DebitOnly | CreditOnly
  owner      : Preserve | SetTo{target} | MayChange
  dataLength : Preserve | Exact{length} | Range{min,max} | MayChange
  presence   : Preserve | MustRemainPresent | MustCreate
             | MayCreate | MustClose | MayClose
  executable : Preserve | Set{value} | MayChange
```

`owner.SetTo.target` is `Exact{address}`, `ProgramId` (the executing program
from the frame), or `SystemProgram` (the all-zero address). A data range must
be non-empty and must not overflow the `u32` account address space.

Presence is a logical dimension observed as pre/post account existence. A close
tombstone is representable as a present, zeroed, System-owned post-state; it is
never inferred as absent from zero lamports or System ownership alone.

### Remaining-account grammar

```text
RemainingAccountsContractV2
  complete        : bool
  allowTrailing   : bool
  duplicatePolicy : DenyAll | DenyWritable | Allow
  groups          : [ RemainingGroupV2 { name, repeat:u16, roles:[AccountRoleContractV2] } ]
```

The grammar expands deterministically in invocation order: the fixed roles
first, then each group repeated `repeat` times, and within each iteration each
role in order. An expanded remaining role is named `group.role[iteration]`.
`duplicatePolicy` governs aliasing across the complete fixed-plus-remaining
account list.

### CPI policy

```text
CpiPolicyV2 =
  Forbidden
  Declared { complete: bool, calls: [CpiEnvelopeV2] }

CpiEnvelopeV2
  id                 : string, unique
  programId          : [u8;32]
  loaderId           : [u8;32]
  executableDigest   : [u8;32]
  discriminatorBytes : [u8], 1..=32
  minCalls, maxCalls : u16, minCalls <= maxCalls and maxCalls >= 1
  allowRollback      : bool
  allowExtraAccounts : bool
  accounts           : [ CpiAccountBindingV2 { childPosition, parentPosition,
                                               signer, writable } ]
```

Each `CpiAccountBindingV2` maps one child account position to one concrete
parent position with a privilege requirement. Within one `programId`, envelope
discriminators must be prefix-free. A `complete` CPI policy may not contain an
envelope with `allowExtraAccounts`.

## Commitments

All four commitments are SHA-256 over a canonical, domain-separated,
length-prefixed byte encoding. They are order-sensitive and are not a generic
JSON canonicalization scheme.

- Contract, domain `grillo.effect-contract.v0.2.c1`: the version, deployment
  binding, and every instruction contract field including transitions, grammar,
  and CPI envelopes.
- Frame, domain `grillo.invocation-frame.v0.2.c1`: the concrete invocation
  frame, including its untrusted provenance label.
- Bound, domain `grillo.bound-invocation.v0.2.c1`: the contract commitment and
  the frame commitment together.
- Verdict, domain `grillo.effect-verdict.v0.2.c1`: the bound commitment, the
  PASS marker, the changed data-byte count, and the sorted observed account set.

The contract commitment is computed only after validation. Program display
metadata that is not part of the authoritative contract does not affect it.
Changing any transition policy, discriminator byte, grammar entry, or CPI
binding changes the contract commitment, and therefore the bound and verdict
commitments derived from it.

## Invocation frame and binding

An `InvocationFrameV2` is caller-supplied evidence for one invocation and its
ordered CPI children:

```text
InvocationFrameV2
  network         : { genesisHash:[u8;32] }
  transaction     : { signature, messageHash, bankSlot,
                      outerInstructionIndex, invocationPath }
  deployment      : { programId, loaderId, executableDigest,
                      programdataAddress?, deploymentSlot?, manifestCommitment }
  instructionData : [u8]
  accounts        : [ {position, transactionIndex, pubkey, signer, writable} ]
  states          : [ AccountTransitionV2 { pubkey, pre, post } ]
  touch           : { complete, digest? }
  children        : [InvocationFrameV2]
  outcome         : Succeeded | Failed | RolledBack
  boundary        : InvocationEntryExit | TransactionPrePost | Unknown
  evidence        : EvidenceCompletenessV2
  provenance      : Fixture | RpcObserved | ReplayClaimed | ProviderClaimed
```

`AccountStateV2` is `Absent` or `Present { lamports, owner, executable, data }`.
Absence is explicit. When the `accounts` evidence dimension is claimed, the
frame carries exactly one state per unique account pubkey.

`bind_invocation_v2(contract, frame)` is the fail-closed binding step. It
validates the contract, validates the frame shape, and then requires all of the
following before it will construct a `BoundInvocationV2`:

- `frame.deployment.manifestCommitment` equals the contract commitment;
- the frame deployment identity matches the contract deployment binding
  (program id, loader id, and executable digest for `ExactDeployment`; the
  executable digest for `ExactArtifact`);
- the instruction data selects exactly one instruction by prefix;
- the concrete account list matches the deterministically expanded role list in
  count, position, effective privileges, and address constraints;
- duplicate accounts are consistent with `duplicatePolicy`;
- exactly one modeled state exists per role pubkey when the accounts dimension
  is claimed; and
- observed CPI children match the declared envelopes in identity, count bounds,
  account bindings, and rollback rules.

The manifest-commitment check is the proof-carrying property: a frame cannot be
verified against any contract it does not name by commitment.
`BoundInvocationV2` has private fields, so it can exist only as the output of a
successful bind.

Provenance is copied verbatim and never promoted. The binder and verifier
perform no signature, ledger, or replay validation, and every result reports
authenticity as `Unauthenticated`.

## Verdict

`verify_bound_invocation_v2(bound)` returns one of:

- `Inconclusive` when the invocation did not succeed, when any contract
  completeness dimension is false, when any evidence completeness dimension is
  false, or when nested CPI children are present under a declared CPI policy but
  the observation boundary is not invocation entry and exit;
- `Violation` with one precise entry per broken dimension; or
- `Pass` with contract, frame, bound, and verdict commitments, the untrusted
  provenance and unauthenticated authenticity, the sorted observed accounts, and
  the changed data-byte count.

For each expanded role, the verifier checks the six transition dimensions
independently:

- presence, against the presence policy;
- lamport direction, against `Preserve`, `DebitOnly`, `CreditOnly`, or
  `MayChange`;
- owner, against `Preserve`, `SetTo`, or `MayChange`;
- data length, against `Preserve`, `Exact`, `Range`, or `MayChange`;
- executable, against `Preserve`, `Set`, or `MayChange`; and
- data bytes: every changed byte in the post-state that differs from the
  pre-state must fall inside a declared data range. Removed tail bytes are
  governed by the data-length dimension because no post-state byte exists there.

## Proof-carrying state placement

The three Hopper analysis layers are threaded by one shared value, the manifest
commitment, so a single declaration drives runtime containment, independent
verification, and placement analysis.

1. Runtime containment. The Hopper runtime installs the instruction's write
   policy as an enforced ambient gate. This is the enforced dimension.
2. Independent verification. The published contract commits to that same
   authorization. A v0.2 invocation frame carries `manifestCommitment` and can
   be bound and verified only against the contract with that exact commitment.
3. Placement analysis. The topology analyzer's `WorkloadProfile` carries a
   required `manifestCommitment` field and emits a plan whose certificate
   commits to its normalized input and output. The placement recommendation is
   therefore attributable to a specific contract, not to an unlabeled workload.

No layer trusts another layer's summary. Each names the same commitment, and
each recomputes what it needs from primary evidence. The topology layer is
described in [hopper-topology](../crates/hopper-topology) and does not predict
validator throughput; it models account-level lock conflict for an explicit,
committed workload.

## Explicit nonclaims

v0.2 does not model or prove:

- authenticity of the invocation frame. Provenance is an untrusted label; the
  crate performs no signature, ledger, or replay validation, and reports
  `Unauthenticated` on every result;
- read sets or read/write conflict freedom for a single invocation;
- return data, logs, compute-unit consumption, or fee effects;
- rent-economic outcomes or account lifecycle beyond the modeled presence,
  length, owner, lamport, executable, and data dimensions;
- direct-account-mapping pointer validity, lifetimes, aliasing, or zero-copy
  memory safety;
- byte-level transaction scheduling. Solana locks writable accounts from the
  transaction account metadata, so disjoint byte ranges inside one account do
  not permit concurrent transactions to write that account; or
- formal correctness of the producer, runtime, touch instrumentation, or program
  handler.

## Reference implementation

- Contract model, validation, and commitment:
  [`grillo-manifest`](../crates/grillo-manifest) (`effect_v2` module).
- Invocation frame, fail-closed binding, and transition verifier:
  [`grillo-verifier`](../crates/grillo-verifier) (`frame_v2` and `verify_v2`
  modules).
- Enforced runtime write policy: [`hopper-runtime`](../crates/hopper-runtime).
- Workload-aware placement analyzer: [`hopper-topology`](../crates/hopper-topology).

## Solana context references

These sources motivate the ABI and roadmap. They do not imply endorsement of
this proposal:

- [SIMD-0219: Stricter ABI and runtime constraints](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0219-stricter-abi-and-runtime-constraints.md)
- [SIMD-0449: Direct account pointers in program input](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0449-direct-account-pointers-in-program-input.md)
- [Solana transaction structure](https://solana.com/docs/core/transactions)
