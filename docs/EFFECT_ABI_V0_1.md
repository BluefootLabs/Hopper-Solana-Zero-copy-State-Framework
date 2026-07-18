# Solana Program Effect ABI v0.1

Status: implemented experimental profile. The semantics are framework-neutral;
the shipped wire carrier is Hopper's `hopper.manifest.json`, Hopper is the
reference producer/runtime, and Grillo is the reference resolver/verifier.
This document describes what the current code proves, not a claim of Solana
Foundation adoption or a finalized network standard.

## Purpose

The Effect ABI publishes a machine-readable upper bound on one instruction's
authorized state mutation. It can narrow a static account-data range to the
single fixed-size cell selected by the concrete instruction invocation. An
independent verifier can then compare the published authorization with
pre/post account snapshots and the instruction's write-touch map.

The v0.1 verdict is:

```text
changed data bytes ⊆ acquired write bytes ⊆ authorized data bytes
```

When the producer also declares the lamport dimension complete, the verifier
additionally checks that every observed lamport delta targets an authorized
account.

## Coordinate system

- Accounts are identified by their zero-based position in the instruction's
  account list (`accountIndex: u8`).
- Data ranges are half-open intervals `[offset, offset + size)` using `u32`
  offsets and sizes. Consumers widen arithmetic before adding them.
- Argument descriptors are ordered in wire-decoding order.
- The resolver accepts the argument payload *after* the instruction
  discriminator. The caller that selected the instruction is responsible for
  removing the correct discriminator length.

The manifest's one-byte `tag` identifies the Hopper instruction. It is not a
claim that imported or future ABIs can only use one-byte discriminators.

## Current wire carrier

v0.1 is carried inside the existing Hopper manifest rather than a standalone
file format. The effect-bearing subset is:

```json
{
  "name": "program-name",
  "version": "program-version",
  "instructions": [{
    "name": "instruction-name",
    "tag": 1,
    "strictWrites": true,
    "args": [{
      "name": "slot",
      "type": "u16",
      "size": 2,
      "encoding": "fixed"
    }],
    "accounts": [{
      "name": "state",
      "writable": true,
      "signer": false
    }],
    "writeRanges": [],
    "parametricWriteRanges": [],
    "mutationComplete": true,
    "lamportAccounts": []
  }]
}
```

`mutationComplete` and `lamportAccounts` are emitted only for a complete
lamport declaration. Argument `encoding` is `fixed`, `boundedString`, or
`boundedVec`; bounded encodings also carry `maxLen`, and bounded vectors carry
`elementSize`.

The carrier does not yet contain an explicit Effect ABI format-version field.
The v0.1 label names this specification; the reference parser structurally
requires `strictWrites`, while hash compatibility is separated by the
versioned commitment domains below. A standalone cross-framework carrier is a
future format, not a shipped claim.

## Static write authorization

For an instruction with `strictWrites: true`, `writeRanges` is the published
data-write authorization. Each entry has this shape:

```json
{ "accountIndex": 1, "offset": 64, "size": 8 }
```

The authorization is the union of the ranges for each account. An empty list
is a valid deny-all data-write contract. When `strictWrites` is false, the
ranges carry no completeness claim and Grillo returns `INCONCLUSIVE`.

In the Hopper reference runtime, this policy gates Context-mediated write
acquisition before a mutable lease is granted. A single acquisition must fit
inside one declared range; the verifier uses union coverage because a complete
touch map may coalesce several separately gated acquisitions.

## Invocation-parametric exact cells

A `parametricWriteRanges` entry has these fields:

```json
{
  "accountIndex": 1,
  "baseOffset": 64,
  "stride": 16,
  "cellSize": 8,
  "count": 32,
  "argumentIndex": 0,
  "argument": "slot",
  "segment": "balances"
}
```

For selector value `x`, the rule defines:

```text
envelope = [baseOffset,
            baseOffset + stride * (count - 1) + cellSize)

selected = [baseOffset + stride * x,
            baseOffset + stride * x + cellSize)
```

The effective authorization for an invocation is:

```text
(static authorization - every parametric envelope)
  union every invocation-selected cell
```

This subtraction is essential: the static range is a conservative column
envelope, not authority to mutate a neighboring cell.

The reference resolver fails closed unless all of the following hold:

- `count > 0`, `cellSize > 0`, and, for multiple cells,
  `stride >= cellSize`;
- offset arithmetic fits the `u32` account address space;
- every envelope is fully covered by the static authorization;
- envelopes on the same account do not overlap;
- each selector name/index identity is consistent across rules; and
- the decoded selector is less than `count`.

`argumentIndex` is the compact index used by Hopper's runtime policy;
`argument` is the stable name used to locate the selector in the ordered wire
arguments. Exact-cell selectors are fixed little-endian `u8`, `u16`, or `u32`
values. Other selector types are rejected.

### Current variable-length decoding boundary

The manifest describes fixed arguments, bounded strings, and bounded vectors.
A bounded string has an exact `u16 length + bytes` encoding, so the resolver
can safely locate a later selector. A bounded vector's `elementSize` is a
maximum, not necessarily an exact element stride. Therefore v0.1 fails closed
when it must skip a bounded vector to locate a later selector. Arguments after
the final selector do not affect effect resolution and need not be skipped.

## Lamport authorization

`mutationComplete: true` means the producer declares both currently modeled
mutation dimensions: account data bytes and lamport balances.
`lamportAccounts` lists the positional account indices whose lamports may
change; an empty list denies all lamport changes. Grillo checks observed
pre/post balances only when `mutationComplete` is true. Absence of the field is
not interpreted as completeness.

The Hopper reference runtime enforces declared lamport permissions at its
supported lamport lifecycle and checked-CPI choke points. Direct substrate
access and unchecked CPI remain explicit escape surfaces and are not made safe
merely by publishing this ABI.

## Resolution certificates and commitments

The unresolved instruction commitment uses SHA-256 with domain
`grillo.mutation-contract.v2` plus instruction domain separation. It commits
to the instruction name/tag, strict/completeness flags, ordered argument wire
descriptors, static ranges, all parametric rule fields, and lamport
permissions. The whole-manifest commitment additionally commits to
instruction order. Program name/version and account display/role metadata are
not part of these mutation commitments.

After resolution, `ResolvedInstructionContract::commitment()` uses domain
`grillo.resolved-effect.v2` and commits to:

- the unresolved source commitment;
- the instruction name/tag and strict/completeness flags;
- the resolved lamport permission set;
- the decoded selector names, compact indices, and values; and
- the resulting effective ranges.

Changing a selector, its wire metadata, a rule, or an effective range changes
the certificate. v0.1 commitments are order-sensitive and are not a generic
JSON canonicalization scheme.

## Verification contract

The reference verifier consumes:

1. an instruction contract from the published manifest;
2. the post-discriminator argument payload when parametric rules exist;
3. pre/post data snapshots (and optional lamport balances) keyed by positional
   account index; and
4. a decoded, complete write-touch map for the instruction.

It reports byte-precise violations for changed bytes without a write touch,
write touches outside effective authorization, and unauthorized lamport
deltas. Acquiring an authorized range without changing it is legal and is
reported as scoped PASS evidence.

The verifier does not issue a PASS when:

- `strictWrites` is false;
- parametric rules have not been resolved for the invocation;
- the touch map reports overflow or skipped records; or
- argument/rule resolution fails.

Every PASS is explicitly scoped to the account data and lamport snapshots
listed in its evidence. An omitted lamport snapshot is distinct from an
observed `0 -> 0` balance. A transaction-level completeness claim additionally
requires the caller to supply snapshots for every relevant account and to
attribute the correct instruction touch map. Omitting an account from
`AccountDelta` means that account is not inspected; an empty scope is not a
transaction proof.

## Producer and consumer conformance

A conforming producer must emit the same policy it enforces, preserve
positional account and argument identities, and reject writes outside the
resolved authorization on every surface it advertises as governed. Emitting a
manifest without enforcing it is metadata compatibility, not Effect ABI
conformance.

A conforming consumer must resolve parametric rules before authorizing bytes,
validate all rule invariants above, use interval-union containment without
overflow, and fail closed on incomplete evidence. A consumer must not treat a
static parametric envelope as invocation authority.

## Explicit nonclaims and roadmap

v0.1 does **not** model or prove:

- read sets or read/write conflict freedom;
- owner, data-length, creation/deletion, executable, rent, or account-presence
  transitions;
- CPI targets, nested call effects, return data, logs, or a complete invocation
  frame;
- pattern-based effects over variable remaining accounts (only concrete
  positional indices emitted as ranges are representable);
- direct-account-mapping pointer validity, lifetimes, aliasing, or zero-copy
  memory safety;
- state-topology selection, automatic sharding, or hot-state placement;
- byte-level transaction scheduling. Solana locks writable accounts from the
  transaction account metadata, so disjoint cells inside one account do not
  permit concurrent transactions to write that account; or
- formal correctness of the producer, runtime, touch instrumentation, or
  program handler.

Planned extensions should use new versioned domains and explicit capability
flags rather than broadening v0.1 silently. Candidate dimensions are reads,
owner/length/presence transitions, CPI envelopes and full-frame effects,
remaining-account role patterns, direct-mapping conformance, and a topology
compiler that maps logical cells to accounts when account-level parallelism is
required.

## Reference implementation

- Manifest model, canonical commitment, and invocation resolver:
  [`grillo-manifest`](../crates/grillo-manifest)
- Snapshot/touch-map verifier: [`grillo-verifier`](../crates/grillo-verifier)
- Runtime write policy: [`hopper-runtime`](../crates/hopper-runtime)

## Solana context references

These sources motivate the ABI and roadmap; they do not imply endorsement of
this proposal:

- [Solana Ecosystem Security: A Shared Mission](https://solana.com/news/solana-ecosystem-security)
- [Anza's 2026 roadmap](https://www.anza.xyz/blog/anza26)
- [SIMD-0219: Stricter ABI and runtime constraints](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0219-stricter-abi-and-runtime-constraints.md)
- [SIMD-0449: Direct account pointers in program input](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0449-direct-account-pointers-in-program-input.md)
- [Solana transaction structure](https://solana.com/docs/core/transactions)
