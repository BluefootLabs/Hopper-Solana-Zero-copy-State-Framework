# Jiminy Framework Analysis -- Deep Source Audit

> **Version audited**: 0.16.0 (crates/jiminy-core + 10 satellite crates)  
> **Source**: `d:\tmp\jiminy\` -- full workspace with 289+ tests passing  
> **Audit scope**: Architecture, macros, ABI, safety model, DeFi primitives, schema, TypeScript tooling

---

## Executive Summary

Jiminy is a **zero-copy ABI standard library** for Solana, not a framework. It provides account layout definition, deterministic ABI fingerprinting (layout_id via SHA-256), 5-tier trust-based loading, cross-program interfaces, segmented variable-length arrays, and DeFi math primitives -- all via declarative `macro_rules!` with zero proc macros, no `std`, and no allocator.

**Key differentiators**: Deterministic layout_id, cross-program interfaces (`jiminy_interface!`), 5-tier loading, segmented layouts, declarative-only macros.

**Key weaknesses**: No discriminator uniqueness enforcement, no inline dynamic fields (only segment-based), no PDA bump caching, verbose boilerplate for layout declarations.

---

## 1. Architecture

### 1.1 Crate Decomposition (Ring-Layered)

| Ring | Crate | Purpose |
|------|-------|---------|
| 0 | `jiminy-core` | Header, overlay, pod, ABI types, checks, math, state, time, events, instructions, interfaces, segments |
| 1 | `jiminy-solana` | Token/Mint readers, Token-2022 screening, CPI guards, sysvar helpers |
| 2 | `jiminy-finance` | AMM math, slippage, oracle, Merkle, Ed25519 |
| Domain | `jiminy-lending` | Lending primitives (collateral, health factor, interest) |
| Domain | `jiminy-staking` | Staking/unstaking primitives |
| Domain | `jiminy-vesting` | Vesting schedule primitives |
| Domain | `jiminy-multisig` | Multisig logic |
| Domain | `jiminy-distribute` | Distribution trees, Merkle claims |
| Schema | `jiminy-schema` | Layout Manifest v1, canonical type normalization → JSON |
| Layouts | `jiminy-layouts` | Standard account layouts package |
| Adapter | `jiminy-anchor` | Anchor interop adapter |

### 1.2 Dependency Graph

```
jiminy (root facade)
  ├── jiminy-core (pinocchio, sha2-const-stable, five8_const)
  ├── jiminy-solana (jiminy-core, pinocchio-token, pinocchio-system)
  ├── jiminy-finance (jiminy-core)
  ├── jiminy-lending (jiminy-core, jiminy-finance)
  ├── jiminy-staking (jiminy-core, jiminy-finance)
  ├── jiminy-vesting (jiminy-core)
  ├── jiminy-multisig (jiminy-core)
  ├── jiminy-distribute (jiminy-core, jiminy-finance)
  ├── jiminy-schema (jiminy-core)
  ├── jiminy-layouts (jiminy-core)
  └── jiminy-anchor (jiminy-core, anchor-lang)
```

### 1.3 Design Principles

1. **No proc macros** -- all `macro_rules!` for full auditability
2. **No `std`, no `alloc`** -- `#![no_std]` with zero heap allocation
3. **Alignment-1 wire types** -- `LeU64`, `LeU128`, `LeBool` etc. all `#[repr(transparent)]` over `[u8; N]`
4. **Deterministic ABI** -- layout_id via SHA-256 at compile time
5. **Pinocchio-native** -- built atop pinocchio v0.10.2 for minimal CU overhead

---

## 2. Header Format (16 bytes)

```
Offset  Bytes  Field         Type
0       1      discriminator u8
1       1      version       u8
2-3     2      flags         u16 (LE)
4-11    8      layout_id     [u8; 8]
12-15   4      reserved      [u8; 4]
```

**HEADER_LEN = 16**. The header is always the first 16 bytes of every Jiminy account.

### Layout ID Computation

```
layout_id = SHA-256("jiminy:v1:<StructName>:<version>:<field_name>:<canonical_type>:<size>,...")[..8]
```

- Computed at **compile time** via `sha2-const-stable`
- Deterministic: same fields → same hash, regardless of program or crate
- 8-byte truncation gives 2^64 collision space (analysis shows ~5.4 billion layouts before 50% collision probability)

---

## 3. Zero-Copy Layout System

### 3.1 `zero_copy_layout!` Macro

The core macro. Generates from a struct declaration:

```rust
zero_copy_layout! {
    pub struct Vault, discriminator = 1, version = 1 {
        header:    AccountHeader = 16,
        authority: Address       = 32,
        balance:   LeU64         = 8,
    }
}
```

**Generates**:
1. `#[repr(C)]` struct with `Copy` + `Clone`
2. `unsafe impl Pod` + `impl FixedLayout` (with compile-time `size_of == LEN` assertion)
3. Constants: `LEN`, `DISC`, `VERSION`, `LAYOUT_ID`
4. 5-tier overlay methods: `load()`, `load_mut()`, `load_foreign()`, `load_unchecked()`, `load_unverified_overlay()`
5. Field offset constants: `OFFSET_authority`, `OFFSET_balance`
6. Borrow-splitting methods: `split_fields()` → `(FieldRef, FieldRef, ...)`, `split_fields_mut()`
7. Compile-time alignment assertion: `align_of::<T>() <= 8`

### 3.2 Layout Inheritance

```rust
zero_copy_layout! {
    pub struct VaultV2, discriminator = 1, version = 2, extends = Vault {
        header:    AccountHeader = 16,
        authority: Address       = 32,
        balance:   LeU64         = 8,
        fee_bps:   LeU16         = 2,  // New field
    }
}
```

Compile-time checks enforce: discriminator match + size(V2) >= size(V1).

### 3.3 Wire Types

| Type | Size | Alignment | Purpose |
|------|------|-----------|---------|
| `LeU8`/`LeI8` | 1 | 1 | Usually unnecessary (u8 already align-1) |
| `LeU16`/`LeI16` | 2 | 1 | Little-endian safe |
| `LeU32`/`LeI32` | 4 | 1 | Little-endian safe |
| `LeU64`/`LeI64` | 8 | 1 | **Critical** for u64 fields |
| `LeU128`/`LeI128` | 16 | 1 | **Mandatory** (u128 requires align-16 natively) |
| `LeBool` | 1 | 1 | Safe boolean wrapper |
| `Address` | 32 | 1 | Pubkey as byte array |

---

## 4. Tiered Loading (5 Tiers)

| Tier | Function | Checks | Use Case |
|------|----------|--------|----------|
| T1 | `load(account, program_id)` | owner + disc + version + layout_id + exact size | Normal account loading |
| T1m | `load_mut(account, program_id)` | Same + writable | Mutable access |
| T2 | `load_foreign(account, owner)` | owner + layout_id + exact size | Cross-program reads |
| T3 | `validate_version_compatible()` | owner + disc + version >= expected + size >= min | Migration/compat |
| T4 | `load_unchecked(data)` | **None** (unsafe) | Hot-path after manual validation |
| T5 | `load_unverified_overlay(data)` | Best-effort header check | Indexers/explorers |

---

## 5. Account Validation System

### 5.1 `check_account!` Macro

```rust
check_account!(vault,
    owner = program_id,
    writable,
    disc = Vault::DISC,
    version >= 1,
    layout_id = &Vault::LAYOUT_ID,
    size >= Vault::LEN
)?;
```

Expands to inline if-statements. Keywords map to specific check functions.

### 5.2 `check_account_strict!` Macro

Requires `owner`, `disc`, and `layout_id` -- compile error if omitted.

### 5.3 Individual Check Functions

**Identity/permissions**: `check_signer`, `check_writable`, `check_owner`, `check_pda`, `check_system_program`, `check_uninitialized`, `check_executable`

**Data shape**: `check_size`, `check_discriminator`, `check_version`, `check_account` (combined)

**Keys**: `check_keys_eq`, `check_has_one`

**Rent/lamports**: `rent_exempt_min`, `check_rent_exempt`, `check_lamports_gte`, `check_closed`

### 5.4 PDA Derivation

```rust
// Runtime (~500 CU via sol_sha256)
derive_address(&seeds, bump, program_id)

// Compile-time (zero CU)
derive_address_const(&seeds, bump, program_id)

// ATA derivation
derive_ata(wallet, mint)
```

---

## 6. Cross-Program Interfaces

### `jiminy_interface!` Macro

```rust
jiminy_interface! {
    pub struct Vault for PROGRAM_A {
        header:    AccountHeader = 16,
        authority: Address       = 32,
        balance:   LeU64         = 8,
    }
}
```

**Generates**: Read-only overlay with T2 (foreign) loading. No mutable access. Layout_id must match the original struct (struct **name** is part of hash -- must match exactly).

**Safety**: Owner check + layout_id match + exact size. Does NOT check discriminator or version (those are the owning program's concern).

---

## 7. Segmented Layouts

```rust
segmented_layout! {
    pub struct OrderBook, discriminator = 2, version = 1 {
        header:    AccountHeader = 16,
        authority: Address       = 32,
    }
    segments: {
        orders: Order,
        bids:   Bid,
    }
}
```

**Wire format**: Fixed Prefix | Segment Table (12 bytes/segment) | Segment Data

**Segment descriptor** (12 bytes): offset(u32) + count(u16) + capacity(u16) + element_size(u16) + flags(u16)

**Operations**: `push`, `swap_remove`, `iter`, random access. Capacity fixed at init.

---

## 8. Instruction Dispatch

```rust
instruction_dispatch! {
    program_id, accounts, instruction_data;
    0 => handler_init(program_id, accounts, instruction_data),
    1 => handler_deposit(program_id, accounts, instruction_data),
}
```

Simple byte-tag match. No hidden control flow.

---

## 9. Account Lifecycle

- **Init**: `init_account!(payer, account, pid, Layout)` -- create, zero-init, write header
- **Realloc**: `safe_realloc(account, new_size, payer)` -- resize + rent adjustment
- **Close**: `close_account!(account, dest)` -- write CLOSE_SENTINEL ([0xFF; 8]), transfer lamports, zero data

---

## 10. CPI & Safety Guards

### Safe CPI Wrappers

`safe_transfer_tokens`, `safe_transfer_sol`, `safe_mint_to`, `safe_burn`, `safe_close_token_account`, `safe_create_account`

### Re-entrancy Guards

- `require_top_level(data, program_id)` -- reject CPI calls
- `require_cpi_from(data, expected_caller)` -- whitelist specific caller
- `detect_flash_loan_bracket(data, program_id)` -- detect same-program sandwich
- `check_no_other_invocation(data, program_id)` -- monolithic tx enforcement
- `check_no_subsequent_invocation(data, program_id)` -- no follow-up calls

### Token-2022 Screening

`check_safe_token_2022_mint(mint)` -- rejects transfer hooks, permanent delegate, non-transferable.

---

## 11. Error Handling

```rust
error_codes! {
    base = 6000;
    Undercollateralized,  // 6000
    Expired,              // 6001
    InvalidOracle,        // 6002
}
```

Generates unit structs with `CODE: u32` constant and `Into<ProgramError>` impl. All errors are `ProgramError::Custom(N)`.

---

## 12. Math & DeFi Primitives

### jiminy-core math

`checked_add/sub/mul/div`, `checked_div_ceil`, `checked_mul_div` (u128 intermediate), `bps_of/bps_of_ceil`, `checked_pow`

### jiminy-finance

- **AMM**: `constant_product_out`, `constant_product_in`, `check_k_invariant`, `isqrt`, `price_impact_bps`
- **Slippage**: `check_slippage`, `check_max_input`, `check_min_amount`, `check_nonzero`, `check_price_bounds`
- **Ed25519**: `check_ed25519_signature` (precompile)
- **Merkle**: Proof verification for airdrops/access control

### jiminy-lending

Collateral ratio checks, health factor computation, interest accrual

### jiminy-staking, jiminy-vesting, jiminy-multisig, jiminy-distribute

Specialized primitives for each domain.

---

## 13. Schema & TypeScript Support

### jiminy-schema

Generates `LayoutManifest` JSON:
```json
{
  "manifest_version": "manifest-v1",
  "name": "Vault",
  "version": 1,
  "discriminator": 1,
  "layout_id": [0xAB, 0xCD, ...],
  "total_size": 56,
  "fields": [...],
  "segments": []
}
```

### @jiminy/ts

TypeScript decoder consumes manifests. Manual transaction builder construction required (no full codegen like Anchor).

---

## 14. All Macros (Complete)

| Macro | Purpose |
|-------|---------|
| `zero_copy_layout!` | Core layout definition + overlay methods |
| `segmented_layout!` | Variable-length array layouts |
| `jiminy_interface!` | Read-only cross-program overlays |
| `check_account!` | Composable account validation |
| `check_account_strict!` | Required-constraint validation |
| `instruction_dispatch!` | Byte-tag routing |
| `init_account!` | Create + zero-init + header write |
| `close_account!` | Sentinel + lamport transfer + zero |
| `error_codes!` | Numbered error code generation |
| `require!`, `require_keys_eq!`, `require_gte!`, etc. | Inline guard macros |
| `check_accounts_unique!` | Pairwise address uniqueness |
| `impl_pod!` | Batch Pod implementations |
| `emit!` | Event emission (up to 8 slices) |

---

## 15. Strengths

1. **Deterministic layout_id** -- content-addressed ABI fingerprinting, unforgeable cross-program identity
2. **Cross-program interfaces** -- `jiminy_interface!` with tiered trust, no crate dependency needed
3. **Declarative-only macros** -- full auditability, no proc-macro black boxes
4. **Zero allocator** -- no `std`, no `alloc`, no heap, all stack/borrow
5. **5-tier loading** -- right validation level for every use case
6. **Segmented layouts** -- first-class variable-length arrays with capacity tracking
7. **Full DeFi primitives** -- AMM, slippage, lending, staking, vesting, multisig, distribution
8. **Mechanical safety** -- compile-time size/alignment assertions, explicit field sizes
9. **Token-2022 extension screening** -- detects dangerous mint extensions
10. **CPI re-entrancy guards** -- flash-loan detection, caller whitelisting

---

## 16. Weaknesses

1. **No discriminator uniqueness enforcement** -- two layouts can share discriminator value accidentally
2. **No inline dynamic fields** -- only segment-based variable-length data (12 bytes overhead per segment vs 1-4 bytes prefix)
3. **No PDA bump caching** -- always calls `create_program_address` (~544 CU), no `BUMP_OFFSET` optimization
4. **Verbose layout declarations** -- must repeat all fields in version extensions, explicit sizes required
5. **No proc-macro convenience** -- more boilerplate than Anchor/Quasar for equivalent declarations
6. **Alignment-1 wrapper requirement** -- must use `LeU64` instead of `u64`, cognitive overhead
7. **Segments not growable** -- capacity fixed at init, require explicit realloc
8. **No TypeScript codegen** -- manual marshaling with @jiminy/ts, no full IDL-to-client pipeline
9. **No unified error code registry** -- collision between programs possible
10. **No batched account header validation** -- individual checks per account vs Quasar's u32 batch compare
11. **No binary search on collections** -- `ZeroCopySlice` is linear-scan only
12. **No context/sysvar caching** -- each sysvar read is a separate syscall
13. **No Codama IDL integration** -- schema is custom JSON, not ecosystem-standard Codama nodes
14. **Struct name in layout_id hash** -- cross-program interface names must exactly match original struct name or hash diverges

---

## 17. CU Efficiency Assessment

| Operation | Jiminy | vs Raw Pinocchio |
|-----------|--------|------------------|
| Account load (T1) | ~60 CU | +10 CU (header + layout_id check) |
| Account load (T4, unsafe) | ~50 CU | ~same |
| PDA validation | ~544 CU | same (no bump cache) |
| Field access | 0 CU | same (zero-copy overlay) |
| Segment access | ~20 CU | +20 CU (descriptor parse) |
| CPI | ~syscall | ~same |

**Key gap**: Quasar's `BUMP_OFFSET` saves ~344 CU per PDA. This is a significant per-instruction win that Jiminy doesn't exploit.

---

## 18. What Hopper Should Take from Jiminy

### MUST adopt:
- **Deterministic layout_id** -- content-addressed ABI fingerprinting is Jiminy's standout feature
- **Cross-program interfaces** -- `jiminy_interface!` pattern with tiered trust
- **5-tier loading** -- Hopper already has 4 tiers, add T5 for indexers
- **Segmented layouts** -- variable-length arrays with capacity tracking
- **Compile-time size/alignment assertions** -- mechanical safety

### MUST improve on:
- **Add PDA bump caching** -- `BUMP_OFFSET` from Quasar, ~344 CU savings per PDA
- **Add inline dynamic fields** -- prefix-based for 1-2 fields (vs 12-byte segment overhead)
- **Add discriminator uniqueness** -- registry macro or compile-time check
- **Add binary search** -- on sorted `ZeroCopySlice` variants
- **Add context caching** -- sysvar cache from Star Frame pattern
- **Add batched header validation** -- Quasar's u32 batch compare technique
- **Integrate Codama** -- ecosystem-standard IDL for multi-language clients

### MUST NOT copy:
- Proc macros (Jiminy's biggest advantage is auditability)
- `std`/`alloc` dependency (Star Frame's weakness)
- Borsh for instruction data (CU overhead)
