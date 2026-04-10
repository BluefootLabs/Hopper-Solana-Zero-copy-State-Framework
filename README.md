# Hopper

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Solana](https://img.shields.io/badge/Solana-mainnet-9945FF.svg)](https://solana.com/)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)
![Tests](https://img.shields.io/badge/tests-workspace%20verified-brightgreen.svg)

**The typed state pipeline framework for Solana.**

Pointer-cast speed. Protocol-grade safety. First-class state evolution.

Hopper maps fixed-layout zero-copy views directly onto account bytes with no
heap allocation and no serialization cycle. Unlike naive pointer-cast
approaches, Hopper layers this on top of ABI-safe overlays, versioned headers,
deterministic layout fingerprints, segmented state, state receipts, and CLI
tooling that can explain any account from raw hex.

Built on Hopper Native, Hopper's sovereign low-level runtime substrate for Solana.
Hopper also supports compatibility backends including Pinocchio and standard
Solana runtime surfaces where needed, but Hopper Runtime is the canonical
API surface all Hopper crates target.

`no_std`, `no_alloc`. No proc macros required for correctness. Proc macros
are optional DX accelerators only, never required for framework correctness.

## The Pipeline

Every Hopper program follows the same seven-step model:

```
1. Define     Layout your state with hopper_layout!
2. Resolve    Parse accounts from the instruction via Frame
3. Validate   Run checks, verify signatures, enforce policy
4. Execute    Mutate state in a controlled phase
5. Record     Capture a StateReceipt of what changed
6. Verify     Assert invariants and compatibility
7. Inspect    Use the CLI to explain, diff, and plan migrations
```

This is the canonical path. You can use less of it for simple programs and more
of it for complex protocols, but the pipeline is always the mental model.

## Memory Access Tiers

Hopper supports three levels of memory access. Most programs use Tier A.

| Tier | Path | What you get |
|------|------|-------------|
| **A** | `Vault::load(account, program_id)?` | Full pipeline: validation, fingerprints, receipts, tooling |
| **B** | `pod_from_bytes::<Vault>(data)?` | Direct typed view, no header validation |
| **C** | `unsafe { Vault::load_unchecked(data) }` | Raw cast, caller owns all risk |

The cast overhead is intentionally minimal across all tiers. The difference is in
what validation runs before and what tracking runs after. Tier A adds header
checks and fingerprint verification, Tier B skips those, and Tier C is a raw
pointer cast with no framework code on the path. See
[MEMORY_ACCESS.md](docs/MEMORY_ACCESS.md) for measured CU numbers.

## Getting Started

There are three tiers of Hopper usage. Most programs only need Tier 1.

### Tier 1: Standard Hopper

The default path. Versioned layouts, phased execution, validation, and receipts.
This is what you reach for on day one.

```rust
use hopper::prelude::*;

hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority>  = 32,
        mint:      TypedAddress<Mint>       = 32,
        balance:   WireU64                  = 8,
        bump:      u8                       = 1,
    }
}
```

Load and validate with tiered trust:

```rust
// T1: Full validation (your own program's accounts)
let vault = Vault::load(account, program_id)?;
let balance = vault.map(|v| v.balance.get());

// T2: Cross-program (read someone else's account by ABI proof)
let vault = Vault::load_foreign(account, &other_program_id)?;

// T3: Version-compatible (accept V1+ during migration, skip layout_id)
let vault = VaultV1::load_compatible(account, program_id, 1)?;
```

On raw `AccountView` values, the runtime-first equivalent of the migration tier
is `account.load_versioned::<VaultV1>()`.

Define what each instruction is allowed to do:

```rust
const DEPOSIT_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::MutatesState)
    .with(Capability::MutatesTreasury);

const POLICY: InstructionPolicy<3> = InstructionPolicy::new()
    .when(Capability::MutatesState, PolicyRequirement::Authority)
    .when(Capability::MutatesState, PolicyRequirement::InvariantCheck)
    .when(Capability::MutatesTreasury, PolicyRequirement::LamportConservation);
```

Record what happened:

```rust
let mut receipt = StateReceipt::<256>::begin(&Vault::LAYOUT_ID, data);
// ... mutations ...
receipt.commit_with_segments(data, &[(offset, size)]);
receipt.set_policy_flags(DEPOSIT_CAPS.bits());
receipt.set_invariants(true, 1);
emit_slices(&[&receipt.to_bytes()]);
```

Receipts capture before/after fingerprints, changed fields, byte-level diffs,
segment tracking, and policy flags. They are Hopper's primary auditability
artifact.

See [`examples/hopper-showcase`](examples/hopper-showcase/src/lib.rs) for the
complete reference implementation that uses every layer.

### Tier 2: Advanced Hopper

For bigger protocols that need segmented accounts, virtual multi-account state,
migration planning, trust profiles, and custom validation graphs.

- Segmented accounts with typed segment roles (Core, Extension, Journal, Index, Cache, Audit, Shard)
- Virtual state mapping across multiple accounts via `hopper_virtual!`
- Schema diffing and migration planning between layout versions
- Trust profiles for cross-program reads at different confidence levels
- Validation graphs with combinators, transition rule packs, and post-mutation hooks

See [`examples/hopper-registry`](examples/hopper-registry/src/lib.rs) and
[`examples/hopper-virtual-state`](examples/hopper-virtual-state/src/lib.rs).

### Tier 3: Escape Hatch

When you need to go lower than the framework, Hopper gets out of your way.
Every macro has an `_unchecked` or manual alternative. You can drop down to raw
overlay access, write custom wire formats, build your own collections, or
bypass validation for init-time writes. The framework is a set of tools, not a
cage.

## What You Get

| Area | What it does |
|------|-------------|
| **Typed overlays** | `#[repr(C)]` structs mapped directly onto account bytes, zero serialization cost |
| **16-byte header** | Every account is self-describing: disc, version, flags, layout fingerprint |
| **Deterministic fingerprints** | SHA-256 of field names/types/sizes, computed at compile time |
| **5-tier loading** | Full, foreign, compatible, unchecked, unverified |
| **3 memory access tiers** | Safe overlay, explicit pod, unsafe raw. Same access model, different validation overhead |
| **Phased execution** | Resolve, Validate, Execute enforced at compile time via typestate |
| **Modifier wrappers** | `Signer<Mut<Account<'a, T>>>` composable constraint types |
| **8 zero-copy collections** | FixedVec, RingBuffer, SlotMap, BitSet, Journal, Slab, PackedMap, SortedVec |
| **Segment roles** | Core, Extension, Journal, Index, Cache, Audit, Shard with typed semantics |
| **State receipts** | Structured mutation proof: before/after fingerprints, changed fields, byte counts, segment tracking, policy flags, phase metadata, compat impact, migration flags |
| **Policy system** | Declare capabilities, auto-trigger validation requirements. Named packs for common patterns |
| **Schema spine** | ProgramManifest > ProgramIdl > CodamaProjection - three projection layers from one source of truth |
| **Migration planner** | Generate step-by-step plans between layout versions, segment-role-aware |
| **Cross-program reads** | `hopper_interface!` reads foreign accounts by fingerprint, no crate dependency |
| **CPI guards** | Detect CPI invocation, flash loan brackets, subsequent calls |
| **Trust profiles** | Strict, Compatible, ReadOnly, and Observational loading for foreign accounts |
| **CLI tooling** | explain, inspect, decode, segments, compat, diff, plan, receipt, schema-export, client gen |
| **Program Manager** | Identify accounts, decode fields, inspect instructions, list policies from manifest |
| **Client SDK generation** | TypeScript and Kotlin (org.sol4k) client generators from program manifests |

## Crate Architecture

```
hopper (root facade, re-exports everything)
|
+-- hopper-core        Ring 0: ABI types, account header, overlay, checks,
|                      collections, frame, lifecycle, fingerprints, migration,
|                      policy, receipts, segment roles, virtual state
+-- hopper-macros      17 declarative macros (proc macros optional, not required)
+-- hopper-solana      SPL Token/Mint readers, CPI guards, typed CPI kits
+-- hopper-schema      Layout manifests, field diffs, migration planning,
|                      program manifests, IDL, Codama projection, field-level decoding
+-- hopper-cli         CLI: explain, inspect, compat, diff, plan, receipt, manager
```

## Examples

Start with `hopper-showcase`. It is the canonical Hopper program that uses
every layer of the pipeline: layout, dispatch, phased frame, policy, receipts,
invariants, segment roles, and state diffs. The other examples focus on
specific patterns.

| Example | What it shows | Tier |
|---------|-------------|------|
| [`hopper-showcase`](examples/hopper-showcase/src/lib.rs) | **Full pipeline reference** (start here) | 1+2 |
| [`hopper-vault`](examples/hopper-vault/src/lib.rs) | Simple SOL vault: layout, dispatch, phased frame | 1 |
| [`hopper-escrow`](examples/hopper-escrow/src/lib.rs) | Token escrow with authority checks | 1 |
| [`hopper-treasury`](examples/hopper-treasury/src/lib.rs) | Multi-segment treasury with permissions | 2 |
| [`hopper-registry`](examples/hopper-registry/src/lib.rs) | Segmented registry with journal and virtual state | 2 |
| [`hopper-migration`](examples/hopper-migration/src/lib.rs) | V1 to V2 layout evolution with migration planner | 2 |
| [`hopper-virtual-state`](examples/hopper-virtual-state/src/lib.rs) | Multi-account entities with VirtualState | 2 |
| [`cross-program-read`](examples/cross-program-read/) | Interface pinning across two programs | 2 |

## CLI

The CLI is Hopper's host-side inspection and generation tool. It reads
hex-encoded account data and schema manifests to help you verify layouts,
segments, version compatibility, and mutation receipts. It is offline-first:
most commands operate on local manifests and raw bytes, while `hopper fetch`
and `hopper manager fetch` optionally use RPC to pull on-chain manifests.

Commands are organized into families:

```
Schema:
  hopper schema export [--manifest|--idl|--codama]  Schema format reference
  hopper schema validate <manifest>  Validate a program manifest
  hopper schema diff <old> <new>     Field-level diff between versions

Inspect:
  hopper inspect <hex>               Raw header decode
  hopper inspect segments <hex>      Segment registry map
  hopper inspect receipt <hex>       Decode a state receipt

Explain:
  hopper explain <hex>               Human-readable account explanation
  hopper explain account <hex>       Explicit account explanation
  hopper explain receipt <hex>       Explain a receipt in plain English
  hopper explain compat <old> <new>  Explain compatibility report
  hopper explain policy <pack>       Explain a named policy pack
  hopper explain layout <manifest>   Explain layout fields, intents, fingerprint
  hopper explain program <manifest>  Explain entire program pipeline
  hopper explain context <manifest> [--type <ContextName>]  Explain instruction contexts

Compatibility:
  hopper compat <old> <new>          Compatibility report
  hopper plan <old> <new>            Migration plan with steps

Receipt:
  hopper receipt <hex>               Decode and display a 64-byte state receipt

Fetch:
  hopper fetch <program-id> [--rpc <url>]  Fetch manifest from on-chain

Interactive:
  hopper interactive <manifest>      Interactive terminal explorer

Client SDK:
  hopper client gen --ts <manifest>  Generate TypeScript client SDK
  hopper client gen --kt <manifest>  Generate Kotlin client SDK (org.sol4k)
```

### Manager

The Manager is Hopper's program-level management and inspection interface.
Given a program manifest today, or an on-chain manifest fetched via
`hopper fetch` / `hopper manager fetch`, it provides a complete view of your
program: every layout, every instruction, every policy, and every account type,
decoded, explained, and cross-referenced.

```
hopper manager summary <manifest>                      Program overview
hopper manager identify <manifest> <hex>               Identify account type
hopper manager decode <manifest> <hex>                 Decode all fields with values
hopper manager instruction <manifest> <tag|name>       Instruction details and policies
hopper manager layouts <manifest>                      List all layouts with fields
hopper manager policies <manifest>                     List policy packs with mappings
hopper manager fingerprints <manifest>                 Show all layout fingerprints
hopper manager events <manifest>                       List events with fields
hopper manager compat <manifest> <hex-old> <hex-new>   Compare two account versions
hopper manager receipt <hex-64-bytes>                  Decode a state receipt
hopper manager explain <manifest>                      Aggregated human-readable summary
hopper manager diff <manifest> <hex-old> <hex-new>     Semantic field-level diff
hopper manager fetch <program-id> [--rpc <url>]        Fetch manifest from on-chain
hopper manager simulate <manifest> <instruction>       Preview instruction requirements
hopper manager interactive <manifest>                  Interactive terminal explorer
```

Manifest arguments accept `@path/to/file.json` to load from disk.
See [`examples/sample-manifest.json`](examples/sample-manifest.json) for the format.

**Roadmap**: live account discovery and program-address-first workflows can
build on the existing on-chain manifest path. Today, the concrete entry points
are `hopper fetch`, `hopper manager fetch`, and the manifest-driven manager
subcommands above.

## Backend Selection

Hopper Native is the default backend. All examples, docs, and the CLI target it.
Pinocchio and solana-program are supported as compatibility backends for
projects that need to interoperate with existing codebases.

```toml
# Default: Hopper Native (no configuration needed)
[dependencies]
hopper = "0.1"

# Pinocchio backend
[dependencies]
hopper = { version = "0.1", default-features = false, features = ["pinocchio-backend"] }

# solana-program backend
[dependencies]
hopper = { version = "0.1", default-features = false, features = ["solana-program-backend"] }
```

All examples include backend feature flags. Build any example with an
alternate backend:

```sh
cargo build -p hopper-vault --no-default-features --features pinocchio-backend
```

Only one backend may be active at a time. Enabling multiple backends produces a
compile error.

## Comparison

| | Hopper | Anchor | Pinocchio | Star Frame |
|---|---|---|---|---|
| Zero-copy overlays | Yes | No (Borsh) | Yes | Yes |
| no_std / no_alloc | Yes | No | Yes | Yes |
| No proc macros required for correctness | Yes | No | Yes | No |
| Deterministic fingerprints | Yes | No | No | Partial |
| 5-tier loading | Yes | No | No | Partial |
| Typestate execution | Yes | No | No | Yes |
| Schema diffing | Yes | No | No | Partial |
| Migration planner | Yes | No | No | No |
| 8 zero-copy collections | Yes | No | No | Partial |
| Segment roles | Yes | No | No | No |
| State receipts | Yes | No | No | No |
| Named policy packs | Yes | No | No | No |
| Cross-program interfaces | Yes | No | No | Partial |
| CLI tooling | Yes | Partial | No | No |
| Program Manager | Yes | No | No | No |
| Memory access tiers | 3 (safe/pod/raw) | 1 (Borsh) | 1 (raw) | 1 (raw) |

## Trust Posture

Hopper is a zero-copy framework, which means it casts raw byte slices into
typed references using `unsafe`. Every other zero-copy framework on Solana does
the same thing. What matters is how those boundaries are managed.

**What is unsafe and why:**

| Boundary | Justification | Mitigation |
|----------|--------------|------------|
| `pod_from_bytes` / `pod_from_bytes_mut` | Core pointer cast from `&[u8]` to `&T` | Size and alignment checked before cast. `T: Pod` trait requires `repr(C)`, no padding, byte-safe fields. |
| `load_unchecked` (Tier C) | Opt-in raw cast for hot paths | Only exposed as `unsafe fn`. Caller accepts all risk. Never used by default pipeline. |
| Segment table reads | Decode offset/size from account bytes | Bounds-checked against buffer length before every access. |
| VerifiedAccount header access | Read disc/version/layout_id from first 16 bytes | Length check (>= HEADER_LEN) runs before any field read. |

**What is *not* unsafe:**

- Layout declaration (`hopper_layout!`) is pure const construction.
- Policy checks, capability sets, and receipts are safe Rust.
- Schema diffing, fingerprinting, and migration planning are pure functions.
- CLI tooling never uses unsafe. All decoding goes through `decode_header`
  which returns `Option`.

**Testing unsafe boundaries:**

Every unsafe entry point has a companion test in
[unsafe_boundary_tests.rs](crates/hopper-core/tests/unsafe_boundary_tests.rs)
that exercises undersized, oversized, empty, and misaligned inputs.
[overlay_equivalence_tests.rs](crates/hopper-core/tests/overlay_equivalence_tests.rs)
proves that overlay casts produce the same values as manual byte decoding.

The full unsafe inventory with line-level justifications is in
[UNSAFE_INVARIANTS.md](docs/UNSAFE_INVARIANTS.md).

## Design Principles

1. **Bytes first.** Think in offsets and wire formats, not abstractions.
2. **Pipeline model.** Define, Resolve, Validate, Execute, Record, Verify, Inspect.
3. **Compile-time safety.** Typestate, const generics, and deterministic hashing over runtime checks.
4. **Zero hidden cost.** No allocations, no trait objects, no dynamic dispatch on-chain.
5. **Self-describing accounts.** The 16-byte header makes every account inspectable.
6. **Append-only evolution.** New fields extend layouts. Old data stays valid.
7. **Rigid where safety matters, flexible where architecture matters.**

## Schema Layering

Hopper produces three progressively narrower schema projections from one source
of truth. Each layer strips internal details while preserving what its audience
needs:

```
ProgramManifest      Full truth: layouts, instructions, events, policies,
   |                 layout metadata, compatibility pairs, tooling hints
   v
ProgramIdl           Public-facing: instructions (with PDA seed hints),
   |                 accounts, events, fingerprints
   v
CodamaProjection     Ecosystem interop: Codama-shaped instructions,
                     accounts, events for client generators
```

The manifest is what the CLI, Manager, and migration planner consume.
The IDL is what documentation and client SDKs consume.
The Codama projection is what ecosystem tools (Kinobi, Umi) consume.

## Documentation

| Doc | What it covers |
|-----|---------------|
| [The Hopper Model](docs/THE_HOPPER_MODEL.md) | Complete framework reference in one page |
| [Memory Access Doctrine](docs/MEMORY_ACCESS.md) | Three-tier memory access with performance data |
| [Schema Architecture](docs/SCHEMA_ARCHITECTURE.md) | Schema model, manifest format, IDL spec, Codama compatibility |
| [Proc Macro Policy](docs/PROC_MACRO_POLICY.md) | Proc macro governance: what's allowed and what's not |
| [Hopper Manager](docs/HOPPER_MANAGER.md) | Manager vision: CLI, web dashboard, embedded admin |
| [Benchmarks](BENCHMARKS.md) | CU measurements for every primitive |
| [Unsafe Invariants](docs/UNSAFE_INVARIANTS.md) | Every unsafe block cataloged with justifications |
| [Architecture](docs/ARCHITECTURE.md) | Crate structure and module map |
| [Publish Readiness](docs/PUBLISH_READINESS.md) | Current release checklist and staged packaging status |
| [Getting Started](docs/GETTING_STARTED_SERIOUS.md) | Build a complete program from scratch |
| [Parity and Differentiation](docs/PARITY_AND_DIFFERENTIATION.md) | Feature-level comparison with every competitor |
| [Bad Evolution](docs/BAD_EVOLUTION.md) | Layout evolution anti-patterns and how Hopper catches them |

## License

Apache-2.0. See [LICENSE](LICENSE) for details.

## Links

- **Author**: [@moonmanquark](https://x.com/moonmanquark)
- **Organization**: [BluefootLabs](https://github.com/BluefootLabs)
- **Repository**: [GitHub](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework)
