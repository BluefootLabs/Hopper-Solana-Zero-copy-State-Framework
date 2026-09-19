# Hopper: One Unified Zero-Copy System

Hopper's three internal tiers (hot-path account bytes, an optional on-chain
registry, and off-chain artifacts; see
[`THREE_TIER_METADATA.md`](THREE_TIER_METADATA.md)) start from one macro
declaration. The macro generates loader traits/helpers, field offsets,
`LayoutManifest`, and a parallel `LayoutDescriptor`; the registry row delegates
to that descriptor. Existing typed loaders and the primary client generators do
not consume the descriptor directly. The application owns registry provisioning
and must explicitly invoke any upgrade gate.

This note describes the unification, the `AccountDescriptor` /
`LayoutDescriptor` one-source-of-truth model, and grounds it against the
2026 Solana zero-copy landscape (Anchor v2, Pinocchio, direct mapping,
SIMD-0219/0268/0339).

## The single developer-facing model

A program author writes two macros and nothing else:

```rust
// One layout declaration. Compact (1-byte disc) hot path selected here.
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Vault {
    #[role = "authority"] pub authority: Address,
    #[role = "balance"]   pub balance: WireU64,
}

// One program-level profile. `governed` selects the strictest compatibility policy;
// application upgrade code must still evaluate and enforce it.
#[hopper::program(manifest = "governed")]
mod vault_program { /* ... */ }
```

From that declaration, the macro emits the zero-copy load helpers, per-field
absolute offsets (folding in the single discriminator byte), `LayoutManifest`,
and a `LayoutDescriptor` whose `registry_entry()` supplies the Tier-2 row.
Those are parallel generated surfaces, not a runtime chain in which the loader
reads the descriptor. Headered layouts
(`#[hopper::state(disc = N, version = V)]`) emit the *same*
`LayoutDescriptor` surface; the only difference is the body offset
(`HEADER_LEN` vs `COMPACT_BODY_OFFSET`). The developer chooses compact vs
headered per struct and otherwise writes identical code.

## One source of truth: `AccountDescriptor`

`hopper_core::manifest::AccountDescriptor` is a pure `const` value for registry
and optional tooling projections:

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

- `AccountDescriptor::compact(name, disc, version, body_size, layout_id)`,
  `body_offset = COMPACT_BODY_OFFSET = 1`, `flags = ENTRY_FLAG_COMPACT`,
  `min_size = 1 + body_size`.
- `AccountDescriptor::headered(name, disc, version, body_size, layout_id)`,
  `body_offset = HEADER_LEN = 16`, `flags = ENTRY_FLAG_HEADERED`,
  `min_size = 16 + body_size`.

`.with_dynamic_tail()` and `.deprecated()` are `const` builders that flip the
corresponding entry flag for variable-length and retired layouts.

The descriptor directly supplies these optional surfaces:

| Consumer            | Derivation                                        |
|---------------------|---------------------------------------------------|
| Tier-2 registry row | `descriptor.registry_entry() -> AccountLayoutEntry` |
| Shape helper        | `descriptor.validate(data)` (minimum len + disc only) |
| Tooling projection  | `descriptor.idl_node()` / `descriptor.fingerprint()` |
| Upgrade-gate input  | `diff_descriptors_vs_registry(&descriptors, &onchain)` |

Typed loader impls and field offsets are parallel macro outputs from the same
source declaration, not descriptor consumers. `validate_hot` is not a substitute
for the full fixed-compact or headered loader. Once a registry row is persisted,
callers must authenticate its account, verify its hashes, and compare it with
the current descriptors.

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
purely additive, the existing `CompactLayout`, `LayoutContract`, and
`SchemaExport` surfaces are untouched.

## Hot path: no manifest reads and an unchanged compact validation path

`validate_hot` (and the `AccountDescriptor::validate` it delegates to) is
`#[inline(always)]` and does exactly two checks:

1. `data.len() < min_size` → `AccountDataTooSmall`
2. `data[0] != disc` → `InvalidAccountData`

No registry fetch, no layout_id comparison, no epoch read. The descriptor is a
`const`, so the discriminator and minimum size are immediates at the call site.
The compact hot path is unchanged from the pre-unification cost: `check_owner`
+ `check_len` + `check_disc` + cast-at-offset-1.

## Off the hot path: governed upgrade compatibility

A `governed` application can classify an upgrade by comparing its *generated*
descriptors against an authenticated *on-chain* registry without leaving the
no-alloc model:

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
`onchain` / `offchain` admit everything short of `Breaking`. These are
compatibility primitives, not a hook into Solana's loader-v3 upgrade path.
Hopper does not provision the registry or intercept a native loader upgrade;
the application's governed upgrade instruction or release workflow must call
the comparison and abort on a rejected result.

## Why this matters now (2026 landscape)

The unification is grounded in where Solana zero-copy is actually heading
(full analysis in
[`ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md`](ZERO_COPY_FRAMEWORK_AUDIT_2026-08-15.md)):

- **Anchor v2 is Pinocchio-backed and zero-copy-by-default.** The archived
  pre-RC comparison does not support a categorical "Anchor is slow / heavy"
  claim. Rebenchmark the current v2 line. Hopper's durable differentiation is
  descriptor coherence: one declaration supplies loader
  checks, an optional registry row, client metadata, and upgrade-compatibility
  inputs. Runtime registry authentication and upgrade enforcement remain
  explicit application work.

- **Direct account mapping is active on testnet/devnet and pending Mainnet.**
  As reverified 2026-09-06, read-only account data on those clusters can be
  mapped without the up-front host data copy; a first write may copy the
  account's full current data, and growth can add realloc work. This rewards
  compact hot layouts, but a dynamic account's current length; not descriptor
  `min_size`: controls its copy class. Loaded-data limits remain an explicit
  transaction-builder decision.

- **SIMD-0219 / 0268 / 0339** continue to tighten the cost model around loaded
  data size and account access. A single descriptor that knows each layout's
  exact `min_size` and shape flags is the right place to compute those limits
  once and feed both the on-chain validator and the off-chain client.

- **Fingerprint-aware client decode.** A provisioned and verified registry's
  `schema_hash` / `registry_hash` can pin off-chain artifacts. Current generated
  clients are transport-neutral: headered readers compare the stored
  `layout_id`; compact readers check the discriminator plus exact size for a
  fixed layout or minimum prefix size for a compact-dynamic layout. Both expose
  descriptor-derived identity metadata for a caller that has a trusted registry
  or release artifact.

## What this change lands

Additive and covered by focused tests, with `no_std` / zero-copy paths kept clean:

1. `AccountDescriptor` + `LayoutDescriptor` in `hopper_core::manifest`, the
   one-source-of-truth type and trait, with `const` constructors for compact
   and headered shapes and `#[inline(always)]` len+disc validation.
2. `diff_descriptors_vs_registry`: no-alloc generated-vs-on-chain comparison
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

Anchor v2 is Pinocchio-backed and zero-copy-by-default, so the archived pre-RC
comparison cannot establish a current speed/DX gap. The ideas worth taking are
about *coherence*, typed account validation,
fail-closed client decode, and tooling that cannot describe a different layout
than the program runs. Hopper adapts each in its own descriptor-native way
(not by copying Anchor syntax or internals). Every API below derives from the
one `AccountDescriptor` and is `const` / `no_std` / no-alloc.

### Client decode fingerprint (`LayoutFingerprint`)

`AccountDescriptor::fingerprint() -> LayoutFingerprint` is a deterministic
16-byte identity over the *wire-identity* fields (name, disc, version, sizes,
body offset, shape flags, `layout_id`), never the `deprecated` lifecycle bit,
so deprecating a layout never changes how it decodes. Generated SDKs expose
that identity as external metadata. Their byte guards are shape-specific:
headered accounts compare the stored eight-byte `LAYOUT_ID`; compact accounts,
which store no fingerprint, require the discriminator plus either fixed exact
size or a dynamic-tail minimum prefix.

```rust
let fp = <Vault as LayoutDescriptor>::fingerprint();   // const, embeddable
// A transport-aware caller may compare `fp` with a trusted registry or release
// artifact through `decode_allowed` before invoking the generated byte decoder.
// Neither the generated SDK nor `decode_allowed` fetches or authenticates that
// external metadata.
```

This is the per-type complement to the registry's `schema_hash` / `registry_hash`
(which pin the whole artifact set): the fingerprint pins one account type.

### Loaded-accounts data-size budgeting

`min_loaded_data_size(&descriptors)` sums each layout's `min_size`;
`recommend_loaded_data_limit(&descriptors, tail_headroom, extra)` adds headroom
per dynamic-tail layout plus a flat margin. These are advisory building blocks:
current transaction builders do not emit `setLoadedAccountsDataSizeLimit`
automatically. A caller must inventory the accounts an instruction can load,
add application-specific tail headroom and margin, and clamp the saturating
result to the runtime maximum. Descriptor `min_size` should match the loader's
minimum, but this helper does not establish or enforce that equality.

### Conservative registry diff

`classify_entry_change(old, new) -> LayoutChange` is finer-grained than
`RegistryCompat`: it separates `FixedPrefixGrew` / `FixedPrefixShrank`,
`VersionBump`, `ShapeFlipped`, and `IdentityChanged`. A version bump remains
`MigrationRequired` even when the row is dynamic-tail: the registry commits no
tail capacity, policy, or reason for the bump, so classifying it as additive
would infer facts that are absent from the artifact.
`diff_descriptors_vs_registry_detailed` is a diagnostic companion to
`diff_descriptors_vs_registry`; an application that provisions a registry must
authenticate it and explicitly enforce the resulting policy decision.

### CoW / account-data-cost layout lint

This is forward-looking/test-cluster advice while direct mapping remains
pending Mainnet as of 2026-09-06. `AccountDescriptor::cost_profile(current_len)`
uses the observed full account-data length, clamped to `min_size`, and reports
that potential first-write `cow_copy_bytes`, a `SizeClass`
(Small/Medium/Large/VeryLarge), and whether the layout is `growable`.
`cost_lint(current_len)` returns `LargeFixedCopy` for a large fixed account and
`ExpensiveGrowth` for a currently large growable account. Supplying the live
length is essential: a small dynamic prefix can back a very large account.
Neither helper is a Mainnet CU quote.

### IDL / Codama projection building block

`AccountDescriptor::idl_node() -> DescriptorIdlNode` is a minimal, stable,
`no_std` projection (name, disc, version, body offset/size, `LayoutKind`,
dynamic-tail/deprecated flags, `layout_id`, and the `LayoutFingerprint`) available
to an IDL or Codama-style generator. The current primary CLI, IDL, and SDK
generators consume `LayoutManifest` instead; `idl_node()` is not yet their input.

### Optional off-chain metadata export (`hopper-schema`)

`hopper_schema::DescriptorMetadata` projects an `AccountDescriptor` plus a
separately supplied field wire map into an optional tooling record, and
`hopper_schema::codama::DescriptorMetadataJson` can serialize it with a
hand-written (`no_std`, no-serde) JSON emitter. This building block is tested but
is not wired into the main CLI or client-generation pipeline:

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

The primary generated SDKs embed `LayoutManifest.layout_id` as identity metadata;
they do not consume this 16-byte descriptor fingerprint record. A caller with
trusted advertised metadata can call
`hopper_schema::decode_allowed(expected, advertised)` in its own integration;
Hopper's transport-neutral generators do not fetch or authenticate a registry. The
`loadedDataSizeRecommendation` value is an input for
`setLoadedAccountsDataSizeLimit`, not an automatically emitted compute-budget
instruction. `DescriptorMetadataSetJson` emits a whole program's accounts plus
summed `minLoadedDataSize` / `recommendedLoadedDataSize` budgets. Existing
headered `SchemaExport` layouts get `descriptor()` / `descriptor_metadata()`
derived from their manifest.

## Next concrete steps

- `DescriptorIdlNode` can feed the standalone `DescriptorMetadata` /
  `DescriptorMetadataJson` emitter. Next: connect it deliberately to the primary
  manifest/IDL pipeline, with coherence tests against the parallel
  `LayoutManifest` path.
- All six generated SDK targets carry manifest layout identity and fail-closed
  byte guards. Next: add an optional authenticated metadata resolver and let
  transaction builders consume `recommend_loaded_data_limit` explicitly.
- Extend `cost_lint` with per-field hot/cold classification once field-level
  role metadata is threaded through the descriptor.
