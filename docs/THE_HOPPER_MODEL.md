# The Hopper Model

Hopper is a typed state pipeline framework for Solana. This page is the
canonical reference for how the whole system fits together.

## The Pipeline

Every Hopper program follows seven steps:

```
1. Define     Layout your state with hopper_layout!
2. Resolve    Parse accounts from the instruction
3. Validate   Run checks, verify signatures, enforce policy
4. Execute    Mutate state in a controlled phase
5. Record     Capture a StateReceipt of what changed
6. Verify     Assert invariants and compatibility
7. Inspect    Use the CLI to explain, diff, and plan migrations
```

You can use less of it for simple programs (a basic vault needs 1-4) and
more of it for complex protocols (a multi-segment treasury uses all seven).
The pipeline is always the mental model.

## State Layouts

State is defined with `hopper_layout!`:

```rust
hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}
```

This generates:

- A `#[repr(C)]` struct with alignment-1 wire types (no padding, no platform variance)
- A deterministic 8-byte `LAYOUT_ID` (SHA-256 fingerprint of type + fields)
- Canonical whole-layout accessors: `load()` / `load_mut()`
- Specialized validation helpers such as `load_foreign()` and `load_versioned()`
- Low-level `overlay()` / `overlay_mut()` helpers for explicit slice-driven access
- `SIZE`, `LEN`, `DISC`, `VERSION` constants
- `BUMP_OFFSET` for PDA verification

Every field is a fixed-size byte-backed type. No heap. No serialization.
The struct is laid directly on top of account bytes via pointer cast.

## The 16-Byte Header

Every Hopper account starts with a standard header:

```
[0]       disc        u8        Account type discriminator
[1]       version     u8        Layout version
[2..4]    flags       u16 LE    Status flags (frozen, segmented, etc.)
[4..12]   layout_id   [u8;8]    SHA-256 fingerprint
[12..16]  reserved    [u8;4]    Reserved
```

The header makes every account self-describing. Any tool can decode the
type, version, and fingerprint without knowing the layout definition.
This is what powers `hopper explain` and `hopper inspect`.

## One Access System

Hopper is easiest to reason about when access is treated as one system with
different guarantees, not multiple frameworks.

**Validated whole-layout access (default).** Full pipeline: validation,
fingerprints, receipts, tooling.

```rust
let vault = Vault::load(account, program_id)?;
```

**Direct typed slice access.** Direct typed view, no header validation.
For hot paths where you need the cast without the checks.

```rust
let vault = pod_from_bytes::<Vault>(data)?;
```

**Explicit raw escape hatch.** Raw cast, caller owns all risk.

```rust
let vault = unsafe { Vault::load_unchecked(data) };
```

The cast itself costs ~8 CU in each case. The difference is what validation
runs before the cast and what tracking runs after it. Most programs use the
validated path. Direct typed slices are for already-proven data. Raw access is
the explicit unsafe escape hatch.

See [MEMORY_ACCESS.md](MEMORY_ACCESS.md) for the full doctrine.

## Specialized Validation Helpers

Hopper keeps one whole-layout loading path and exposes specialized helpers when
the guarantee changes:

| Helper | What changes | Use case |
|--------|--------------|----------|
| `load()` / `load_mut()` | default full Hopper validation | Own program accounts |
| `load_foreign()` / `load_foreign_multi()` | foreign ownership and ABI proof | Cross-program reads |
| `load_compatible()` / `load_versioned()` | version compatibility instead of exact identity | Migration windows |
| `load_unchecked()` | caller owns validation | Benchmarks, init-time writes |
| `load_unverified()` | best-effort tooling read | Indexers, tooling |

`load()` is the default. `load_foreign()` enables cross-program reads without
crate dependencies via `hopper_interface!`. `load_compatible()` and
`load_versioned()` are for migration rollouts where a single instruction must
accept more than one layout version. Trust profiles (`strict`, `compatible`,
`read_only`, `observational`) remain additional configuration over the same
underlying loading story.

At the raw runtime layer, the equivalent Hopper-first helpers are
`account.load_versioned::<T>()`, `account.load_foreign::<T>()`, and
`account.layout_info()`.

## Validation and Checks

Hopper provides two validation styles. Both are in the prelude.

**Guards** (free functions, return `ProgramResult`):

```rust
require_signer(depositor)?;
require_owner(pool, program_id)?;
require_writable(pool)?;
```

**Core checks** (free functions, return `ProgramResult`):

```rust
check_account(pool, program_id, 1, Pool::SIZE)?;
check_has_one(vault.authority.as_bytes(), signer)?;
verify_pda(expected_key, &seeds, bump, program_id)?;
```

**Chainable checks** (methods on `AccountView`, return `Result<&Self>`):

```rust
pool.check_signer()?.check_writable()?.check_owned_by(program_id)?;
```

For complex validation, use `ValidationGraph`:

```rust
let mut graph = ValidationGraph::<8>::new();
graph.add("signer", check_signer(depositor));
graph.add("owner", check_owner(pool, program_id));
graph.add("writable", check_writable(pool));
graph.run_all()?;
```

The validation graph names each check so failures are identifiable in logs.

## Policy and Capabilities

Every instruction declares what it does through capabilities and what
validation that triggers through policy:

```rust
// Use a named policy pack (ships with Hopper):
const DEPOSIT_CAPS: CapabilitySet = TREASURY_WRITE_CAPS;

// Resolve requirements at const time:
let reqs = TREASURY_WRITE_POLICY.resolve(&DEPOSIT_CAPS);
// reqs.has(PolicyRequirement::Authority)          -> true
// reqs.has(PolicyRequirement::LamportConservation) -> true
// reqs.has(PolicyRequirement::StateSnapshot)       -> true
// reqs.has(PolicyRequirement::InvariantCheck)      -> true
```

Named packs for common patterns:

| Pack | Triggers |
|------|----------|
| `TREASURY_WRITE` | Authority + snapshot + lamport conservation + invariants |
| `JOURNAL_TOUCH` | Authority + journal capacity + snapshot |
| `EXTERNAL_CALL` | CPI guard + post-mutation check + snapshot |
| `SHARD_MUTATION` | Authority + snapshot + invariants |
| `MIGRATION_SENSITIVE` | Authority + rent exemption + snapshot + invariants |
| `AUTHORITY_CHANGE` | Authority + CPI guard + post-mutation check + invariants |

Each pack is a `const` pair: `*_POLICY` (requirement bindings) and
`*_CAPS` (capability set). You can also build custom policies with
`InstructionPolicy::new().when(cap, req)`.

## Phased Execution

Hopper uses typestate to enforce execution phases:

```rust
let frame = Frame::resolve(accounts)?
    .validate(|ctx| { /* checks */ })?
    .execute(|ctx| { /* mutations */ })?;
```

The compiler prevents calling `.execute()` before `.validate()`. Phases
map directly to the pipeline: Resolve (step 2), Validate (step 3),
Execute (step 4).

## State Receipts

After mutation, capture what changed:

```rust
let mut receipt = StateReceipt::<256>::begin(&Vault::LAYOUT_ID, buf);
// ... mutate ...
receipt.commit_with_segments(buf, &segments);
receipt.set_invariants(passed, count);
receipt.set_policy_flags(DEPOSIT_CAPS.bits());
emit_slices(&[&receipt.to_bytes()]);
```

The 64-byte receipt encodes:

- Before/after fingerprints (FNV-1a)
- Changed byte count and field regions
- Resize detection (old/new sizes)
- Segment change mask
- Invariant pass/fail summary
- Policy flags (which capabilities were declared)
- Journal append count
- CPI invocation count
- Committed flag

Receipts are the signature Hopper artifact. Every serious mutation can
produce a receipt that explains what changed, why it was allowed, and
whether the account remains compatible.

Decode receipts with `hopper receipt <hex>`.

## Segments and Roles

Complex accounts can be divided into segments:

```rust
hopper_layout! {
    pub struct PoolState, disc = 1, version = 1 { ... }
}
hopper_layout! {
    pub struct PoolConfig, disc = 2, version = 1 { ... }
}
```

Each segment has a role that carries semantic meaning:

| Role | Meaning | Migration behavior |
|------|---------|-------------------|
| Core | Primary state | Must preserve |
| Extension | Optional extra fields | Must preserve |
| Journal | Append-only log | Clearable on migration |
| Index | Derived lookup structure | Rebuildable |
| Cache | Cached/precomputed data | Rebuildable |
| Audit | Immutable audit trail | Must preserve |
| Shard | Partitioned data | Must preserve |

Roles reduce cognitive load. When someone reads your code, they know
a Journal segment is append-only and clearable. They know a Cache segment
can be rebuilt. The migration planner uses roles to classify what must be
preserved, what can be cleared, and what can be rebuilt.

## Fingerprints and Compatibility

Every layout has a deterministic `LAYOUT_ID`:

```
sha256("hopper:v1:Vault:1:authority:[u8;32]:32,balance:WireU64:8,bump:u8:1,")[..8]
```

This fingerprint lets any tool verify that account data matches
expectations without parsing the full layout. Compatibility checking is
built in:

```rust
// Is V2 a strict superset of V1?
assert!(is_append_compatible(&v1_manifest, &v2_manifest));

// Can V2 readers still parse V1 data?
assert!(is_backward_readable(&v1_manifest, &v2_manifest));
```

The migration planner generates step-by-step plans:

```
hopper plan @v1.json @v2.json

Migration: Vault v1 -> v2
  Policy: AppendOnly
  Steps:
    1. Realloc from 57 to 73 bytes
    2. CopyPrefix 57 bytes
    3. ZeroInit bytes 57..73
    4. UpdateHeader (version, layout_id)
```

## Invariants

Post-mutation correctness checks:

```rust
let mut invariants = InvariantSet::new();
invariants.check(
    vault.total_deposit.get() >= vault.total_withdrawn.get(),
    BalanceInvariantViolation::CODE,
);
invariants.finalize()?; // returns first failure as ProgramError
```

Invariant results are recorded in receipts so tooling can verify that
every mutation passed its correctness checks.

## Collections

Hopper ships 8 zero-copy collections that live directly in account data:

- **FixedVec** -- fixed-capacity vector
- **CircularBuffer** -- ring buffer with wrap-around
- **PackedMap** -- key-value map in contiguous bytes
- **SortedVec** -- always-sorted vector
- **Bitfield** -- compact bit flags
- **SlabAllocator** -- fixed-size block allocator
- **Journal** -- append-only log with circular wrap
- **VersionedField** -- field with version tag

All collections are `no_std`, `no_alloc`, and operate on `&[u8]` /
`&mut [u8]` slices.

## CLI Tooling

Hopper includes a CLI for inspecting, comparing, and planning:

```
hopper explain <hex>           Human-readable account explanation
hopper inspect <hex>           Raw header decode
hopper segments <hex>          Segment registry map with roles
hopper receipt <hex>           Decode a 64-byte state receipt
hopper compat <v1> <v2>        Compatibility report
hopper diff <v1> <v2>          Field-level diff
hopper plan <v1> <v2>          Migration plan with steps
hopper schema-export           Schema format reference
```

`explain` is the killer command. It tells you what an account is, how
it is structured, which segments exist, what roles they play, and whether
the account is migration-ready. Combined with `receipt`, you can trace
exactly what happened to an account in any transaction.

## Cross-Program Interfaces

Hopper accounts are self-describing. Any program can read another
program's accounts by verifying the header:

```rust
hopper_interface! {
    ExternalVault, expected_owner = "VaultProgramId...", layout_id = [...];
}
```

This generates a read-only overlay that checks the owner and layout_id
but requires no crate dependency on the source program.

## Error Handling

Define sequential error codes with `hopper_error!`:

```rust
hopper_error! {
    base = 6000;
    PoolFrozen,
    UnauthorizedAdmin,
    DepositExceedsMax,
}
```

Each variant becomes a struct with a `CODE` constant and `Into<ProgramError>`
impl. No panics on-chain. Every error path returns a specific code.

## Design Principles

1. **Bytes first.** Think in offsets and wire formats, not abstractions.
2. **Pipeline model.** Define, Resolve, Validate, Execute, Record, Verify, Inspect.
3. **Compile-time safety.** Typestate, const generics, and deterministic hashing over runtime checks.
4. **Zero hidden cost.** No allocations, no trait objects, no dynamic dispatch on-chain.
5. **Self-describing accounts.** The 16-byte header makes every account inspectable.
6. **Append-only evolution.** New fields extend layouts. Old data stays valid.
7. **Rigid where safety matters, flexible where architecture matters.**

## Where to Go Next

- [README.md](../README.md) -- quick start and comparison table
- [MEMORY_ACCESS.md](MEMORY_ACCESS.md) -- memory tier doctrine and performance
- [UNSAFE_INVARIANTS.md](UNSAFE_INVARIANTS.md) -- every unsafe block cataloged
- [ARCHITECTURE.md](ARCHITECTURE.md) -- crate structure and module map
- [hopper-showcase](../examples/hopper-showcase/src/lib.rs) -- canonical example
