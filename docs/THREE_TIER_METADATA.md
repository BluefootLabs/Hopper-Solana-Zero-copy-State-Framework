# Three-Tier Metadata Model

How Hopper separates *hot-path account bytes* from *program-level
metadata* from *off-chain generated artifacts* so that the common case
pays for nothing it does not use.

> These three tiers are an implementation detail. The developer faces one
> system: see [`HOPPER_UNIFIED_ZERO_COPY.md`](HOPPER_UNIFIED_ZERO_COPY.md)
> for the `AccountDescriptor` / `LayoutDescriptor` one-source-of-truth model
> that derives the loader, the registry row, the field offsets, and the
> upgrade gate from a single layout declaration.

> **Implementation boundary, reverified 2026-09-21.** Tier-1 compact loading,
> the Tier-2 binary data model/parser/diff helpers, manifest-profile constants,
> and Tier-3 local generators ship. Hopper does **not** ship a generic
> transaction that creates/publishes the binary registry PDA, or runtime
> enforcement that consults a deployed `onchain`/`governed` registry. Those
> profiles currently describe generated intent for tooling. `hopper
> publish-idl` publishes only a losslessly representable Solana IDL v0.1
> projection through Program Metadata and fails closed otherwise; the current
> Cicada surface is refused because its u16-prefixed bounded route data and
> remaining-account contract are not faithfully expressible in that schema.
> `hopper publish-manifest` publishes the full JSON manifest under the custom
> Program Metadata seed `hopper-manifest`, and `hopper publish-security` the
> `security.txt` record; both are declarations on the ledger, not the binary
> registry and not runtime enforcement.

## Motivation

Hopper's default account layout carries a 16-byte universal header
(`HopperHeader`: disc, version, flags, layout_id, schema_epoch). That
header is excellent for self-describing accounts and schema evolution,
but it is *per account*: every PDA pays 16 bytes and every typed load
re-validates the full header (disc + version + layout_id + epoch).

For the hottest accounts in a protocol -- the ones touched on every
instruction -- most of that self-description is redundant. The program
*already knows* what layout sits behind discriminator `1`; it does not
need to re-read a layout_id fingerprint on every load. The identity of a
schema is a *program-level* fact, not a *per-account* fact.

The three-tier model makes that separation explicit:

```text
Tier 1  hot-path account bytes      [disc:u8][zero-copy body]
Tier 2  on-chain program registry   one PDA per program (optional)
Tier 3  off-chain generated         IDL / SDKs / manager schema / docs
```

## Tier 1 -- Compact accounts (hot path)

A compact account stores exactly one discriminator byte followed by the
zero-copy body:

```text
byte 0   : disc (u8)
bytes 1..: zero-copy body (alignment-1 Pod fields)
```

A compact account has **no 16-byte header**; the repository-wide default
remains the headered path unless `compact` is selected. The compact authoring
surface is:

```rust
#[hopper::state(compact, disc = 1)]
pub struct Vault {
    pub authority: Pubkey,
  pub balance:   WireU64,
}
// → byte 0 = 1, bytes 1.. = { authority, balance }
```

Loading a compact fixed account is `check_owner` + `check_len_exact` + `check_disc`
+ cast-body-at-offset-1. No layout_id read, no epoch comparison, no
manifest fetch on the hot path. The runtime support for this lives in
[`hopper_runtime::compact`](../crates/hopper-runtime/src/compact.rs):
the `CompactLayout` trait plus `AccountView::load_compact`,
`load_compact_mut`, and `init_compact`.

Back-compatibility: the existing 16-byte-header path is unchanged and
remains the default for `#[hopper::state]`. Compact is opt-in per
struct. The two coexist because they are distinguished by the layout
type the caller projects, not by a global mode switch.

### When *not* to go compact

Compact trades self-description for bytes and CU. Use the full header
when an account must be decoded by foreign programs that do not have the
program's registry, when schema epoching/migration gates are needed
per-account, or when you want `layout_info()` introspection without a
registry lookup. A protocol can mix both: compact hot accounts, headered
config/governance accounts.

### Optional body-level self-verification

Compact does not mean *zero* metadata. Critical accounts can carry their
own verification fields **as ordinary body fields** -- e.g. a
`version: u8`, a `layout_id: WireU64`, or a `runtime_flags: WireU32` at a
fixed body offset -- and the handler checks them explicitly. This is a
deliberate, per-account choice paid for only where it earns its keep,
rather than a universal header tax.

## Tier 2 -- On-chain program registry (optional)

A single optional PDA per program describes *all* of the program's
account layouts in a compact, zero-copy, `no_std`-readable binary form.
This is the on-chain answer to "what does discriminator `N` mean for
this program?" without parsing JSON on-chain.

Seed (see [`hopper_core::manifest`](../crates/hopper-core/src/manifest.rs)):

```text
find_program_address(&[REGISTRY_SEED, program_id], program_id)
REGISTRY_SEED = b"hopper:registry"
```

> **Why `hopper:registry` and not `hopper:manifest`?**
> `hopper-schema` already defines `MANIFEST_SEED = b"hopper:manifest"` for a
> legacy JSON-manifest account shape consumed by fetch/Manager code. No generic
> publisher for that PDA ships. The binary zero-copy registry is a *distinct*
> account with a distinct hot-path purpose, so it gets its own seed to
> avoid clobbering the JSON manifest PDA. The two are siblings: the
> registry is the on-chain, hot-path-readable form; the JSON manifest is
> the rich publication form.

Binary layout (all multi-byte fields are alignment-1 wire integers, so
the whole structure is `Pod` and overlays directly on account bytes):

```text
ProgramManifestHeader (80 bytes)
  magic         : [u8; 8]    "HOPRREG1"
  version       : WireU16
  account_count : WireU16
  flags         : WireU32
  schema_hash   : [u8; 32]   deterministic hash of the whole schema
  registry_hash : [u8; 32]   deterministic hash of the entry table

AccountLayoutEntry (31 bytes each, account_count of them follow)
  disc       : u8
  version    : WireU16
  min_size   : WireU32
  fixed_size : WireU32
  layout_id  : WireU64
  flags      : WireU32
  name_hash  : [u8; 8]
```

The registry stores discriminator → (version, sizes, layout_id, flags,
name hash) for every account type, plus two deterministic hashes:
`registry_hash` over the entry table (tamper / drift detection) and
`schema_hash` over the broader schema (links the on-chain registry to
the off-chain manifest/IDL). `ProgramManifestView` provides
bounds-checked iteration, `find_by_disc`, and hash verification.

## Tier 3 -- Off-chain generated metadata

The richest tier is generated, never on the hot path: the Hopper manifest;
Hopper public IDL; Codama JSON; conditional, fail-closed Solana IDL v0.1;
TypeScript, Kotlin, Python, Go, C, and off-chain Rust SDKs; Manager inputs;
audit artifacts; and docs. These live in `hopper-schema` and the codegen
modules. The binary registry data model includes a `schema_hash` that a future
publisher/consumer can use to pin off-chain artifacts; Hopper does not yet ship
that registry lifecycle or consult a deployed registry from generated clients.

Generated clients branch on account encoding. Headered layouts read the
8-byte layout fingerprint from the Hopper header at bytes `4..12` before
decoding. Compact layouts have no such header: all six generated SDK targets
check the discriminator and use the manifest's explicit tail metadata to
require exact size for a fixed layout or minimum prefix size for a dynamic-tail
layout. They then expose the fingerprint from manifest/IDL metadata. The field offsets in
compact generated clients are absolute `[disc][body]` offsets, so a
41-byte compact vault decodes `authority` from `1..33` and `balance`
from `33..41`.

See also [`docs/ONCHAIN_SCHEMA_PUBLICATION.md`](ONCHAIN_SCHEMA_PUBLICATION.md)
(shipped Program Metadata IDL publication versus proposed Hopper pointer/effect
publication) and
[`docs/SCHEMA_ARCHITECTURE.md`](SCHEMA_ARCHITECTURE.md).

## Manifest profiles

A program can declare intended registry semantics. The macro emits the profile
constant, but no shipped publisher or runtime currently acts on it:

| Profile     | Shipped local artifacts | Intended registry semantics | Intended upgrade semantics |
|-------------|-------------------------|-----------------------------|----------------------------|
| `offchain`  | yes                     | none                        | none                       |
| `onchain`   | yes                     | publish and read a registry | no registry gate           |
| `governed`  | yes                     | publish and read a registry | require compatible registry changes |

The authoring surface is
`#[hopper::program(manifest = "offchain" | "onchain" | "governed")]`.
The macro emits a `HOPPER_PROGRAM_MANIFEST_PROFILE: ManifestProfile`
const for build tooling to read; unknown profile strings fail closed at
macro-expansion time. The profile semantics (`ManifestProfile`) live in
`hopper_core::manifest`.

## What ships in this change

This change lands the local runtime/data model, **macro ergonomics**, and
validation helpers, fully tested and `no_std`/zero-copy clean. It does not land
the generic on-chain registry account lifecycle or publisher:

1. This design note.
2. Tier 1 runtime support: `hopper_runtime::compact` (`CompactLayout`,
   `AccountView::load_compact` / `load_compact_mut` / `init_compact`).
3. Tier 2 data model: `hopper_core::manifest` (`ProgramManifestHeader`,
   `AccountLayoutEntry`, `ProgramManifestView`, `ManifestProfile`,
   `REGISTRY_SEED`, deterministic FNV-1a-64 hashing, builder helper).
4. Macro ergonomics (below): `#[hopper::state(compact, disc = N)]` and
   `#[hopper::program(manifest = "...")]`.
5. Validation/diff primitives (below): `diff_entry`, `diff_registries`,
   `registry_matches`, `ManifestProfile::try_parse` /
   `permits_upgrade`, `RegistryCompat`.
6. A macro-based, devnet-ready example (`examples/hopper-compact-vault`),
   six-language generated compact-client coverage, and compile-time coverage.

## Macro ergonomics

### `#[hopper::state(compact, disc = N)]`

Declares a Tier-1 compact account. The macro emits the `CompactLayout`
impl (`DISC = N`, body = the struct), the `Pod`/`Zeroable` proofs, the
`[disc:u8][body]` load helpers (`load_compact`, `load_compact_mut`,
`init_compact`, `overlay_body`), the `registry_entry()` Tier-2 row
builder, field roles/invariants, and per-field offset consts. It
deliberately does **not** emit `LayoutContract`/`HopperLayout` or
`SchemaExport`: those surfaces assume the 16-byte headered account path,
so `account.load::<T>()` must not be made available for compact layouts.
Offsets are body-relative inside the struct but absolute on the wire --
the emitted `{FIELD}_ABS_OFFSET` consts fold in the single discriminator
byte (`COMPACT_BODY_OFFSET = 1`), not `HEADER_LEN`:

```rust
#[derive(Clone, Copy, Debug, Default)]
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Vault {
    #[role = "authority"] pub authority: Address,
    #[role = "balance"]   pub balance: WireU64,
}

// Vault::BODY_SIZE == 40, COMPACT_LEN == MIN_SIZE == 41, DISC == 1
// Vault::AUTHORITY_ABS_OFFSET == 1, Vault::BALANCE_ABS_OFFSET == 33
let entry = Vault::registry_entry(); // ENTRY_FLAG_COMPACT row for Tier 2
```

`compact` supports both fixed and growable forms. A fixed declaration implements
`CompactLayout` and requires exact `COMPACT_LEN`. Adding `dynamic_tail = T`,
`raw_tail = true`, or the program-managed `dynamic` option implements
`CompactDynamicLayout`: `COMPACT_LEN` is the minimum `[disc][fixed head]`
prefix and tail bytes may follow it. The generated manifest records that size
policy so off-chain readers do not reject a valid grown account.

### `#[hopper::program(manifest = "...")]`

Parses the profile string into a `ManifestProfile` and emits
`pub const HOPPER_PROGRAM_MANIFEST_PROFILE: ManifestProfile`. Unknown
strings fail closed during expansion (only `offchain`, `onchain`,
`governed` are accepted). The constant records intent for future build/publish
and upgrade-gate tooling; current tooling does not publish the registry PDA or
install a runtime upgrade gate from it.

## Validation and diff primitives

`hopper_core::manifest` provides pure, no-alloc helpers over the
zero-copy `ProgramManifestView` so a governed program can compare its
*generated* registry against the *on-chain* one without leaving the hot
path's allocation-free model:

- `registry_matches(view, &schema_hash, &registry_hash)` -- confirms an
  on-chain view pins the expected schema and verifies its own registry
  hash.
- `diff_entry(old, new) -> RegistryCompat` and
  `diff_registries(old, new) -> RegistryCompat` -- classify a layout
  change as `Unchanged`, `Additive`, `MigrationRequired`, or `Breaking`
  (a layout-id change, shape-flag flip, removed disc, or shrunk size is
  `Breaking`; a grown size or version bump is `MigrationRequired`; a new
  disc is `Additive`). `RegistryCompat` is severity-ordered, so
  combining rows is a `max`.
- `ManifestProfile::try_parse(s)` -- fail-closed string parse mirroring
  the macro.
- `ManifestProfile::permits_upgrade(compat)` -- `governed` admits only
  `Unchanged`/`Additive`; `offchain`/`onchain` admit everything short of
  `Breaking`.

Hot-path compact account loading stays manifest-free: these helpers are
for upgrade/migration instructions and off-chain tooling, never the
per-account read.
