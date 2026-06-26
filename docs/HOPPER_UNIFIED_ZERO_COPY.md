# Hopper: One Unified Zero-Copy System

Hopper's three internal tiers (hot-path account bytes, on-chain registry,
off-chain artifacts — see [`THREE_TIER_METADATA.md`](THREE_TIER_METADATA.md))
are an *implementation* detail. The developer faces **one** system: declare a
layout once, and the loader, the on-chain registry row, the field offsets, the
schema export, and the upgrade gate all derive from that single declaration.

This note describes the unification — the `AccountDescriptor` /
`LayoutDescriptor` one-source-of-truth model — and grounds it against the
2026 Solana zero-copy landscape (Anchor v2, Pinocchio, direct mapping,
SIMD-0219/0268/0339).

## The single developer-facing model

A program author writes two macros and nothing else:

```rust
// One layout declaration. Compact (1-byte disc) hot path by default.
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Vault {
    #[role = "authority"] pub authority: Address,
    #[role = "balance"]   pub balance: WireU64,
}

// One program-level profile. `governed` gates upgrades on the on-chain registry.
#[hopper::program(manifest = "governed")]
mod vault_program { /* ... */ }
```

From that, the macro emits — with **no hand-written glue** — the zero-copy
load helpers, the per-field absolute offsets (folding in the single
discriminator byte), the Tier-2 registry row, the schema export, and the
`LayoutDescriptor` impl that ties them together. Headered layouts
(`#[hopper::state(disc = N, version = V)]`) emit the *same*
`LayoutDescriptor` surface; the only difference is the body offset
(`HEADER_LEN` vs `COMPACT_BODY_OFFSET`). The developer chooses compact vs
headered per struct and otherwise writes identical code.

## One source of truth: `AccountDescriptor`

`hopper_core::manifest::AccountDescriptor` is a pure `const` value that
captures everything the rest of the system needs to know about a layout:

```rust
pub struct AccountDescriptor {
    pub name:        &'static str,
    pub name_hash:   [u8; 8],
    pub disc:        u8,
    pub version:     u16,
    pub body_size:   u32,
    pub min_size:    u32,
    pub body_offset: u8,    // COMPACT_BODY_OFFSET (1) or HEADER_LEN (16)
    pub layout_id:   [u8; 8],
    pub flags:       u32,   // ENTRY_FLAG_COMPACT | ENTRY_FLAG_HEADERED | ...
}
```

Two `const fn` constructors cover both shapes:

- `AccountDescriptor::compact(name, disc, version, body_size, layout_id)` —
  `body_offset = COMPACT_BODY_OFFSET = 1`, `flags = ENTRY_FLAG_COMPACT`,
  `min_size = 1 + body_size`.
- `AccountDescriptor::headered(name, disc, version, body_size, layout_id)` —
  `body_offset = HEADER_LEN = 16`, `flags = ENTRY_FLAG_HEADERED`,
  `min_size = 16 + body_size`.

`.with_dynamic_tail()` and `.deprecated()` are `const` builders that flip the
corresponding entry flag for variable-length and retired layouts.

Everything downstream is *derived* from this one value:

| Consumer            | Derivation                                        |
|---------------------|---------------------------------------------------|
| Tier-2 registry row | `descriptor.registry_entry() -> AccountLayoutEntry` |
| Hot-path validation | `descriptor.validate(data)` (len + disc, no registry read) |
| Field offsets       | `body_offset` is the absolute base for `{FIELD}_ABS_OFFSET` |
| Governed upgrade    | `diff_descriptors_vs_registry(&descriptors, &onchain)` |

Because the registry row, the loader's validation, and the offsets all read
from the same `const`, they cannot drift apart — there is no second place to
update.

## The `LayoutDescriptor` trait

Both compact and headered layouts implement one trait:

```rust
pub trait LayoutDescriptor {
    const DESCRIPTOR: AccountDescriptor;

    fn registry_entry() -> AccountLayoutEntry {
        Self::DESCRIPTOR.registry_entry()
    }

    #[inline(always)]
    fn validate_hot(data: &[u8]) -> Result<(), ProgramError> {
        Self::DESCRIPTOR.validate(data)
    }
}
```

The macro emits `impl LayoutDescriptor for #name` in user code, referencing
`::hopper::manifest::AccountDescriptor::{compact, headered}`. No blanket impl,
no crate cycle: `hopper-core` owns the type, the macro emits the impl. This is
purely additive — the existing `CompactLayout`, `LayoutContract`, and
`SchemaExport` surfaces are untouched.

## Hot path: zero manifest reads, zero CU regression

`validate_hot` (and the `AccountDescriptor::validate` it delegates to) is
`#[inline(always)]` and does exactly two checks:

1. `data.len() < min_size` → `AccountDataTooSmall`
2. `data[0] != disc` → `InvalidAccountData`

No registry fetch, no layout_id comparison, no epoch read. The descriptor is a
`const`, so the discriminator and minimum size are immediates at the call site.
The compact hot path is unchanged from the pre-unification cost: `check_owner`
+ `check_len` + `check_disc` + cast-at-offset-1.

## Off the hot path: governed upgrade gate

A `governed` program proves an upgrade is safe by comparing its *generated*
descriptors against the *on-chain* registry — without leaving the no-alloc
model:

```rust
let descriptors = [<Vault as LayoutDescriptor>::DESCRIPTOR];
let onchain     = read_registry(account_data)?;       // ProgramManifestView
let compat      = diff_descriptors_vs_registry(&descriptors, &onchain);
assert!(ManifestProfile::Governed.permits_upgrade(compat));
```

`diff_descriptors_vs_registry` walks both the descriptor slice and the
zero-copy view in place (no allocation) and classifies the change:

- on-chain disc with **no** matching descriptor → `Breaking` (a layout was removed)
- descriptor with **no** on-chain row → `Additive` (a new layout)
- matched disc → `diff_entry(onchain_row, &descriptor.registry_entry())`
- result is the `worst()` (severity-max) across all rows

`ManifestProfile::Governed` admits only `Unchanged` / `Additive`;
`onchain` / `offchain` admit everything short of `Breaking`. This is the live
ABI-drift gate: a redeploy that would silently break a layout the chain still
advertises fails closed at the upgrade instruction, not in production.

## Why this matters now (2026 landscape)

The unification is grounded in where Solana zero-copy is actually heading
(full analysis in `HOPPER_UNIFIED_RESEARCH.md`):

- **Anchor v2 is Pinocchio-backed and zero-copy-by-default.** The historical
  "Anchor is slow / heavy" wedge is gone. Hopper's durable advantage is *not*
  raw speed — it is the **validation contract as one system**: a single
  descriptor that simultaneously drives the loader, the on-chain registry, the
  client decode fingerprint, and the upgrade gate. Competitors make the
  developer wire those together by hand.

- **Direct account mapping is live on mainnet.** Reads are ≈ free, the first
  write triggers a copy-on-write copy, and *growth* is the expensive operation.
  This rewards compact fixed-size hot layouts (Tier 1) and makes the
  `setLoadedAccountsDataSizeLimit` story matter — the descriptor's `min_size`
  is the natural source for an auto-emitted data-size limit.

- **SIMD-0219 / 0268 / 0339** continue to tighten the cost model around loaded
  data size and account access. A single descriptor that knows each layout's
  exact `min_size` and shape flags is the right place to compute those limits
  once and feed both the on-chain validator and the off-chain client.

- **Fingerprint-pinned client decode.** The registry's `schema_hash` /
  `registry_hash` already pin off-chain artifacts to an on-chain schema; the
  descriptor's `layout_id` is the per-type fingerprint a generated client
  checks before zero-copy-decoding an account it fetched.

## What this change lands

Additive, fully tested, `no_std` / zero-copy clean:

1. `AccountDescriptor` + `LayoutDescriptor` in `hopper_core::manifest` — the
   one-source-of-truth type and trait, with `const` constructors for compact
   and headered shapes and `#[inline(always)]` len+disc validation.
2. `diff_descriptors_vs_registry` — no-alloc generated-vs-on-chain comparison
   for the governed upgrade gate.
3. Macro emission (`hopper-macros-proc`): both `#[hopper::state(compact, ...)]`
   and `#[hopper::state(...)]` (headered) emit `impl LayoutDescriptor`, and the
   compact `registry_entry()` now delegates to the descriptor so there is a
   single row builder.
4. Tests: descriptor shape/consistency, single-source registry-entry equality,
   len+disc-only validation, governed diff classification, and end-to-end
   "one descriptor feeds loader + registry + offsets" in both the compact and
   headered examples, plus a trybuild pass exercising the trait const.

## Next concrete steps

- Auto-emit `setLoadedAccountsDataSizeLimit` from each descriptor's `min_size`
  (sum over the accounts an instruction touches) in the generated client.
- Surface `layout_id` as the client-side decode fingerprint in the TypeScript
  / Rust SDKs so a fetched account is checked against the descriptor before a
  zero-copy cast.
- Extend `diff_descriptors_vs_registry` with a dynamic-tail-aware size policy
  (a grown *tail* is `Additive`, a grown *fixed prefix* is `MigrationRequired`).
