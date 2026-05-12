# Hopper Self Audit

**Audit date:** 2026-05-11  
**Scope:** `Hopper-Solana-Zero-copy-State-Framework` workspace: root framework
crate, runtime, core overlay layer, macro crates, companion crates, CLI, SDK,
schema/manager crates, examples, tests, and release documentation.  
**Method:** Source-level review of public APIs, unsafe boundaries, account
loading paths, dynamic-tail encoding, segment borrowing, CPI helpers, generated
macro output, CLI scaffolding, release gates, and example coverage.

This document is a code audit, not a market or framework-comparison document.
It records what the Hopper tree currently guarantees, what was remediated, and
what remains worth tracking before wider production use.

## Executive Summary

Hopper's core design is coherent: fixed-layout account state stays on a
validated zero-copy path, unsafe operations are concentrated behind named
runtime APIs, and higher-level macros lower to inspectable Rust. The strongest
parts of the tree are the typed account loading tiers, segment borrow registry,
runtime policy model, schema emission, and CLI verification flow.

The main audit risks are operational rather than architectural:

- Public documentation must stay synchronized with the published crate package
  name and feature model.
- New examples must compile in CI before they are presented as migration or
  starter paths.
- Unsafe APIs need complete invariants and regression tests whenever the
  surface changes.
- Benchmark and profiling claims must be tied to reproducible commands,
  lockfiles, raw logs, and exact toolchain versions.

## Implemented Remediations

The previous review produced ten concrete recommendations. The current tree has
implemented them in code or moved the relevant artifact to the appropriate
repository boundary.

### R1: Runtime Ownership Boundaries

The README now clarifies the division between `hopper-core` and
`hopper-runtime`. `hopper-core` owns account overlays, headers, collections,
and layout validation primitives. `hopper-runtime` owns account views, CPI,
context handling, the segment borrow registry, and execution-facing guards.

### R2: Benchmark Provenance

Release-facing claims now require same-provenance measurements. Cross-framework
or cross-implementation comparisons must live with the benchmark harness,
lockfile, raw logs, seed set, toolchain, runner version, and exact command
line. The framework README no longer treats unproven benchmark columns as
release facts.

### R3: Lazy Dispatch Benchmark Isolation

Lazy-dispatch benchmarking moved into the benchmark product boundary. The
benchmark target builds the same source under eager and lazy feature modes and
uses compile-time feature guards so measurements cannot accidentally mix modes.

### R4: Day-One Authoring Path

The public README leads with the proc-macro authoring path for new users and
keeps the declarative macro path documented for users who want lower-level
control or no proc macros.

### R5: POD Alignment Gates

Trybuild fixtures pin both sides of the POD alignment contract: alignment-1
wire types are accepted, and over-aligned user types are rejected before they
can become account overlays.

### R6: Token Extension Validation

Token-extension helper coverage now includes direct readers and validation
helpers for extension-bound program IDs. Tests cover happy path, missing
extension, truncated data, and authority/program mismatch cases.

### R7: Unsafe CPI Documentation

Unchecked CPI entry points now document explicit safety invariants, including
borrow aliasing, account list consistency, writable/signer coverage, duplicate
account discipline, instruction encoding, seed derivation, and seed lifetime.

### R8: Schema Interoperability Output

The schema crate and CLI can emit an ecosystem-compatible IDL representation
from Hopper manifests. The emitter follows the existing `core::fmt::Write`
pattern and keeps Hopper's manifest as the source of truth.

### R9: External Benchmark Targets

Benchmark-only implementations stay out of the framework workspace so their
dependency trees cannot collide with core framework crates. The sibling
benchmark repository owns those targets and the reproducibility envelope.

### R10: Unsafe Surface Inventory

`docs/UNSAFE_INVARIANTS.md` now acts as the primary inventory for unsafe entry
points and invariants. New unsafe APIs should update that document in the same
change that introduces or modifies the unsafe surface.

## Current Code Audit

### Runtime And Account Loading

The runtime account API has a clear tiering model:

- Validated whole-account loads check ownership, header, discriminator,
  version, and layout size before exposing typed views.
- Cross-program loads validate owner and layout fingerprint before exposing a
  read-only verified wrapper.
- Segment accessors register byte ranges with the borrow registry before
  yielding references.
- Unchecked escape hatches are named explicitly and documented as unsafe
  boundaries.

The segment borrow registry is one of the most important safety components. Its
tests cover overlapping read/write conflicts, adjacent ranges, guard release,
capacity, duplicate-account handling, and account separation.

### Dynamic Tail Encoding

Dynamic tail support is intentionally not zero-copy. Fixed account fields remain
zero-copy; tail payloads are explicitly length-prefixed and encoded through
`TailCodec`.

The current pass adds bounded helper types:

- `BoundedString<N>` stores a UTF-8 byte string with a `u16` length prefix.
- `BoundedVec<T, N>` stores up to `N` `TailCodec` elements with a `u16` count.
- `Address` implements `TailCodec`, enabling bounded address lists without a
  custom codec.
- `hopper_dynamic_tail!` generates small struct codecs from ordered fields.

Runtime tests now cover bounded string and bounded vector roundtrips in
addition to the existing primitive, option, length-prefix, truncation, and
excess-payload checks.

### Macro Surface

The proc-macro path validates layout shape at compile time and emits constants
for length, discriminator, version, layout ID, field offsets, absolute offsets,
schema metadata, dynamic-tail helpers, and typed load methods.

The declarative macro path remains available for users who need a no-proc-macro
surface. Both paths should continue to share the same runtime invariants.

### CLI And Scaffolding

The CLI can scaffold programs, generate manifests, inspect schemas, lint common
project issues, and profile ELF output. The current pass adds a bounded-tail
port scaffold path and expands the profiling reference for HTML flamegraphs,
baselines, and watch-mode usage.

CLI tests cover template rendering, package-name normalization, key rewriting,
manifest encoding, verification helpers, lint checks, profile output helpers,
and RPC manifest codecs.

### Examples And Documentation

Examples should remain CI-backed and should avoid acting as sketches. The new
bounded dynamic-tail example compiles as a workspace member and includes a unit
test for label/signer roundtripping through the generated tail helpers.

Release documentation now includes a warning-free lane. Launch-facing examples
should either compile without warnings or use narrow, documented `allow(...)`
attributes.

## Residual Risks

### RSK-1: Documentation Drift

Public docs must consistently use the published package alias:

```toml
hopper = { package = "hopper-lang", version = "0.1.0" }
```

Any future release should search for stale source-only install snippets before
publishing docs or crate READMEs.

### RSK-2: Unsafe Surface Growth

Any new unchecked API, raw pointer path, or manual account-byte operation must
update `docs/UNSAFE_INVARIANTS.md` and add focused tests around the boundary.

### RSK-3: Dynamic Tail Allocation

`TailCodec::MAX_ENCODED_LEN` is an upper bound. Programs that grow tails at
runtime must allocate enough account space before calling `tail_write`. Example
and scaffold code should make allocation constants obvious.

### RSK-4: Benchmark Reproducibility

Performance claims are only release-grade when tied to the benchmark product
repository, raw output, command line, exact dependency graph, and toolchain.

### RSK-5: Feature Matrix Coverage

The root crate supports multiple backend feature combinations. Release checks
should continue to exercise default features, explicit native backend,
compatibility backend, Solana-program backend, proc macros, and selected SPL
feature combinations.

## Verification Commands

Use these commands for a focused pre-release code-audit pass:

```sh
cargo test -p hopper-runtime --offline
cargo test -p hopper-cli --offline
cargo test -p hopper-vault --locked
```

For a broader gate, use the release checklist in
`docs/RELEASE_CHECKLIST.md` and run workspace checks in CI.

## Maintenance Rules

- Keep this audit focused on Hopper's own code, tests, docs, and release
  process.
- Put market positioning, comparisons, and benchmark narratives in separate
  non-audit documents.
- Keep every public example either compile-checked or clearly marked as a
  sketch.
- Record the exact verification commands used for each audit refresh.
- Treat documentation accuracy as part of the release surface, not as an
  afterthought.