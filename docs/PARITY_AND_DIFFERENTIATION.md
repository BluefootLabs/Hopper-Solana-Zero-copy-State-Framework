# Parity and Differentiation

Where Hopper stands relative to other Solana zero-copy frameworks and
substrates.

This document covers concrete feature-level comparison, not marketing. Each row
is backed by code that exists today.

Important scope note:

- Anchor comparisons refer to Anchor's actual zero-copy path (`AccountLoader`),
	not just its default Borsh flow.
- Quasar comparisons count its public CLI, profiler, IDL, and generated client
	tooling.
- `solana-zero-copy` is a substrate crate, not a full peer framework.

## Current Audit Verdict

As of this pass, Hopper closes the two biggest internal consistency gaps that
used to weaken its parity claims:

1. instruction-scope duplicate writable aliases are now a first-class runtime
	audit surface and are enforced by `hopper_validate!` by default
2. macro-generated layouts now participate in the same
	`AccountView -> LayoutContract -> FieldMap -> SchemaExport` chain as the
	runtime's manual layout contracts

That materially improves Hopper's claim to being a coherent zero-copy runtime
rather than a loose collection of good ideas.

It still would be technically dishonest to say Hopper has surpassed Anchor,
Pinocchio, and Quasar in all areas.

- Hopper is ahead on versioned state contracts, schema evolution, runtime
	inspection, receipts, policy/lint semantics, and segmented state
- Quasar still has the stronger public profiler workflow, packaging polish,
	and more obvious day-to-day zero-copy DX in some paths
- Anchor still leads on ecosystem maturity, public adoption, and end-to-end
	client/IDL expectations
- Pinocchio remains the leanest raw substrate and still sets the baseline for
	minimal SDK surface

The correct claim today is: Hopper is differentiated and now internally more
consistent, with clear leadership in state-contract semantics, but not yet the
category winner in every workflow dimension.

## Feature Matrix

| Dimension | **Hopper** | **Anchor zero-copy** | **Pinocchio** | **Quasar** | **Jiminy** |
|---------|-----------|----------------------|---------------|-----------|-----------|
| Raw boundary ownership | Yes | No | Yes | Yes | Via Hopper Runtime |
| Zero-copy account access | Yes | `AccountLoader` | Yes | Yes | Yes |
| Layout/version/schema contract | **Full runtime contract** | Partial | Minimal | Partial | Strong |
| Foreign/versioned typed loads | **Yes** | No | No | No | Yes |
| Field maps + manager metadata | **Yes** | IDL only | No | IDL only | Partial |
| Segment roles / segmented state | **Yes** | No | No | No | Partial |
| Receipts / mutation proof | **Yes** | No | No | No | Yes |
| Policy / semantic linting | **Yes** | No | No | No | No |
| Public tooling / IDL / clients | Strong | **Very strong** | Minimal | **Strong** | Moderate |
| Backend portability | **3 backends** | solana-program | pinocchio | pinocchio | Hopper Runtime backends |

## What Hopper Does That Nobody Else Does

### Policy System

No other Solana framework has declarative capability-based policy enforcement.
Anchor has "constraints" (proc-macro-generated runtime checks), but those are
per-account, not per-instruction-capability.

Hopper's model:

1. Declare what an instruction does (`CapabilitySet`)
2. Declare what requirements those capabilities trigger (`InstructionPolicy`)
3. Resolve at const time to get the exact set of checks needed

This is compile-time policy resolution. The 9 named packs (treasury write,
journal touch, external call, authority change, account init/close, etc.)
cover standard patterns without user configuration.

### Borrow-Carried Typed Loads

Hopper's safe runtime load path now keeps the borrow alive all the way through
the typed projection. `AccountView::load`, `load_mut`, `overlay`, and
`overlay_mut` return Hopper-owned borrow guards instead of naked references.

That matters for two reasons:

- typed zero-copy refs cannot outlive the borrow that authorized them
- duplicate account handles with the same address now hit an address-keyed
	alias registry before Hopper hands out a mutable view

That runtime protection is now backed by an instruction-scope audit layer as
well. `AccountAudit`, `Context::require_unique_writable_accounts()`,
`TransactionConstraint::unique_writable()`, and the default
`hopper_validate!` path all reject duplicated writable aliases before handler
logic runs.

This is the exact middle ground Pinocchio does not try to provide: raw-speed
zero-copy with explicit runtime alias enforcement.

### Unified Runtime-to-Schema Bridge

Hopper's macro-generated layouts now implement the same runtime and schema
traits as hand-written runtime contracts:

- `FieldMap` for wire offsets and field inspection
- `LayoutContract` for discriminator, version, fingerprint, and typed loads
- `SchemaExport` for manager metadata and rich manifests

That means `AccountView::load::<T>()`, field inspection, manager metadata, and
schema export now all describe the same layout object instead of parallel,
partially disconnected systems.

### State Receipts

A 64-byte structured mutation proof that captures:

- Before/after fingerprints (8 bytes each)
- Changed field bitmask, byte count, region count
- Segment change mask (16 segments max)
- Policy flags (which capabilities were exercised)
- Phase tag (Init, Update, Close, Migrate, ReadOnly)
- Compatibility impact (None, Append, Migration, Breaking)
- Invariant pass/fail count
- CPI invocation count
- Journal append count

No other framework produces this. The CLI can decode these from transaction
logs and explain them in English.

### Phased Execution with Typestate

PhasedFrame uses Rust's type system to enforce instruction phase ordering:

```
PhasedFrame<Unresolved>  --.resolve()-->  ResolvedFrame
ResolvedFrame            --.validate()-->  ValidatedFrame
ValidatedFrame           --.execute()-->   Result<R>
```

You cannot call `.execute()` without first calling `.validate()`. The compiler
rejects it. Star Frame has a similar concept but ties it to proc macros.

### Segment Roles with Semantic Behavior

Seven typed roles (Core, Extension, Journal, Index, Cache, Audit, Shard), each
with distinct behavior properties:

- `should_emit_receipt()` -- does this segment warrant a mutation proof?
- `is_operator_relevant()` -- should operators monitor changes?
- `may_hold_financial_state()` -- could this segment contain treasury data?

The migration planner uses these roles to generate role-aware plans. Clearing a
Cache segment is safe; touching an Audit segment is never safe.

### Semantic Lint Engine

`lint_layout` and `lint_policy` inspect a layout's field intents, behavior
flags, and policy classification to catch design-level mistakes before deploy:

- E001: Authority field in a non-signer context
- W001: Monetary field without financial behavior flag
- W002: Init-only field in a mutable layout
- W003: Mutable layout without signer requirement
- W004: Balance behavior flag without monetary fields
- W005: Financial mutation class without financial policy class
- W006: Financial policy class without financial mutation class

This runs at compile time. No other framework has anything like it.

### Virtual State

`hopper_virtual!` maps a logical entity across multiple accounts with
slot-level ownership and writability constraints, then validates the entire
mapping in one call. Useful for orderbook/AMM patterns where state lives in
3-5 accounts.

### Layout Stability Grades

`LayoutStabilityGrade::compute()` inspects field intents and returns a
heuristic assessment: Stable, Evolving, MigrationSensitive, or
UnsafeToEvolve. Programs with heavy authority and financial surfaces get
flagged before they hit mainnet.

### Role-Aware Compatibility Refinement

`CompatibilityVerdict::refine_with_roles()` adjusts migration verdicts based
on segment semantics. A `MigrationRequired` verdict softens to `AppendSafe`
when all changed segments are clearable (Cache, Index). An `AppendSafe`
verdict escalates to `MigrationRequired` when immutable segments (Audit) are
touched.

## Where Hopper Matches Parity

### vs. Jiminy

Hopper and Jiminy share a common heritage in zero-copy overlay design. Both
use:

- 16-byte self-describing headers (disc, version, flags, layout_id)
- Deterministic layout fingerprints (SHA-256 of field descriptors)
- 5-tier loading (full, foreign, compatible, unchecked, unverified)
- Cross-program interface macros
- Wire-safe integer types with alignment of 1
- Schema export and migration planning
- State receipts with byte-level diffs

Hopper adds: policy system, phased typestate execution, segment roles,
semantic lint engine, layout stability grades, virtual state, program
manager, client SDK generation (TS + Kotlin), Codama projection.

Jiminy remains a close sibling with parallel domain crates and Anchor
interop, but Hopper now ships its own oracle/TWAP, vesting,
distribution, multisig, Token-2022 screening, and Anchor bridge crates.

### vs. Anchor

Anchor is the dominant public framework and still leads on ecosystem maturity,
generated workflows, and broad tooling. Hopper's lead over Anchor is narrower
but real: Hopper is stronger on layout/version/schema semantics, no_std/no_alloc
execution, segmented state, receipts, and manager-oriented metadata. Anchor's
actual zero-copy path is `AccountLoader<T>`, so comparisons against Anchor need
to distinguish that from the default Borsh account path.

### vs. Steel / Pinocchio (raw)

Steel is Pinocchio's overlay layer. It provides bytemuck Pod casts and account
loading. Hopper matches its performance (both are zero-copy) and adds:

- Versioned headers (Steel has no versioning)
- Typed fields (Steel uses raw bytemuck Pod)
- Schema diffing and migration
- Policy system, receipts, segment roles
- CLI tooling

Steel's unsound safety model (documented in its issue tracker) is a
differentiator in Hopper's favor. Hopper documents every unsafe block and
provides companion tests for each boundary.

### vs. Quasar

Quasar is stronger than earlier Hopper comparisons gave it credit for. It has a
real raw-boundary story, generated validation, public docs, CLI support,
profiling, IDL, and generated clients. Hopper leads on layout/runtime/schema
contract semantics, receipts, policies, segment roles, and backend portability.
The specific gaps Hopper used to have against Quasar are materially narrower
now:

- `hopper deploy` and `hopper dump` close the old build-only CLI hole
- manager commands and the interactive shell can now start from
	`--program-id` instead of a local manifest file
- cached PDA verification already exists through `verify_pda_cached`,
	`find_and_verify_pda`, and generated `BUMP_OFFSET` helpers
- `AccountView::extension_bytes(_mut)` and named segment collection adapters
	close the old raw-tail and vec-like access gaps

Quasar still has the more polished public profiler workflow and stronger
one-command packaging around that tooling.

### vs. Star Frame

Star Frame uses bytemuck Pod with a 4-phase lifecycle and typed CPI.
Hopper matches its typestate execution, exceeds it with 7 segment roles
(vs. none), 8 collections (vs. 3), receipts, policies, and CLI tooling.
Star Frame's published Miri story is still a real advantage. Hopper's answer is
stronger than it was before: documented unsafe inventory, header/layout smoke
tests, duplicate-writable CPI rejection tests, and a borrow-carried runtime
load path with address-keyed alias checks. The remaining gap is public proof
packaging, not a missing runtime model.

## Known Gaps

Hopper does not yet have:

1. **Public profiler UX** on par with Quasar's most polished profiling flow
2. **Higher-level typed tail adapters** for string/vec-style tails beyond the new raw extension byte API
3. **Independent benchmark replication and audit proof** for any blanket performance or safety claim
4. **Public CI/CD, release, and proof lanes** for packaging, benchmark publication, and Miri-style validation visibility

These are workflow and ecosystem gaps, not missing core runtime primitives.

## CU Performance

Raw overlay cast performance (measured, not estimated):

| Operation | Hopper | Anchor | Delta |
|-----------|--------|--------|-------|
| Layout overlay (64B) | ~50 CU | N/A | Pointer cast only |
| Borsh deserialize (64B) | N/A | ~800 CU | Not applicable |
| Header validation | ~120 CU | N/A | disc + version + layout_id check |
| Full Tier A load | ~170 CU | ~800 CU | 4.7x faster |
| Tier C (raw cast) | ~30 CU | N/A | Pure pointer arithmetic |
| WireU64 get/set | ~10 CU | ~10 CU | Same (LE byte swap) |
| Policy resolve (const) | 0 CU | N/A | Compile-time only |

See [BENCHMARKS.md](../BENCHMARKS.md) for full measurements.
