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

## Anchor v2-informed descriptor tooling

Anchor v2 is Pinocchio-backed and zero-copy-by-default, so the speed/DX gap is
gone. The ideas worth taking are about *coherence* — typed account validation,
fail-closed client decode, and tooling that cannot describe a different layout
than the program runs. Hopper adapts each in its own descriptor-native way
(not by copying Anchor syntax or internals). Every API below derives from the
one `AccountDescriptor` and is `const` / `no_std` / no-alloc.

### Client decode fingerprint (`LayoutFingerprint`)

`AccountDescriptor::fingerprint() -> LayoutFingerprint` is a deterministic
16-byte identity over the *wire-identity* fields (name, disc, version, sizes,
body offset, shape flags, `layout_id`) — never the `deprecated` lifecycle bit,
so deprecating a layout never changes how it decodes. A generated SDK embeds
`LayoutFingerprint::to_hex()` (32 ASCII bytes) as a constant and **fails closed**
if the layout the program advertises for a discriminator does not match:

```rust
let fp = <Vault as LayoutDescriptor>::fingerprint();   // const, embeddable
// TS/Kotlin/Rust SDK: before casting fetched bytes, compare the embedded
// fingerprint to the one computed from the program's on-chain registry row.
// Mismatch ⇒ refuse to zero-copy-decode (the program was redeployed with a
// different layout at this disc) rather than read stale/mis-shaped memory.
```

This is the per-type complement to the registry's `schema_hash` / `registry_hash`
(which pin the whole artifact set): the fingerprint pins one account type.

### Loaded-accounts data-size budgeting

`min_loaded_data_size(&descriptors)` sums each layout's `min_size`;
`recommend_loaded_data_limit(&descriptors, tail_headroom, extra)` adds headroom
per dynamic-tail layout plus a flat margin. A transaction builder sizes
`setLoadedAccountsDataSizeLimit` directly from the descriptors of the accounts
an instruction touches — sourced from the same `min_size` the loader enforces,
so the limit and the validation can never disagree. Both are saturating; the
caller clamps to the runtime maximum.

### Dynamic-tail-aware registry diff

`classify_entry_change(old, new) -> LayoutChange` is finer-grained than
`RegistryCompat`: it separates `FixedPrefixGrew` / `FixedPrefixShrank`,
`VersionBump`, `ShapeFlipped`, `IdentityChanged`, and — crucially —
`TailCapacity`. A dynamic-tail layout that keeps its fixed prefix and identity
but bumps version only moved its (off-chain) tail capacity/policy: that is
`Additive`, not `MigrationRequired`, because the loader only checks the fixed
prefix. `diff_descriptors_vs_registry_detailed` is the tail-aware companion to
`diff_descriptors_vs_registry`; feed either to `ManifestProfile::permits_upgrade`.

### CoW / account-data-cost layout lint

Direct mapping makes reads ≈ free, the first write a copy-on-write copy, and
growth (realloc) the most expensive operation. `AccountDescriptor::cost_profile()`
reports `cow_copy_bytes` (= `min_size`), a `SizeClass`
(Small/Medium/Large/VeryLarge by byte thresholds), and whether the layout is
`growable`. `cost_lint()` returns an advisory `CostLint`: `LargeFixedCopy` for
big fixed layouts on hot write paths, `ExpensiveGrowth` for large growable ones.
This is a descriptor-level model (no per-field metadata required) tied to the
runtime cost direction without asserting unsupported specifics.

### IDL / Codama export hook

`AccountDescriptor::idl_node() -> DescriptorIdlNode` is a minimal, stable,
`no_std` projection (name, disc, version, body offset/size, `LayoutKind`,
dynamic-tail/deprecated flags, `layout_id`, and the `LayoutFingerprint`) that an
IDL or Codama-style generator consumes to emit an account node *and* the
fail-closed decode guard — sourced from the same descriptor the loader enforces,
so the generated IDL can never describe a layout the program does not run.

### Off-chain metadata export (`hopper-schema`)

The IDL hook is wired into the off-chain emitter. `hopper_schema::DescriptorMetadata`
projects an `AccountDescriptor` (plus the layout's field wire map) into the exact
record a generated client embeds to **fail closed before decode**, and
`hopper_schema::codama::DescriptorMetadataJson` serializes it with the existing
hand-written (`no_std`, no-serde) JSON emitters:

```jsonc
{
  "name": "Vault",
  "disc": 1,
  "version": 1,
  "kind": "compact",
  "bodyOffset": 1,
  "bodySize": 40,
  "fixedSize": 40,
  "minSize": 41,
  "hasDynamicTail": false,
  "deprecated": false,
  "layoutId": "abababababababab",
  "fingerprint": "…32 ASCII hex chars…",
  "loadedDataSizeRecommendation": 297,
  "fields": [ { "name": "authority", "size": 32, "offset": 1, … } ]
}
```

A generated SDK embeds `fingerprint` (and/or `layoutId`) as a constant and calls
`hopper_schema::decode_allowed(expected, advertised)` — comparing its embedded
fingerprint to the one computed from the program's on-chain registry row — before
zero-copy-decoding. A mismatch means the program was redeployed with a different
layout at that discriminator, so the client refuses rather than reading
mis-shaped memory. `loadedDataSizeRecommendation` feeds
`setLoadedAccountsDataSizeLimit` directly. `DescriptorMetadataSetJson` emits a
whole program's accounts plus summed `minLoadedDataSize` /
`recommendedLoadedDataSize` budgets. Existing headered `SchemaExport` layouts get
`descriptor()` / `descriptor_metadata()` for free, derived from their manifest.

### Composite account groups (`DescriptorGroup`)

Anchor v2's `Nested<T>` shares validation across a bundle of accounts. Hopper's
descriptor-native answer is `hopper_core::manifest::DescriptorGroup`: a `const`,
*ordered* set of `AccountDescriptor`s validated as one unit, with no borrowed
Anchor syntax.

```rust
static SETTLE: &[AccountDescriptor] = &[
    <Vault as LayoutDescriptor>::DESCRIPTOR,
    <Order as LayoutDescriptor>::DESCRIPTOR,
];
let group = DescriptorGroup::new("Settle", SETTLE);

// Positional hot-path validation of the instruction's account data slices.
group.validate_all(&[vault_data, order_data])?;   // Err names the bad index

// Composite identity: folds each member fingerprint *in order* with a length
// prefix, so reordering or adding a member changes the group fingerprint. A
// client embeds it and fails closed if the program advertises a different
// composition.
let gfp = group.fingerprint();

// One loaded-data-size budget for the whole bundle.
let limit = group.recommend_loaded_data_limit(1024, 256);
```

`hopper_schema::codama::DescriptorGroupJson` serializes a group (name, composite
`groupFingerprint`, ordered `members`, and summed `minLoadedDataSize` /
`recommendedLoadedDataSize`) through the same hand-written `no_std` emitters.

### Typed account expectations (`AccountExpectation`)

`AccountDescriptor::validate` is the inlined hot-path len+disc check. Off the hot
path, a manager or generated client can afford more: `expect_owned_by(owner)`
projects the descriptor into an `AccountExpectation` (expected owner, disc,
`min_size`, fingerprint), and `check` / `check_decodable` return an
`AccountCheck` verdict:

```rust
let exp = <Vault as LayoutDescriptor>::DESCRIPTOR.expect_owned_by(program_id);
match exp.check_decodable(&account_owner, &data, advertised_fingerprint) {
    AccountCheck::Ok => { /* safe to zero-copy-decode */ }
    AccountCheck::WrongOwner
    | AccountCheck::TooSmall
    | AccountCheck::WrongDiscriminator
    | AccountCheck::FingerprintMismatch => { /* fail closed */ }
}
```

The owner check is the piece the hot path deliberately skips; the fingerprint
check is the same fail-closed guard `decode_allowed` performs, sourced from the
descriptor the loader enforces.

### CLI fail-closed decode (`manager accounts read`)

`hopper manager accounts read <pubkey>` now closes the loop against a live
account: it reads the headered account's embedded `layout_id` (bytes 8..16) and
compares it to the `layoutId` the manifest declares for that discriminator. On a
mismatch it prints a `FAIL-CLOSED` line and exits non-zero rather than reporting
a layout the account was not written under — so actual tooling, not just library
code, refuses a stale/mis-shaped account.

## Next concrete steps

- `DescriptorIdlNode` now feeds the `hopper-schema` emitter via
  `DescriptorMetadata` / `DescriptorMetadataJson`, and composite bundles via
  `DescriptorGroup` / `DescriptorGroupJson`, so the off-chain metadata and the
  on-chain registry derive from one descriptor pass. Next: have the per-language
  client generators (`rust_client`, `python_client`, …) consume that JSON to
  embed `fingerprint` and call `recommend_loaded_data_limit` in their
  transaction builders.
- Emit `DescriptorGroup`s from the macro for multi-account instructions so
  composite fingerprints are generated rather than hand-built.
- Extend `cost_lint` with per-field hot/cold classification once field-level
  role metadata is threaded through the descriptor.

See [`RESEARCH_COVERAGE.md`](RESEARCH_COVERAGE.md) for the full recommendation →
implementation audit.
