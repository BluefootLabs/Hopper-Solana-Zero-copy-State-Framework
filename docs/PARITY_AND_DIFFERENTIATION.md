# Parity and Differentiation

Where Hopper stands relative to every other Solana state framework.

This document covers concrete feature-level comparison, not marketing. Each row
is backed by code that exists today.

## Feature Matrix

| Feature | **Hopper** | **Anchor** | **Steel / Pinocchio** | **Quasar** | **Star Frame** | **Jiminy** |
|---------|-----------|-----------|----------------------|-----------|---------------|-----------|
| Zero-copy overlays | repr(C) + 16B header | Borsh deser | bytemuck Pod | bytemuck Pod | bytemuck Pod | repr(C) + 16B header |
| ABI versioning | version byte + fingerprint | None | None | None | None | version + layout_id |
| no_std / no_alloc | Yes | No | Yes | Yes | No | Yes |
| Proc macro weight | Light (layout + dispatch) | Heavy (accounts, derive) | None (decl macros) | Heavy | Heavy | Light (layout) |
| Collections (zero-copy) | 8 types | None | None | None | 3 types | 8 types |
| State receipts | 64-byte wire proof | None | None | None | None | Yes |
| Schema diffing + migration | CLI + planner | None | None | None | None | CLI + planner |
| Bump caching | BUMP_OFFSET | None | None | BUMP_OFFSET | None | BUMP_OFFSET |
| Trust profiles | 5-tier loading | None | None | None | 4-phase lifecycle | 5-tier |
| CPI safety | Typed CPI + flash loan detect | CPI context | Heap alloc CPI | RawEncoded | Typed CPI | Typed CPI + guard |
| Policy system | Capability + PolicyRequirement | None | None | None | None | None |
| Phased execution (typestate) | Resolve-Validate-Execute | None | None | None | Yes | None |
| Cross-program interfaces | hopper_interface! | None | None | None | None | jiminy_interface! |
| CLI tooling | 20+ commands | anchor verify | None | None | None | CLI |
| Program Manager | Yes | None | None | None | None | None |
| Client SDK gen (TS + Kotlin) | Yes | TS only | None | None | TS (Codama) | None |
| DeFi math | 16 checked ops + bps | External crate | None | None | Fixed-point | Same |
| AMM / slippage | hopper-finance | None | None | None | None | jiminy-finance |
| Lending formulas | hopper-lending | None | None | None | None | jiminy-lending |
| Staking formulas | hopper-staking | None | None | None | None | jiminy-staking |
| Segment roles | 7 typed roles | None | None | None | None | No |
| Instruction dispatch | 1-byte + 2-byte | IDL-derived | Manual | Proc macro | Proc macro | 1-byte |
| Memory access tiers | 3 (safe/pod/raw) | 1 (Borsh) | 1 (raw) | 1 (pod) | 1 (raw) | 3 |
| Safety model | Documented unsafe inventory | Hidden by proc macros | Unsound (Steel) | Documented | Miri-validated | Documented |
| Error system | hopper_error! (sequential codes) | anchor_lang errors | ProgramError only | Custom | Custom | Custom |
| Virtual state | hopper_virtual! multi-account | None | None | None | None | None |
| Semantic lint engine | lint_layout + lint_policy (7 rules) | None | None | None | None | None |
| Receipt narratives | Human-readable explain | None | None | None | None | None |
| Layout stability grades | Computed heuristic | None | None | None | None | None |
| Role-aware compatibility | refine_with_roles | None | None | None | None | None |

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

You cannot call `.execute()` without first calling `.validate()`. This is not a
runtime check -- the compiler rejects it. Star Frame has a similar concept but
ties it to proc macros.

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

Anchor is the dominant framework in the Solana ecosystem. Hopper matches or
exceeds it in every technical dimension:

- **Serialization**: Hopper uses zero-copy overlays (0 CU), Anchor uses Borsh (500-2000 CU per deser)
- **Proc macros**: Hopper uses `macro_rules!` (compile in seconds), Anchor requires proc macros (slow builds)
- **std**: Hopper is `no_std` + `no_alloc`, Anchor requires `std`
- **Tooling**: Hopper's CLI has 20+ commands, Anchor has `anchor verify` and `anchor idl`
- **Receipts**: Hopper generates structured mutation proofs, Anchor has nothing comparable
- **Migration**: Hopper has schema diffing + migration planning, Anchor has manual migration

Where Anchor wins: ecosystem adoption, documentation volume, tutorial count,
third-party tooling integration, and the "everyone uses it" network effect.

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

Quasar uses bytemuck Pod overlays with BUMP_OFFSET caching. It is
pre-release (v0.0.0). Hopper matches its bump caching and exceeds it in
every other dimension: headers, fingerprints, policies, receipts, schema,
CLI, collections, segment roles.

### vs. Star Frame

Star Frame uses bytemuck Pod with a 4-phase lifecycle and typed CPI.
Hopper matches its typestate execution, exceeds it with 7 segment roles
(vs. none), 8 collections (vs. 3), receipts, policies, and CLI tooling.
Star Frame's Miri validation is a genuine advantage; Hopper compensates
with a documented unsafe inventory and companion boundary tests.

## Known Gaps

Hopper does not yet have:

1. **Project scaffolding / build / deploy workflows** in the CLI
2. **CU profiling and disassembly tooling** comparable to `quasar profile` / `quasar dump`
3. **Program-address-first Manager workflows** beyond manifest fetch + manifest-driven inspection
4. **Inline dynamic field ergonomics** for string/vec-style tails
5. **Vec-like segment collection APIs** on top of Hopper's raw segment accessors
6. **CI/CD pipeline and public packaging rollout**

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
