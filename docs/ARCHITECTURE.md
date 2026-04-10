# Hopper Architecture

Canonical technical reference for the Hopper zero-copy state framework.

This document covers the pipeline model, the wire format, every public module,
the dependency graph, and the design invariants that hold them together.

## The Pipeline

Hopper is a typed state pipeline. Every program follows the same seven steps:

```
1. Define      Layout state with hopper_layout!, declare errors, register discs
2. Resolve     Parse accounts from the instruction via Frame
3. Validate    Run checks, verify signatures, enforce policy
4. Execute     Mutate state in a controlled phase
5. Record      Capture a StateReceipt of what changed
6. Verify      Assert invariants and compatibility
7. Inspect     Use the CLI to explain, diff, and plan migrations
```

Steps 1-6 happen on-chain. Step 7 happens off-chain with the CLI. Simple
programs can skip steps 5 and 6. Complex protocols use all seven.

## Tiered Learning

The framework has depth. You do not need all of it at once.

**Tier 1 (Standard Hopper):** Versioned layouts, phased execution, validation
bundles, receipts, invariants, CLI inspect. This covers most programs.

**Tier 2 (Advanced Hopper):** Segmented accounts with roles, virtual
multi-account state, migration planning, trust profiles, validation graphs,
capability-policy binding.

**Tier 3 (Escape Hatch):** Raw overlay access, `load_unchecked`, manual wire
formats, custom collections, `segment_data_mut_unchecked`. The framework steps
aside when you need it to. Hopper Native provides direct syscall access for
anything below the framework layer.

## Overview

Hopper is `#![no_std]`, zero-allocation, built on
Hopper Native, Hopper's sovereign low-level runtime substrate. Every account is a flat byte
overlay with a 16-byte self-describing header. No proc macros are required,
no heap allocations, and no trait objects in the on-chain path.

The framework is organized into concentric rings:

| Ring | Crate | Scope |
|------|-------|-------|
| 0 | `hopper-core` | ABI types, header, overlay, pod, checks, collections, state, events, CPI, frame, dispatch |
| 0 | `hopper-macros` | `macro_rules!` code generation (layout, dispatch, init, close, error, PDA, etc.) |
| 1 | `hopper-solana` | SPL Token/Mint readers, Token-2022 screening, typed CPI helpers |
| 2 | `hopper-schema` | Layout manifests, field-level diffing, migration planning (usable off-chain and on) |
| -- | `hopper-cli` | CLI tooling: explain, inspect, decode, segments, compat, diff, plan, schema-export |

All on-chain crates are `#![no_std]` with `#![deny(unsafe_op_in_unsafe_fn)]`.

```
hopper (umbrella, re-exports macros + prelude)
 |
 +-- hopper-runtime      <- hopper-native (primary), pinocchio / solana-program (compat)
 +-- hopper-core         <- hopper-runtime, sha2-const-stable
 +-- hopper-macros       <- references hopper-core / hopper-runtime paths
 +-- hopper-schema       <- hopper-core
 +-- hopper-solana       <- hopper-core, hopper-runtime, five8_const
 +-- hopper-cli (std)    <- hopper-schema
```

## Sovereign Boundary Ownership

Hopper's architecture depends on a hard split between substrate and semantics.

- `hopper-native` owns raw execution: loader parsing, duplicate-account
   resolution, `raw_input`, `raw_account`, entrypoint macros, syscall wrappers,
   lazy parsing, and the substrate `AccountView`.
- `hopper-runtime` owns Hopper semantics: typed state access, `LayoutContract`,
   `Context`, checked CPI rules, and Hopper-facing PDA ergonomics.
- `hopper-runtime::compat/*` owns every backend bridge. If a file outside
   `compat/` needs to name Pinocchio or solana-program identity directly, that is
   an architectural regression.

This keeps Hopper Native sovereign at the execution boundary while letting
Hopper Runtime stay framework-owned instead of adapter-shaped.

---

## Wire Format

### Account Header (16 bytes)

Every Hopper account begins with the same 16-byte header:

```
Offset  Size  Field        Description
------  ----  ----------   -----------
0       1     disc         Account discriminator (unique per type)
1       1     version      Layout version (starts at 1)
2       2     flags        Status flags (u16 LE)
4       8     layout_id    SHA-256 fingerprint of the layout (first 8 bytes)
12      4     reserved     Reserved for future header format versions
```

`HEADER_LEN = 16`. `HEADER_FORMAT = 1`.

**layout_id computation** (deterministic, compile-time):

```
sha256("hopper:v1:{Name}:{version}:{field_name}:{canonical_type}:{size},"...)[..8]
```

Fields appear in declaration order. Each field contributes
`"{name}:{canonical_type}:{size},"` with a trailing comma. The hash is computed
at compile time via the `sha2-const-stable` crate. Any change to name, type,
size, or field order produces a different layout_id.

### Segmented Accounts

Accounts that need variable-length regions use a segment table starting at
byte 16:

```
Offset  Size   Field
------  -----  -----------
16      2      segment_count
18      2      registry_flags
20      16*N   segment entries (SegmentEntry: id[4] + offset[4] + count[2] +
                                capacity[2] + element_size[2] + flags[2])
20+16*N ...    segment data regions
```

Segment IDs are 4-byte FNV-1a hashes of the segment name (computed at compile
time). Flags encode the segment role in the upper 4 bits and operational flags
(LOCKED, FROZEN, DYNAMIC) in the lower bits.

### Segment Roles

Seven semantic roles classify segment behavior during migration and at runtime:

| Role | Upper bits | Migration | Runtime write |
|------|-----------|-----------|---------------|
| Core | 0x0 | Must preserve | Read/write |
| Extension | 0x1 | Must preserve | Read/write |
| Journal | 0x2 | Append-only, rebuildable | Append-only |
| Index | 0x3 | Clearable, rebuildable | Read/write |
| Cache | 0x4 | Clearable, rebuildable | Read/write |
| Audit | 0x5 | Must preserve | **Immutable after init** |
| Shard | 0x6 | Must preserve | Read/write |

Audit-role segments reject mutable borrows (`segment_data_mut()` returns an
error). The escape hatch `segment_data_mut_unchecked()` exists for init-time
writes only.

---

## Ring 0: hopper-core

### ABI Types (`abi/`)

All wire-level field types are `#[repr(transparent)]` over `[u8; N]` with
`align_of == 1`. This is non-negotiable: overlay structs must work at any
byte offset without alignment padding.

| Type | Backing | Canonical name |
|------|---------|---------------|
| `WireU16` | `[u8; 2]` | `WireU16` |
| `WireU32` | `[u8; 4]` | `WireU32` |
| `WireU64` | `[u8; 8]` | `WireU64` |
| `WireU128` | `[u8; 16]` | `WireU128` |
| `WireI16` | `[u8; 2]` | `WireI16` |
| `WireI32` | `[u8; 4]` | `WireI32` |
| `WireI64` | `[u8; 8]` | `WireI64` |
| `WireI128` | `[u8; 16]` | `WireI128` |
| `WireBool` | `[u8; 1]` | `WireBool` |
| `TypedAddress<T>` | `[u8; 32]` | `[u8;32]` |

The `WireType` unsafe trait marks types that are align-1 and valid for all bit
patterns. `LayoutFingerprint` is the 8-byte layout_id type.

### Pod and Overlay (`account/pod.rs`, `account/overlay.rs`)

`Pod` is an unsafe marker trait: the type is `repr(C)`, has no padding, and
all bit patterns are valid. `FixedLayout` adds a `SIZE` const.

- `pod_from_bytes(&[u8]) -> &T` -- zero-copy cast (validates length)
- `pod_from_bytes_mut(&mut [u8]) -> &mut T` -- mutable zero-copy cast
- `overlay(data) -> &T` / `overlay_mut(data) -> &mut T` -- thin wrappers
- `overlay_at(data, offset)` / `overlay_at_mut(data, offset)` -- at offset

### Account Header (`account/header.rs`)

- `write_header(buf, disc, version, layout_id)` -- writes the 16-byte header
- `check_header(data, disc, version, layout_id)` -- full validation
- `read_version(data)`, `read_layout_id(data)`, `read_header_flags(data)`
- `AccountHeader` -- `#[repr(C)]` struct matching the wire layout

### Tiered Loading

Five trust tiers, each validating a different subset of header properties:

| Tier | Function | Checks |
|------|----------|--------|
| T1 | `load()` | owner + disc + version + layout_id + exact size |
| T2 | `load_foreign()` | owner + layout_id + exact size |
| T3 | version compat | owner + disc + version range + min size |
| T4 | `load_unchecked()` | `unsafe`, no validation |
| T5 | `load_unverified()` | best-effort for indexers/tooling |

These are generated per layout by the `hopper_layout!` macro.

### Verified Accounts (`account/verified.rs`)

`VerifiedAccount<'a, T>` and `VerifiedAccountMut<'a, T>` are proof-of-validation
wrappers. If you hold a `VerifiedAccount`, the account has passed load validation.
Methods: `get()`, `get_mut()`, `map()`, `overlay_at()`.

### Lifecycle (`account/lifecycle.rs`)

- `zero_init(data)` -- memset to zero (required before header write)
- `safe_close(account, destination)` -- zero data + transfer lamports
- `safe_close_with_sentinel(account, destination)` -- writes `0xFF` sentinel to
  prevent account revival via rent-exempt deposit
- `safe_realloc(account, new_size, payer)` -- handles rent delta

### Dynamic Fields (`account/dynamic.rs`)

Inline variable-length fields with length prefix. Alternative to segments for
1-3 small variable-length values:

- `read_dynamic_u8/u16/u32(data, offset)` -- read length + bytes
- `write_dynamic_u8/u16/u32(data, offset, value)` -- write length + bytes
- `DynamicView` / `DynamicViewMut` -- typed access

### Realloc Guard (`account/realloc_guard.rs`)

`ReallocGuard<const N>` enforces a per-instruction cumulative growth budget.
Stack-only. Prevents runaway reallocation within a single instruction.

### Cursor and Reader (`account/cursor.rs`, `account/reader.rs`)

`SliceCursor` -- sequential reader: `read_u8/u16/u32/u64/i64`, `read_bytes`,
`skip`. `AccountReader` -- header-aware reader with `header()`, `body()`,
`body_bytes()`, `u64_at()`.

---

### Validation (`check/`)

Five-layer validation hierarchy:

1. **Account-local** (`check/mod.rs`): `check_signer`, `check_writable`,
   `check_owner`, `check_size`, `check_discriminator`, `check_rent_exempt`
2. **Cross-account** (`check/mod.rs`): `check_has_one`, `check_keys_eq`,
   `keys_eq_fast` (4x u64 compare), `require_all_unique`
3. **PDA** (`check/mod.rs`): `verify_pda` (~200 CU), `verify_pda_cached`,
   `find_and_verify_pda`
4. **CPI guards** (`check/mod.rs`): `require_top_level`,
   `detect_flash_loan_bracket`, `check_no_subsequent_invocation`
5. **Composition** (`check/guards.rs`): `check_lamport_conservation`,
   `snapshot_lamports`, `check_writable_coherence`

#### Fast-path Checks (`check/fast.rs`)

Quasar-inspired single-u32 compare: reads the RuntimeAccount 4-byte prefix
(`borrow_state|is_signer|is_writable|executable`) as one u32. Saves 4-8 CU
per account. `#[cfg(target_os = "solana")]` gated.

#### Modifier Wrappers (`check/modifier.rs`)

Composable type-level wrappers where each layer validates one property:

```rust
Signer<Mut<Account<'a, Vault>>>
```

If you can construct the type, validation has passed. Zero runtime cost after
construction.

#### Validation Graph (`check/graph.rs`)

`ValidationGraph<const N>` -- stack-allocated pipeline of `ValidateFn` closures.
- `run()` -- fail-fast, returns first error
- `run_all()` -- accumulates all errors, returns first

Combinators: `require_signer_at()`, `require_writable_at()`,
`require_owned_at()`, `require_data_min()`, `require_keys_equal()`,
`require_unique()`, `require_lamports_gte()`.

`TransitionRulePack` dispatches validation by instruction tag.
`PostMutationValidator` runs after state mutation.

#### Trust Profiles (`check/trust.rs`)

`TrustProfile` with `TrustLevel`:
- **Strict** -- owner + layout_id + exact size
- **Compatible** -- min size
- **Observational** -- layout_id only

Explicit declaration of trust assumptions for foreign accounts.

---

### Collections (`collections/`)

Eight zero-copy, zero-alloc collection types. All operate directly over
`&mut [u8]` slices with inline wire headers:

| Type | Wire Header | Key Operations | Complexity |
|------|-------------|---------------|------------|
| `FixedVec<T>` | 4B count | push, pop, swap_remove, index | O(1) |
| `RingBuffer<T>` | 8B (head + count) | push, read | O(1) |
| `SlotMap<T>` | 8B (count + free_head) | insert, remove, access | O(1) |
| `BitSet` | none | get, set, toggle, count_ones | O(1) |
| `SortedVec<T>` | 4B count | binary_search, insert, remove | O(log n) / O(n) |
| `PackedMap<K,V>` | 4B count | get, insert, remove | O(n) |
| `Journal<T>` | 16B | append, read (strict/circular) | O(1) |
| `Slab<T>` | 16B + bitmap | alloc, free (double-free safe) | O(1) |

`SlotMap` uses generation counters to prevent ABA problems. `Journal` supports
strict mode (reject on full) and circular mode (overwrite oldest). `Slab` uses
an occupancy bitmap to prevent double-free.

---

### Frame / Execution (`frame/`)

#### Frame (`frame/mod.rs`)

`Frame<'a>` manages runtime mutable borrow tracking via a `u64` bitmask.
`account_mut()` checks the bit before granting exclusive access.
`FrameAccountMut` clears the bit on `Drop`. Supports up to 64 accounts.

#### Phased Execution (`frame/phase.rs`)

Typestate pattern enforcing execution phase ordering at compile time:

```
Unresolved -> Resolved -> Validated -> Executed
```

Each transition is a zero-cost move. The six conceptual phases:

1. **Resolve** -- parse accounts from the instruction
2. **Validate** -- run checks, verify signatures, verify PDAs
3. **Borrow** -- acquire mutable references
4. **Mutate** -- write account data
5. **Emit** -- fire events
6. **Commit** -- release borrows

#### Instruction Args (`frame/args.rs`)

`InstructionArgs<'a>` trait for zero-copy argument parsing with lifetime into
the instruction data buffer. `ValidateArgs` trait for independent arg validation
before account validation.

---

### Dispatch (`dispatch/`)

`dispatch_instruction()` reads a 1-byte discriminator tag from instruction data
and dispatches to the matching handler. `dispatch_instruction_u16()` for 2-byte
tags. The `hopper_dispatch!` macro generates the match statement.

### Events (`event/`)

`emit_event<T: Pod>()` serializes a Pod type via `sol_log_data` syscall (~100 CU).
`emit_event_tagged()` prepends a discriminator byte. `emit_slices()` emits raw
byte slices. Zero allocation.

### State Machine (`state/`)

`check_state_transition(current, from, to)` validates FSM transitions.
`transition_state()` validates and writes. `check_state()`, `check_state_not()`,
`check_state_in()` for guards.

### CPI (`cpi/`)

`HopperCpi<'a, ACCTS, DATA>` -- fully const-generic, stack-only CPI builder.
Uses `MaybeUninit` and `sol_invoke_signed_c`. Max 4 signers, 16 seeds each.
`HopperCpiBuf<'a, ACCTS, MAX>` for runtime-length data.

### State Diff (`diff/`)

`StateSnapshot<const SIZE>` captures a before-snapshot on the stack.
`StateDiff` computes byte-level changes: `has_changes()`, `range_changed()`,
`changed_regions::<N>()`, `field_diff_mask()`, `restore_into()`.

### Invariants (`invariant/`)

`check_invariant()` and `check_invariant_fn()` (lazy evaluation).
`InvariantSet` runs all invariants and reports the first failure.
Custom error codes per invariant via `InvariantDescriptor`.

### Migration (`migrate/`)

On-chain migration support: `migrate_append()` handles realloc + header update +
zero-fill for append-only upgrades. `MigrationDescriptor` and `MigrationKind`
(Append / SegmentAppend / Full).

### Policy (`policy.rs`)

Declarative capability/requirement system:

- `Capability` enum (10 capabilities: ReadsState, MutatesState, TouchesJournal,
  ExternalCall, MutatesTreasury, ReallocatesAccount, CreatesAccount,
  ClosesAccount, ModifiesAuthority, TransitionsState)
- `PolicyRequirement` enum (8 requirements: Authority, JournalCapacity,
  PostMutationCheck, CpiGuard, RentExemption, InvariantCheck, StateSnapshot,
  LamportConservation)
- `InstructionPolicy<const N>` maps capabilities to requirements and `.enforce()`s
  them at runtime

### Receipt (`receipt.rs`)

`StateReceipt<const SNAP_SIZE>` captures a before-snapshot and computes a diff
on commit. Tracks: `layout_id`, `changed_fields` (u64 bitmask), `changed_bytes`,
`changed_regions`, `was_resized`, `invariants_passed`, `cpi_invoked`,
`before_fingerprint` (FNV-1a of pre-mutation data), `after_fingerprint`,
`segment_changed_mask`, `policy_flags` (capability bits), `journal_appends`,
and `cpi_count`.

`commit_with_segments(data, segments)` is the preferred commit path when segment
tracking matters. `to_bytes()` produces a 64-byte wire payload for event emission.
`DecodedReceipt` provides off-chain decoding via `from_bytes()`.

### Virtual State (`virtual_state/`)

`VirtualState<const N>` maps N logical slots to physical accounts for
multi-account patterns. `VirtualSlot` declares ownership and writability
requirements per slot. `validate()` checks bounds, ownership, and writability.
`overlay()` / `map()` provide typed access across constituent accounts.

### Math (`math/`)

Checked arithmetic: `checked_add/sub/mul/div` (u64), `checked_add_i64/sub_i64`,
`scale_bps()` (basis points via u128), `scale_fraction()`, `div_ceil()`.
All return `ProgramError::ArithmeticOverflow` on overflow.

### Time (`time/`)

Timestamp guards: `check_deadline_passed/not_passed`, `check_cooldown_elapsed`,
`check_staleness`, `check_in_future/past`. All operate on `i64` Unix timestamps.

### Sysvar (`sysvar/`)

`Clock` and `Rent` structs. `read_clock()` (40 bytes), `read_rent()`.
`CachedClock` / `CachedRent` / `SysvarContext` for parse-once reuse.
Guards: `check_not_expired()`, `check_expired()`, `check_within_window()`,
`check_cooldown()`, `check_slot_staleness()`.

---

## Ring 1: hopper-solana

| Module | Purpose |
|--------|---------|
| `token.rs` | Zero-copy SPL Token field readers at fixed offsets. `token_account_mint/owner/amount/state`, `check_token_initialized/owner/mint`, `check_not_frozen`, `check_token_balance_gte`. |
| `mint.rs` | SPL Mint readers. `mint_supply/decimals`, `mint_authority/freeze_authority`, `COption<Pubkey>` handling. |
| `cpi_guard.rs` | `assert_no_cpi()`, instruction index/count helpers, token program ID checks. |
| `typed_cpi.rs` | Typed wrappers: `create_account/signed`, `transfer_sol/signed`, `token_transfer/signed`, `token_mint_to/signed`, `token_burn`. |

---

## Ring 2: hopper-schema

Off-chain and on-chain schema tooling:

- `LayoutManifest` -- name, disc, version, layout_id, total_size, fields
- `FieldDescriptor` -- name, canonical_type, size, offset
- `is_append_compatible()` -- all old fields exist at same positions in new
- `is_backward_readable()` -- V(N) code can read V(N+1) accounts (prefix match)
- `requires_migration()` -- any structural change at all
- `compare_fields::<N>()` -- per-field diff (Identical / Changed / Added / Removed)
- `MigrationPlan::<N>::generate()` -- produces policy, steps, copy/zero byte
  counts, and backward-readability flag
- `decode_header()`, `decode_segments()` -- hex data inspection

---

## Macros (hopper-macros)

All `macro_rules!`. No proc macros are required for correctness or core
functionality (see [PROC_MACRO_POLICY.md](PROC_MACRO_POLICY.md)).

| Macro | Purpose |
|-------|---------|
| `hopper_layout!` | `#[repr(C)]` struct + compile-time LAYOUT_ID + DISC/VERSION/LEN + tiered load functions + overlay + HopperLayout trait |
| `hopper_check!` | Composable constraint: `owner=, writable, signer, disc=, size>=` |
| `hopper_error!` | Sequential error code generation |
| `hopper_require!` | Assert with custom error return |
| `hopper_init!` | CreateAccount CPI + zero_init + write_header |
| `hopper_close!` | safe_close_with_sentinel wrapper |
| `hopper_dispatch!` | 1-byte tag match dispatch |
| `hopper_register_discs!` | Compile-time discriminator uniqueness |
| `hopper_verify_pda!` | PDA verify with BUMP_OFFSET (~200 CU) |
| `hopper_invariant!` | Inline invariant runner |
| `hopper_manifest!` | const LayoutManifest with field descriptors |
| `hopper_segment!` | Segmented account declaration |
| `hopper_validate!` | Inline validation pipeline |
| `hopper_virtual!` | Multi-account virtual state mapping |
| `hopper_assert_compatible!` | Compile-time layout version compat check |
| `hopper_assert_fingerprint!` | Compile-time fingerprint pinning |
| `hopper_interface!` | Cross-program read-only interface |

---

## Design Invariants

These are the rules that all code must satisfy. Violations are bugs.

1. **Align-1 wire types.** All ABI field types are `#[repr(transparent)]` over
   `[u8; N]` with `align_of == 1`. No native integers in overlay structs.

2. **Deterministic layout_id.** The SHA-256 input string is
   `"hopper:v1:{Name}:{version}:{field}:{type}:{size},"` per field in
   declaration order. Any structural change must produce a different layout_id.

3. **Zero-init before header write.** Global invariant. `hopper_init!` enforces
   this. Manual paths must call `zero_init()` before `write_header()`.

4. **Append-only versioning.** V(N+1) is a strict superset of V(N). Fields
   are never reordered or removed. New fields go at the end.

5. **No proc macros required.** All macros are `macro_rules!`. Proc macros
   are allowed for optional ergonomics (see
   [PROC_MACRO_POLICY.md](PROC_MACRO_POLICY.md)).

6. **No std, no alloc.** All on-chain crates are `#![no_std]` with zero heap usage.

7. **Every unsafe has a SAFETY comment.** Justifying alignment, length, aliasing.

8. **`size_of == LEN` and `align_of == 1` compile-time assertions** for every
   `#[repr(C)]` overlay struct.

9. **Explicit error codes.** Every error path returns a specific `ProgramError`
   or custom error code. No panics on-chain.

10. **No hidden runtime behavior.** No global state, no lazy init, no implicit
    allocations, no trait objects, no dynamic dispatch in on-chain code.

---

## Examples

Start with `hopper-showcase`. It is the canonical reference program that uses
every layer of the pipeline.

| Example | What it shows | Tier |
|---------|-------------|------|
| `hopper-showcase` | Full pipeline: layout, dispatch, phased frame, policy, receipts, invariants, segment roles | 1+2 |
| `hopper-vault` | Simple SOL vault: layout, dispatch, phased frame | 1 |
| `hopper-escrow` | Token escrow with authority checks | 1 |
| `hopper-treasury` | Multi-segment treasury with permissions | 2 |
| `hopper-registry` | Segmented registry with journal and virtual state | 2 |
| `hopper-migration` | V1 to V2 layout evolution with migration planner | 2 |
| `hopper-virtual-state` | Multi-account entities with VirtualState | 2 |
| `cross-program-read` | Interface pinning across two programs, zero dependencies | 2 |

---

## Test Coverage

| Suite | Test count | Scope |
|-------|-----------|-------|
| Unit tests | 36 | Core module-level tests |
| Property tests | 75 | Randomized invariant checking |
| Trust tests | 96 | CPI guards, collections, migration, receipts, validation, segments, backward compat, danger zone golden tests |
| Migration tests | 9 | On-chain migration paths |
| Schema tests | 4 | Manifest generation and diffing |
| Virtual-state tests | 5 | Multi-account mapping |
| **Total** | **225** | |

---

## CLI

The CLI is a host-side inspection tool. It reads hex-encoded account data and
schema manifests to verify layouts, segments, compatibility, and receipts.
It does not connect to RPC or interact with live clusters.

```
hopper explain <hex>           Human-readable account explanation
hopper inspect <hex>           Raw header decode
hopper decode <hex>            Alias for inspect
hopper segments <hex>          Segment registry map
hopper compat <v1.json> <v2>   Compatibility report (append-safe, backward-readable, migration)
hopper diff <v1.json> <v2>     Field-level diff
hopper plan <v1.json> <v2>     Migration plan with steps, byte counts, backward readability
hopper receipt <hex>           Decode and explain a 64-byte state receipt
hopper schema-export           Schema format reference
```
