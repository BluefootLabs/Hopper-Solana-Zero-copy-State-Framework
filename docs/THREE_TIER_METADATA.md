# Three-Tier Metadata Model

How Hopper separates *hot-path account bytes* from *program-level
metadata* from *off-chain generated artifacts* so that the common case
pays for nothing it does not use.

> These three tiers are an implementation detail. The developer faces one
> system: see [`HOPPER_UNIFIED_ZERO_COPY.md`](HOPPER_UNIFIED_ZERO_COPY.md)
> for the `AccountDescriptor` / `LayoutDescriptor` one-source-of-truth model
> that derives the loader, the registry row, the field offsets, and the
> upgrade gate from a single layout declaration.

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

There is **no universal 16-byte header by default**. The authoring
surface is:

```rust
#[hopper::state(compact, disc = 1)]
pub struct Vault {
    pub authority: Pubkey,
    pub balance:   u64,
}
// → byte 0 = 1, bytes 1.. = { authority, balance }
```

Loading a compact account is `check_owner` + `check_len` + `check_disc`
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
> `hopper-schema` already defines `MANIFEST_SEED = b"hopper:manifest"`,
> the PDA that stores the program's **JSON** manifest (a Tier-3
> publication artifact). The binary zero-copy registry is a *distinct*
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

The richest tier is generated, never on the hot path: the JSON
`ProgramManifest`, the `ProgramIdl`, TypeScript / Kotlin / Rust / Go /
Python SDKs, the Hopper Manager schema, audit manifests, and docs. These
live in `hopper-schema` and the codegen modules. The on-chain registry's
`schema_hash` pins the off-chain artifacts to a specific schema so a
client can prove the IDL it holds matches the program it is calling.

See also [`docs/ONCHAIN_SCHEMA_PUBLICATION.md`](ONCHAIN_SCHEMA_PUBLICATION.md)
(the JSON manifest PDA + `HopperSchemaPointer`) and
[`docs/SCHEMA_ARCHITECTURE.md`](SCHEMA_ARCHITECTURE.md).

## Manifest profiles

A program declares how much of the registry machinery it wants:

| Profile     | Off-chain artifacts | On-chain registry PDA | Upgrade gating |
|-------------|---------------------|-----------------------|----------------|
| `offchain`  | yes (JSON / IDL)    | no                    | no             |
| `onchain`   | yes                 | published & read      | no             |
| `governed`  | yes                 | published & read      | upgrades/migrations must match the on-chain registry |

The authoring surface is
`#[hopper::program(manifest = "offchain" | "onchain" | "governed")]`.
The macro emits a `HOPPER_PROGRAM_MANIFEST_PROFILE: ManifestProfile`
const for build tooling to read; unknown profile strings fail closed at
macro-expansion time. The profile semantics (`ManifestProfile`) live in
`hopper_core::manifest`.

## What ships in this change

This change lands the model end to end -- runtime, data model, **macro
ergonomics**, and validation helpers -- fully tested and
`no_std`/zero-copy clean:

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
6. A macro-based example (`examples/hopper-compact-vault`) and the
   compile-pass/compile-fail trybuild coverage.

## Macro ergonomics

### `#[hopper::state(compact, disc = N)]`

Declares a Tier-1 compact account. The macro emits the `CompactLayout`
impl (`DISC = N`, body = the struct), the `Pod`/`Zeroable` proofs, the
`[disc:u8][body]` load helpers (`load_compact`, `load_compact_mut`,
`init_compact`, `overlay_body`), the `registry_entry()` Tier-2 row
builder, and per-field offset consts. It deliberately does **not** emit
`LayoutContract`/`HopperLayout`: a compact account has no 16-byte header,
so `account.load::<T>()` must not be made available for it. Offsets are
body-relative inside the struct but absolute on the wire -- the emitted
`{FIELD}_ABS_OFFSET` consts fold in the single discriminator byte
(`COMPACT_BODY_OFFSET = 1`), not `HEADER_LEN`:

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

`compact` is fixed-size by construction: combining it with
`dynamic_tail = T` or `raw_tail = true` is rejected at macro-expansion
time.

### `#[hopper::program(manifest = "...")]`

Parses the profile string into a `ManifestProfile` and emits
`pub const HOPPER_PROGRAM_MANIFEST_PROFILE: ManifestProfile`. Unknown
strings fail closed during expansion (only `offchain`, `onchain`,
`governed` are accepted). Build/publish tooling reads the const to decide
whether to publish the registry PDA and whether to gate upgrades.

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
