# Solana programs built with Hopper

Hopper supplies the execution and authoring layers for a complete Solana program:
entrypoint, instruction decoding, account validation, zero-copy access, PDA
signing, cross-program calls, account lifecycle, and client metadata.

## From a transaction to an on-chain action

1. Solana loads the program and serializes the instruction's accounts.
2. `hopper-native` parses that memory and resolves duplicate account references.
3. `hopper-runtime` exposes guarded views, checked CPI, and layout contracts.
4. Generated account bindings validate the handler's declared constraints.
5. The handler enforces application rules and updates state or invokes programs.
6. Solana validates the result and commits the transaction atomically, or rolls
   back its state changes on failure. Transaction fees still apply to failures.

A handler can create accounts, transfer SOL, move tokens, mint, burn, close
accounts, and invoke application-specific instructions. Zero-copy describes its
state access; it does not restrict its ability to perform those actions.

## Layers and ownership

| Layer | Crates | Responsibility |
|---|---|---|
| Native execution | `hopper-native` | Loader ABI, account memory, entrypoints, syscall wrappers, PDA and native CPI machinery |
| Runtime | `hopper-runtime` | Borrow guards, layout contracts, checked CPI, safe lifecycle operations, tracked-write policies |
| Program authoring | `hopper-lang`, `hopper-systems`, `hopper-macros`, `hopper-derive` | Typed accounts, handlers, constraints, state declarations, collections, dispatch |
| Solana integrations | `hopper-system`, `hopper-token`, `hopper-token-2022`, `hopper-associated-token`, `hopper-metaplex` | System and token instructions, account readers, mint/extension policies, asset integrations |
| Tooling | `hopper-schema`, `hopper-cli`, client generators | Program descriptions, inspection, build/deploy workflows, clients |

The native crate has no dependencies. The runtime depends directly on it.
`hopper-lang` is the umbrella API. Host tools and testing crates are separate
from the deployed execution path. Keep explicit unsafe access confined to the
parts of an application that need and can review it.

## Assets follow Solana's ownership rules

- A wallet's SOL is debited through a System Program CPI with its signature.
- A validated program-owned vault can pay SOL through checked lamport updates.
- SPL Token and Token-2022 balances change through their owning token programs.
- PDA authority comes from seeds supplied to a signed CPI; it does not permit
  writing another program's data directly.
- Release incompatible account borrows before CPI. Validate the target program,
  account identities, signer/writable privileges, mint, and token authority.

These rules follow the [Solana account model](https://solana.com/docs/core/accounts)
and [CPI model](https://solana.com/docs/core/cpi). The funded escrow and treasury
examples exercise the asset movements, not just application counters.

## State is a choice, not the whole framework

The usual authoring path is `#[account]`, `#[derive(Accounts)]`, `#[program]`,
`Ctx<T>`, and `ctx.accounts.*`. Use named initialization inputs for fixed fields.
Reach for the systems layer when explicit segment geometry or raw wire layout
is useful.

Headered account layouts start with 16 bytes: discriminator, version, flags,
8-byte layout ID, and schema epoch. Compact layouts use `[disc][body]` and keep
layout identity in metadata. The chosen contract determines loading checks.
Fixed fields use alignment-safe wire representations. Dynamic fields have
bounded codecs or deliberate final-tail semantics.

A raw `overlay` projects an already-borrowed slice. It does not establish account
ownership or application authority. Use typed account bindings or validated
loaders at instruction boundaries; validate every segment header before manually
projecting an independently declared segment.

## Concurrency and policies

Solana locks whole writable accounts. Separate accounts can enable independent
transactions; separate byte ranges inside one account do not. Account placement
therefore trades contention against rent, account metas, and lifecycle costs.

Tracked write policies constrain selected program-side data and lamport paths.
They complement application authorization and checked CPI. They are not a
sandbox for every instruction a downstream program can execute.

Ordinary program execution does not require an indexer, remote policy service,
or an offline verifier. Optional receipts and inspection tools help development
and operations. Features requiring cluster gates must be enabled only after
checking the target cluster's activation state.

## Wire Format

### Default/headered account header (16 bytes)

Every headered Hopper account begins with the same 16-byte header:

```
Offset  Size  Field        Description
------  ----  ----------   -----------
0       1     disc         Account discriminator (unique per type)
1       1     version      Layout version (starts at 1)
2       2     flags        Status flags (u16 LE)
4       8     layout_id    SHA-256 fingerprint of the layout (first 8 bytes)
12      4     schema_epoch Schema evolution epoch (u32 LE, default 1)
```

`HEADER_LEN = 16`. `HEADER_FORMAT = 1`. Compact accounts deliberately omit
this header and store only a one-byte discriminator followed by the zero-copy
body.

**Layout identity depends on the declaration API.** Current `#[account]`
and `#[hopper::state]` proc macros hash an ordered wire descriptor with SHA-256
and keep its first eight bytes. For a fixed layout its input has this shape:

```text
hopper:wire:v2|S:<Name>|V:<version>|f0:<field>:<wire_stem>|f1:<field>:<wire_stem>
```

There is no trailing separator. A dynamic tail appends `|tail:<schema-or-type>`.
Path prefixes are normalized; array lengths and relevant generic arguments
participate in the wire stem. The phantom tag in `TypedAddress<Tag>` is omitted.
Use the emitted `LAYOUT_ID` and manifest instead of reconstructing the hash
from a Rust type's display spelling.

Older declarative/manual APIs can use `hopper:v1:...` descriptors or an
explicitly supplied identity. These are separate contracts, not interchangeable
hash formulas. The proc macro does not recursively inspect arbitrary user-defined
field types: changing a nested type or a type alias still requires deliberate
versioning and compatibility review. A matching fingerprint does not prove
business invariants or migration safety.

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

### Whole-Layout Loading

Hopper keeps one canonical whole-layout loading path and exposes specialized
helpers when the guarantee changes:

| Helper | Checks |
|--------|--------|
| `load()` | owner + disc + version + layout_id + exact size |
| `load_foreign()` | expected foreign owner + layout_id + exact size |
| `load_compatible()` / `load_versioned()` | owner + disc + compatible version + min size |
| `load_unchecked()` | caller-owned validation |
| `load_unverified()` | best-effort tooling read |

These are generated per layout by the `hopper_layout!` macro, but they all map
back to the same Hopper account access model.

### Verified Accounts (`account/verified.rs`)

Headered DSL, modifier, and migration wrappers project through
`HopperLayout::OVERLAY_OFFSET`. Declarative layouts include the header at offset
zero; proc-macro states contain only the body and use offset 16. Projection
retains the original borrow guard and checks its bounds.

`VerifiedAccount<'a, T>` and `VerifiedAccountMut<'a, T>` are size-checked overlay
wrappers. Their public constructors check byte length only; they do not check
ownership, headers, PDA derivation, signer authority, or write policy. Tiered
loaders perform their own validation before constructing these wrappers. Holding
one alone does not prove authorization.
They can expose `&T` / `&mut T`, but those references are tied to the wrapper,
and the wrapper owns the borrow guard or size-checked slice. Use `with()` /
`with_mut()` when you want closure-shaped guard access; use generated segment
accessors for the default hot path. Methods: `get()`, `get_mut()`, `with()`,
`with_mut()`, `map()`, `overlay_at()`.

### Lifecycle (`account/lifecycle.rs`)

- `zero_init(data)` -- memset to zero (required before header write)
- `safe_close(account, destination)` -- zero data + transfer lamports
- `safe_close_with_sentinel(account, destination)` -- writes `0xFF` sentinel to
  prevent account revival via rent-exempt deposit
- `safe_realloc(account, new_size, payer)` -- handles rent delta

### Dynamic Tail Payloads (`tail.rs`)

Variable-length account metadata lives behind pretty dynamic fields in
`#[hopper::account]`, explicit `#[hopper::dynamic_account]`, or the explicit
`#[hopper::state(dynamic_tail = T)]` path:

```text
[ Hopper header ][ fixed account body ][ tail_len: u32 LE ][ encoded tail ]
```

The fixed body remains the zero-copy hot path. `#[hopper::account]` lets authors
write bounded `String<'a, N>` and `Vec<'a, T, N>` fields inline, then generates
the compact tail struct, view, owned editor, extension trait, and allocation
constants. `#[hopper::dynamic_account]` keeps the explicit `#[tail(...)]`
systems-mode spelling. `Address` / `Pubkey` vectors keep borrowed-slice views;
other vectors return `HopperVec<T, N>`. The explicit path uses `tail_read` /
`tail_write` with a bounded `TailCodec` payload.
`HopperString<N>` and `HopperVec<T, N>` cover the common string/list cases
without pulling heap allocation into SBF builds. `hopper_dynamic_fields!` lowers
`string<N>` to `HopperString<N>` and `vec<T, N>` to `HopperVec<T, N>` for custom
explicit tails.

Use dynamic tails for small bounded payloads that are read or written as one
logical unit. Use named extension segments or companion accounts for larger
regions that need independent borrow tracking, migration role metadata, or
collection-specific update paths.

### Bounded Dynamic Instruction Args

Authored handler arguments decode through `DecodeInstructionArg` over the bytes
after the one-byte instruction discriminator. Fixed scalars use their native
little-endian width. `HopperString<N>` and `HopperVec<T, N>` use the same
`TailCodec` encoding as account tails, so on-chain handlers and generated
clients share one deterministic bounded wire contract for small dynamic inputs.

Use a final `&[u8]` argument only when the protocol deliberately wants an opaque
remaining payload. Prefer `HopperString` / `HopperVec` for typed labels, signer
sets, memo fields, and other bounded dynamic instruction data.

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

Single-u32 compare: reads the RuntimeAccount 4-byte prefix
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

Phase markers are zero-sized types; there is no runtime phase tag. Transition methods still perform account-count checks and execute the caller's resolution, validation, and execution closures. Their checks, borrows, mutations, and syscalls have runtime cost. Typestate enforces ordering; it does not make those operations free. The six conceptual phases:

1. **Resolve** -- parse accounts from the instruction
2. **Validate** -- run checks, check signer privilege, verify PDAs
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

`emit_event<T: Pod>()` emits a Pod type via `sol_log_data`. A 32-byte payload
measured 240 CU net in the dated 2026-07-09 primitive fixture; treat that as a
fixture/toolchain result, not a universal syscall constant.
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
tracking matters. `to_bytes()` produces the current 72-byte wire payload for
event emission. `DecodedReceipt::from_bytes()` accepts that format plus legacy
64-byte receipts whose failure-payload suffix is absent.

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
| `hopper_init!` | CreateAccount or Allocate/Assign CPI + zero_init + write_header |
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

## Design contracts

1. **Checked wire projections.** Stored fields must satisfy the selected
   overlay's Pod, alignment, bounds, and padding contract. Wire integers
   provide alignment-safe representations of multi-byte values.
2. **Explicit layout identity.** Use the identity emitted by the declaration
   API. Review nested types, aliases, version changes, and migration semantics;
   a fingerprint is not a recursive proof of every field's meaning.
3. **Initialized storage.** The System creation path returns zeroed allocation
   before Hopper writes its header. Manual creation and migration paths must
   initialize every byte their resulting layout requires.
4. **Deliberate evolution.** Append compatibility is one policy. Explicit typed
   migrations may transform fields and grow or shrink accounts; validate the
   old identity before interpreting old state and fund the live-rent shortfall.
5. **Optional proc macros.** The manual/core path remains available; attribute
   macros generate validation and authoring code over the same runtime.
6. **No heap by default in program paths.** The program runtime supports
   `no_std` and allocation-free APIs. Host tooling uses `std`; applications
   that opt into other dependencies must account for their own allocations.
7. **Documented unsafe boundaries.** Each unsafe operation needs a concrete
   alignment, lifetime, aliasing, or loader-input contract and appropriate tests.
8. **Body size and account size are distinct.** Generated overlays check their
   body layout. Account storage also includes its selected header/discriminator
   and any dynamic tail; `size_of::<T>()` is not a universal account-space formula.
9. **Checked errors.** Public validation returns program errors. Unsafe raw
   entrypoints require loader-valid input; malformed raw loader frames may trap.
10. **Scoped runtime state.** Borrow and write-policy registries track the current
    invocation. CPI, mutation, and lifecycle paths must honor those scopes and
    release borrows at the appropriate boundaries.

---

## Examples

Start with `hopper-showcase`. It is the canonical reference program that uses
every layer of the pipeline.

| Example | What it shows | Tier |
|---------|-------------|------|
| `hopper-showcase` | Full pipeline: layout, dispatch, phased frame, policy, receipts, invariants, segment roles | 1+2 |
| `hopper-vault` | Macro-first SOL vault: `#[account]`, `#[derive(Accounts)]`, `#[program]` | 1 |
| `hopper-escrow` | Macro-first escrow with authority checks and raw-account escape hatch | 1 |
| `hopper-treasury` | Multi-segment treasury with permissions | 2 |
| `hopper-registry` | Segmented registry with journal and virtual state | 2 |
| `hopper-migration` | V1 to V2 layout evolution with migration planner | 2 |
| `hopper-virtual-state` | Multi-account entities with VirtualState | 2 |
| `cross-program-read` | Interface pinning across two programs, zero dependencies | 2 |

---

## Test Coverage

Coverage spans unit, property, Kani/Miri/fuzz, integration, compiled-SBF, and
public-cluster evidence lanes. Counts move with the source and feature set, so
Hopper does not publish a timeless aggregate here. Use the dated release
evidence and CI commands for the exact revision under review.

---

## CLI

The commands below are the CLI's local inspection aliases. They read
hex-encoded account data and layout JSON to inspect headers, segments,
compatibility, and receipts; separate lifecycle/fetch command families can use
RPC and interact with live clusters.

```
hopper explain <hex>           Human-readable headered-account explanation
hopper inspect <hex>           Raw header decode
hopper decode <hex>            Alias for inspect
hopper segments <hex>          Segment registry map
hopper compat @v1.json @v2.json  Compatibility report (append-safe, backward-readable, migration)
hopper diff @v1.json @v2.json    Field-level diff
hopper plan @v1.json @v2.json    Migration plan with steps, byte counts, backward readability
hopper receipt <hex>           Decode and explain a 72-byte state receipt, or a legacy 64-byte receipt
hopper schema-export           Schema format reference
```
