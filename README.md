# Hopper

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Solana](https://img.shields.io/badge/Solana-mainnet-9945FF.svg)](https://solana.com/)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)
![Tests](https://img.shields.io/badge/tests-workspace%20verified-brightgreen.svg)

**The zero-copy state framework for Solana.**

Pointer-cast speed. Protocol-grade safety. First-class state evolution.
Segment-level borrow enforcement.

Hopper maps fixed-layout zero-copy views directly onto account bytes with no
heap allocation and no serialization cycle. Unlike naive pointer-cast
approaches, Hopper layers this on top of ABI-safe overlays, versioned headers,
deterministic layout fingerprints, segmented state, **segment-level borrow
enforcement**, state receipts, and CLI tooling that can explain any account
from raw hex.

**What makes Hopper different from every other Solana framework:** segment-level
memory access. When you mutate a vault's `balance` field, Hopper locks exactly
those 8 bytes. A parallel read of `authority` on the same account? Fine, no
conflict. Every other framework locks the entire account. That is not a minor
ergonomic win. It is the difference between catching aliasing bugs at the byte
level and trusting developers to get it right manually.

Built on Hopper Native, Hopper's sovereign low-level runtime substrate for Solana.
Hopper also supports compatibility backends including Pinocchio and standard
Solana runtime surfaces where needed, but Hopper Runtime is the canonical
API surface all Hopper crates target.

Hopper also ships Hopper-owned companion program surfaces so authored programs
do not need to reach through external system/token helper crates: use
`hopper_system`, `hopper_token`, `hopper_token_2022`, and
`hopper_associated_token` directly, or via the root `hopper` crate re-exports.

Hopper Native owns the raw execution boundary: loader parsing, duplicate-account
resolution, eager and lazy entrypoints, syscall wrappers, and substrate account
views. Hopper Runtime sits one layer up and owns typed loading, layout
contracts, CPI semantics, context, and Hopper-facing PDA ergonomics.

`no_std`, `no_alloc`. No proc macros required for correctness. Proc macros
are optional DX accelerators only, never required for framework correctness.

## One Access Model

Hopper is not supposed to feel like a pile of parallel modes. There is one
runtime path and one access model, with different guarantees layered on top:

- whole-layout typed access via `account.load::<T>()` and `account.load_mut::<T>()`
  (or the indexed shortcut: `ctx.load::<T>(idx)` / `ctx.load_mut::<T>(idx)`)
- segment-aware typed access via `account.segment_ref(...)` and `account.segment_mut(...)`
  (or `ctx.segment_ref::<T>(idx, offset)` / `ctx.segment_mut::<T>(idx, offset)`)
- explicit raw escape hatches via `unsafe account.raw_ref::<T>()` and `unsafe account.raw_mut::<T>()`

Specialized helpers such as `load_cross_program()` and `load_versioned()` are not
separate frameworks. They are the same Hopper runtime path with a different
validation contract.

Hopper supports two authoring styles over that same access model:

**Core authored style (no proc macros required):** Write `hopper_layout!`
layouts, manage dispatch yourself, and call Hopper runtime accessors directly.

**Proc authored style (optional DX layer):** Annotate structs with
`#[hopper::state]`, `#[hopper::context]`, and `#[hopper::program]`. The macros
generate the same constants, typed accessors, and dispatch glue you would write
by hand.

Both paths compile to identical code: `ptr + const_offset → cast → &mut T`.
The proc macros are sugar, not structure. Enable them with:

```toml
[dependencies]
hopper = { version = "0.1", features = ["proc-macros"] }
```

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
  pub authority: TypedAddress<Authority>,
  pub balance: WireU64,
  pub bump: u8,
}

// Generates:
// - impl SegmentMap for Vault { const SEGMENTS = &[...] }
// - const VAULT_AUTHORITY_OFFSET: u32 = 0;
// - const VAULT_AUTHORITY_SIZE: u32 = 32;
// - const VAULT_BALANCE_OFFSET: u32 = 32;
// - const VAULT_BALANCE_SIZE: u32 = 8;
// . etc
```

Proc `state` layouts are body-only zero-copy views. Use `#[repr(C)]` and
alignment-1 Hopper wire types such as `WireU64`, `WireBool`, and
`TypedAddress<T>` so the generated layout contract can be loaded safely from
account bytes.

A typical handler just accesses the context and mutates the segment:

```rust
#[hopper::program]
mod vault {
  use super::*;

  #[instruction(1)]
  pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
    let mut balance = ctx.vault_balance_mut()?;
    *balance = WireU64::new(balance.get() + amount);
    Ok(())
  }
}
```

Typed contexts always generate per-account accessors such as
`ctx.vault_account()?`, `ctx.vault_load()?`, and `ctx.vault_raw_ref()?`.
When an account is declared fully mutable with `#[account(mut)]`, the proc
surface also emits whole-layout mutation accessors such as
`ctx.vault_load_mut()?` and `ctx.vault_raw_mut()?`, plus general typed
segment escapes like `ctx.vault_segment_mut::<WireU64>(abs_offset)` that
share the same const-offset lowering as the named accessors.

Opt-in handler attributes (`#[hopper::receipt]`, `#[hopper::invariant(..)]`,
and the duplicate-check `#[hopper::pipeline]`) layer additional guarantees
on top of the same access model. They are sugar, not structure. See
[docs/HOPPER_LANG.md](docs/HOPPER_LANG.md) for the full list.

## Program Lifecycle (advanced)

Beyond the one access model, Hopper surfaces an optional lifecycle for
protocols that want post-mutation invariants, structured receipts, and
migration tooling. This is **opt-in**. The access model stands alone.

```
1. Define     Layout your state with hopper_layout!
2. Resolve    Parse accounts from the instruction via Frame
3. Validate   Run checks, verify signatures, enforce policy
4. Execute    Mutate state in a controlled phase
5. Record     Capture a StateReceipt of what changed
6. Verify     Assert invariants and compatibility
7. Inspect    Use the CLI (and the hopper-manager library) to explain,
              diff, and plan migrations
```

Simple programs use steps 1, 3, and 4 and skip the rest. Complex protocols
layer in `#[hopper::receipt]`, `#[hopper::invariant]`, and the manager
tooling as needed. None of these steps are separate frameworks. They are
opt-in guarantees on top of the same access model.

## Access Guarantees

Hopper exposes one access system with three guarantee levels. Most programs use
validated whole-layout access first.

| Tier | Path | What you get |
|------|------|-------------|
| **A** | `Vault::load(account, program_id)?` | Full pipeline: validation, fingerprints, receipts, tooling |
| **B** | `pod_from_bytes::<Vault>(data)?` | Direct typed view, no header validation |
| **C** | `unsafe { Vault::load_unchecked(data) }` | Raw cast, caller owns all risk |

The cast overhead is intentionally minimal across all three. The difference is
what validation runs before the cast and what tracking runs after it. The safe
path adds header checks and fingerprint verification, the Pod path skips those,
and the raw path is an explicit caller-owned escape hatch. See
[MEMORY_ACCESS.md](docs/MEMORY_ACCESS.md) for measured CU numbers.

## Getting Started

There are three progressively deeper ways to use Hopper. Most programs only need
the standard path.

### Standard Hopper

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

Load and validate with the default path first, then reach for specialized
guarantees only when the use case changes:

```rust
// T1: Full validation (your own program's accounts)
let vault = Vault::load(account, program_id)?;
let balance = vault.map(|v| v.balance.get());

// Cross-program read with ABI proof
let vault = Vault::load_foreign(account, &other_program_id)?;

// Migration-compatible read on the same runtime path
let vault = account.load_versioned::<Vault>()?;

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
// . mutations ...
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

### Advanced Hopper

For bigger protocols that need segmented accounts, virtual multi-account state,
migration planning, trust profiles, and custom validation graphs.

- Segmented accounts with typed segment roles (Core, Extension, Journal, Index, Cache, Audit, Shard)
- Virtual state mapping across multiple accounts via `hopper_virtual!`
- Schema diffing and migration planning between layout versions
- Trust profiles for cross-program reads at different confidence levels
- Validation graphs with combinators, transition rule packs, and post-mutation hooks

See [`examples/hopper-registry`](examples/hopper-registry/src/lib.rs) and
[`examples/hopper-virtual-state`](examples/hopper-virtual-state/src/lib.rs).

### Escape Hatch

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
| **Segment-level borrows** | Fine-grained conflict detection at the byte range level. Read `authority` while writing `balance` on the same account. 16-entry compact registry (u64 fingerprint keys, ~280 bytes stack), no heap, inline checks |
| **SegmentMap** | Compile-time field→offset mapping via const trait. `segment("balance")` resolves to `StaticSegment { offset: 32, size: 8 }` at compile time |
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
+-- hopper-core          Ring 0: ABI types, account header, overlay, checks,
|                        collections, frame, lifecycle, fingerprints, migration,
|                        policy, receipts, segment roles, SegmentMap, virtual state
+-- hopper-macros        17 declarative macros (no proc macros, always available)
+-- hopper-macros-proc   Optional proc macros: #[hopper_state], #[hopper_context],
|                        #[hopper_program]. DX layer, never required.
+-- hopper-system        Hopper-owned System Program instruction builders
+-- hopper-token         Hopper-owned SPL Token instruction builders
+-- hopper-token-2022    Hopper-owned Token-2022 instruction builders + screening
+-- hopper-associated-token Hopper-owned ATA helpers + ATA instruction builders
+-- hopper-solana        SPL Token/Mint readers, CPI guards, typed CPI kits
+-- hopper-schema        Layout manifests, field diffs, migration planning,
|                        program manifests, IDL, Codama projection, field-level decoding
+-- hopper-cli           CLI: explain, inspect, compat, diff, plan, receipt, manager
```

## Sovereign Boundary

Hopper is split on purpose:

- **Hopper Native** owns raw loader parsing, duplicate-account handling,
  `hopper_program_entrypoint!`, `hopper_lazy_entrypoint!`, syscall wrappers,
  and the substrate `AccountView`.
- **Hopper Runtime** owns Hopper semantics: typed `AccountView` validation,
  `LayoutContract`, `Context`, checked CPI, and Hopper-facing PDA helpers.
- **compat/** owns every backend bridge. Pinocchio and solana-program support
  exist for interoperability, not as Hopper's identity.

That split is what lets Hopper stay raw-pointer fast without collapsing into a
thin wrapper around another framework.

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
| [`hopper-token-2022-vault`](examples/hopper-token-2022-vault/src/lib.rs) | Hopper-owned Token-2022 vault flow with local manifest-backed CLI preview | 2 |
| [`cross-program-read`](examples/cross-program-read/) | Interface pinning across two programs | 2 |

## CLI

The CLI is Hopper's host-side inspection and generation tool. It reads
hex-encoded account data and schema manifests to help you verify layouts,
segments, version compatibility, and mutation receipts. It is offline-first:
most commands operate on local manifests and raw bytes, while `hopper fetch`
and `hopper manager fetch` optionally use RPC to pull on-chain manifests.

When a package already contains `hopper.manifest.json`,
`hopper compile --emit rust` can infer it from the current project root. Use
`--package <name>` to target another workspace member and `--out <path>` to
write the lowered preview to disk.

Commands are organized into families:

```
Compile:
  hopper compile --emit rust [<manifest>]  Emit lowered runtime Rust: accessors, offsets, pointer path

Schema:
  hopper schema export [--manifest|--idl|--codama]  Schema format reference
  hopper schema validate <manifest>  Validate a program manifest
  hopper schema diff <old> <new>     Field-level diff between versions

Inspect:
  hopper inspect <hex>               Raw header decode
  hopper inspect layout <manifest> <hex>  Decode fields using a program manifest
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
  hopper explain context <manifest> [--type <ContextName>]  Explain instruction contexts and generated accessors

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

Hopper's strongest differentiator is not "pointer casts exist". Pinocchio,
Quasar, and Anchor's `AccountLoader` already cover raw zero-copy access in
different ways. Hopper's lead is that it treats layouts as runtime contracts:
versioned headers, deterministic layout fingerprints, foreign/versioned loads,
field maps, schema export, segment roles, and manager-ready metadata all come
from the same state model.

### Benchmark (Parity Vault, 8-seed average)

| Scenario | Hopper | Quasar | Pinocchio-style |
|----------|--------|--------|-----------------|
| Authorize | **432 CU** | 585 CU | 2543 CU |
| Auth-fail | **70 CU** | 66 CU | 74 CU |
| Counter (segment-safe) | **539 CU** | 607 CU | 2575 CU |
| Deposit | **1651 CU** | 1768 CU | 3763 CU |
| Withdraw | **455 CU** | 605 CU | 2567 CU |
| **Binary size** | **7.62 KiB** | 8.36 KiB | 10.13 KiB |

**Hopper beats Quasar on 4 of 5 instructions** on the parity-vault bench.
Hopper's counter-access uses `segment_ref` and `segment_mut` with
segment-level borrow tracking. Quasar and Pinocchio use raw byte slicing
with no conflict detection. The verify-only PDA path (sha256 only, no
`curve_validate` syscall) saves ~350 CU per PDA-bearing instruction.
Hopper produces the **smallest binary** of all three frameworks. Source
numbers in `bench/results/` and methodology in
[bench/METHODOLOGY.md](bench/METHODOLOGY.md).

| | Hopper | Anchor zero-copy | Pinocchio | Quasar |
|---|---|---|---|---|
| Raw entrypoint ownership | Yes | No | Yes | Yes |
| Zero-copy account access | Yes | `AccountLoader` | Yes | Yes |
| no_std / no_alloc | Yes | No | Yes | Yes |
| Segment-level borrow enforcement | Yes | No | No | No |
| Compile-time SegmentMap | Yes | No | No | No |
| Deterministic layout fingerprints | Yes | No | No | No |
| Versioned + foreign typed loads | Yes | No | No | No |
| Segment roles and registries | Yes | No | No | No |
| Field maps + schema export | Yes | IDL only | No | IDL only |
| State receipts | Yes | No | No | No |
| Policy system | Yes | No | No | No |
| Optional proc macros (not required) | Yes | N/A (required) | No | Yes |
| CLI / profiling / client tooling | Strong | Strong | Minimal | Strong |
| Backend portability | 3 backends | solana-program | pinocchio | pinocchio |
| Memory access tiers | 3 (safe/pod/raw) | 1 (`AccountLoader`) | 1 (raw) | 1 (raw) |

Anchor still leads on ecosystem reach and polished public tooling. Quasar is
stronger than older comparisons often gave it credit for in CLI, profiling,
IDL, and generated clients. Hopper's claim is different: it is the only one of
these frameworks that makes the layout contract itself the center of runtime,
schema, migration, and tooling.

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
3. **Segment-level precision.** Lock bytes, not accounts. The smallest unit of mutation is a field, not a buffer.
4. **Compile-time safety.** Typestate, const generics, and deterministic hashing over runtime checks.
5. **Zero hidden cost.** No allocations, no trait objects, no dynamic dispatch on-chain.
6. **Self-describing accounts.** The 16-byte header makes every account inspectable.
7. **Append-only evolution.** New fields extend layouts. Old data stays valid.
8. **Control-first, DX-optional.** The core is always hand-writeable. Macros accelerate, never gate.
9. **Rigid where safety matters, flexible where architecture matters.**

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

## Guard macros

Early-return guard macros in the Jiminy-replacement tradition. Zero
overhead, zero dependencies, available at `hopper::*`:

```rust
hopper::require!(amount > 0, ProgramError::InvalidArgument);
hopper::require_eq!(vault.version, 1, ProgramError::InvalidAccountData);
hopper::require_neq!(source, destination, ProgramError::InvalidAccountData);
hopper::require_keys_eq!(vault.authority, signer.address(), ProgramError::InvalidAccountData);
hopper::require_keys_neq!(authority_a, authority_b, ProgramError::InvalidAccountData);
hopper::require_gte!(account.lamports(), required, ProgramError::InsufficientFunds);
hopper::require_gt!(fresh_slot, last_slot, ProgramError::InvalidArgument);
```

Every macro has a short form that defaults to a sensible error
(`InvalidArgument`, `InvalidAccountData`, or `InsufficientFunds`).
Regression suite lives in [tests/require_macros.rs](tests/require_macros.rs).

## `hopper verify`

ABI-integrity command. Catches the silent refactor where a developer
changes a layout, rebuilds the program, and forgets to re-export the
manifest. Two phases, first fatal, second informational by default:

```bash
# Manifest-only integrity: unique disc, unique LAYOUT_ID, non-zero bytes
hopper verify @my-program.manifest.json

# Plus binary scan for each LAYOUT_ID fingerprint in the compiled .so
hopper verify @my-program.manifest.json --so target/deploy/my_program.so

# Infer both from a workspace package
hopper verify --package my-program

# Treat missing anchors as fatal
hopper verify --package my-program --strict
```

`#[hopper::state]` emits a `#[used]` static per layout so the
`LAYOUT_ID` bytes survive SBF link-time optimization. Declarative
`hopper_layout!` does not yet emit the anchor, so `--strict` is the
right gate for programs built exclusively with the proc-macro path.

## Client-side ABI verification

Generated clients (TypeScript, Kotlin, and Rust) include a per-layout
`assert_{name}_layout` helper that reads the 8-byte `LAYOUT_ID`
fingerprint from the 16-byte Hopper header and rejects mismatches:

```ts
import { assertVaultLayout, decodeVault, VAULT_LAYOUT_ID } from "./accounts";

const data = await connection.getAccountInfo(vaultPubkey).then(a => a!.data);
assertVaultLayout(data); // throws if the on-chain ABI drifted
const vault = decodeVault(data);
```

```rust
use hopper_vault_client::{assert_vault_layout, decode_vault, deposit_ix, DepositArgs};

let account = rpc.get_account(&vault_pubkey).await?;
let vault = decode_vault(&account.data)?; // LayoutMismatch if the ABI drifted
let ix = deposit_ix(&program_id, &accounts, &DepositArgs { amount: 1_000_000 });
```

Three target languages, one fingerprint contract. The client and program
agree on the layout fingerprint byte-for-byte, so an upgrade that
silently changes field order fails at the SDK boundary instead of
corrupting state. Produce any of them with:

```bash
hopper compile --emit ts          @program.manifest.json --out client.ts
hopper compile --emit kt          @program.manifest.json --out Client.kt
hopper compile --emit rust-client @program.manifest.json --out client.rs
```

## Token-CPI safety default

`hopper_token::{Transfer, MintTo, Burn, CloseAccount, Approve, Revoke}`
builders enforce the authority-signer invariant on the no-signer
`invoke()` path before reaching the CPI:

```rust
// Raises MissingRequiredSignature if authority is not a transaction signer.
hopper_token::Transfer { from, to, authority, amount: 1_000 }.invoke()?;

// PDA-signed path (no pre-check; the signer seeds are the authorization).
hopper_token::Transfer { from, to, authority: &vault_pda, amount: 1_000 }
    .invoke_signed(&[vault_signer])?;
```

The SPL Token program enforces the same rule at CPI time, but the
pre-check surfaces a Hopper-branded error identifying exactly which
field is wrong instead of an opaque CPI failure.

## Safety Audit Closure

The Hopper Safety Audit drove a full pass through the framework. Every
must-fix, should-fix, and structural item is closed in-tree with
file-and-line evidence in [UNSAFE_INVARIANTS.md](docs/UNSAFE_INVARIANTS.md).
A summary of the shipping closures:

| Audit item | Where it closes |
|---|---|
| Malformed duplicate-account rejection | `crates/hopper-native/src/raw_input.rs` + safe `parse_instruction_frame_checked` |
| RAII segment leases | `crates/hopper-runtime/src/segment_lease.rs` |
| Canonical wire fingerprint | `crates/hopper-macros-proc/src/state.rs` canonical_wire_stem |
| Field-level Pod proof | `__FieldPodProof<T: bytemuck::Pod + Zeroable>` in `pod.rs` + `state.rs` |
| Compile-fail harness | 10 trybuild fixtures in `tests/compile_fail/` |
| Fuzzing | `fuzz/` crate with 4 libfuzzer targets |
| Anchor-grade constraints | `#[hopper::context]` parses init/payer/space/seeds/bump/close/realloc/has_one/owner/address/constraint |
| Typed wrappers | `Signer<'info>`, `Account<'info, T>`, `InitAccount<'info, T>`, `Program<'info, P>` in `hopper-runtime::account_wrappers` |
| Schema-epoch migrations | `#[hopper::migrate]` + `hopper::layout_migrations!` + `apply_pending_migrations` |
| Hybrid serialization | `#[hopper::state(dynamic_tail = T)]` + `TailCodec` |
| Foreign-account lenses | `ForeignLens<T>` + `ForeignManifest` in `hopper-runtime::foreign` |
| Multi-target compile | `hopper compile --emit <target>` for rust, ts, kt, idl, codama, schema |
| Cross-framework bench | `bench/METHODOLOGY.md` + anchor slot in `framework-vault-bench` |

## Documentation

| Doc | What it covers |
|-----|---------------|
| [The Hopper Model](docs/THE_HOPPER_MODEL.md) | Complete framework reference in one page |
| [Memory Access Doctrine](docs/MEMORY_ACCESS.md) | Three-tier memory access with performance data |
| [Schema Architecture](docs/SCHEMA_ARCHITECTURE.md) | Schema model, manifest format, IDL spec, Codama compatibility |
| [Proc Macro Policy](docs/PROC_MACRO_POLICY.md) | Proc macro governance: what's allowed and what's not |
| [Hopper Manager](docs/HOPPER_MANAGER.md) | Manager vision: CLI, web dashboard, embedded admin |
| [Benchmarks](BENCHMARKS.md) | Benchmark lab, current CU baselines, and automation coverage |
| [Unsafe Invariants](docs/UNSAFE_INVARIANTS.md) | Every unsafe block cataloged with justifications |
| [Architecture](docs/ARCHITECTURE.md) | Crate structure and module map |
| [Getting Started](docs/GETTING_STARTED_SERIOUS.md) | Build a complete program from scratch |
| [Parity and Differentiation](docs/PARITY_AND_DIFFERENTIATION.md) | Feature-level comparison with every competitor |
| [Bad Evolution](docs/BAD_EVOLUTION.md) | Layout evolution anti-patterns and how Hopper catches them |

## License

Apache-2.0. See [LICENSE](LICENSE) for details.

## Links

- **Author**: [@moonmanquark](https://x.com/moonmanquark)
- **Organization**: [BluefootLabs](https://github.com/BluefootLabs)
- **Repository**: [GitHub](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework)
