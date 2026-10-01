# Moving to Hopper 0.5

0.5 exists because two safe readers in hopper-native were unsound, and the
fix changes their signatures. hopper-native and hopper-runtime move to 0.5,
and so does every crate whose public signatures name their types: a 0.4
`AccountView` and a 0.5 `AccountView` are different types, so mixing the
lines would not compile. Upgrade the Hopper crates together. `hopper-derive`
and `hopper-macros` also move to 0.5: their expansions target the matching
runtime APIs, which a signature-only API inventory cannot fully describe.

```toml
hopper = { package = "hopper-lang", version = "0.5", features = ["proc-macros"] }
```

`python scripts/api-lock.py --against-published` prints, for every
published crate, the changes since the version on crates.io and the version
a subsequent release needs. The list below records the 0.4-to-0.5 changes,
including behavior changes a signature does not show.

This release preserves the wire formats, discriminators, layout fingerprints,
and account header for unchanged layout declarations. Programs using the same
layout can read the same bytes, subject to their owner, version, and application
authorization checks. Changing a layout can still require a migration.

## Signatures

### `AccountView::layout_id`

The eight bytes are returned by value. The old reference pointed into the
account's data with no borrow behind it, so safe code could hold it across
a `try_borrow_mut` of the same bytes. It also returns `None` while the data
is exclusively borrowed.

```rust
// 0.4
if account.layout_id() == Some(&Vault::LAYOUT_ID) { /* ... */ }
let id: [u8; 8] = *account.layout_id().unwrap();

// 0.5
if account.layout_id() == Some(Vault::LAYOUT_ID) { /* ... */ }
let id: [u8; 8] = account.layout_id().unwrap();
```

The typed loaders (`load`, `load_mut`, `load_cross_program`) validate the
header through their own borrow and are not affected.

### `DataFingerprint::capture`

Capture reads the data under a shared borrow, so it can fail.

```rust
// 0.4
let before = DataFingerprint::capture(vault, 64);

// 0.5
let before = DataFingerprint::capture(vault, 64)?;
```

It returns `AccountBorrowFailed` while the data is exclusively borrowed.
`verify_unchanged` already returned a `ProgramResult` and is unchanged.

### `MAX_HASH_SEGMENTS`

`hopper_native::hash::MAX_HASH_SEGMENTS` was 16 and documented as the
runtime's limit. The runtime's limit is 20,000, and the constant says so
now. Code that sized a stack array with it would allocate 320 KB of slice
references; size the array by the number of segments you hash.

### `MintProgram`

`token::MintProgram` and `token_mint::MintProgram` are aliases of
`token::TokenProgram`, the enum every builder takes. The variants
(`Legacy`, `Token2022`) and the methods are the same, so
`MintProgram::Token2022` and `program.address()` compile unchanged. Two
uses do not:

- `use MintProgram::*;` (a glob import through an alias). Name the enum:
  `use hopper::token::TokenProgram::*;`.
- A trait implemented for both `MintProgram` and `TokenProgram`: they are
  one type now, so keep one impl.

### `MintExtension`

`token_mint::MintExtension` gained seven variants (`DefaultAccountState`,
`InterestBearing`, `ScaledUiAmount`, `Pausable`, `GroupPointer`,
`GroupMemberPointer`, `PermissionedBurn`) and is `#[non_exhaustive]`, since
Token-2022 keeps adding extensions. Building a variant is unchanged. A
`match` on it outside Hopper needs a wildcard arm:

```rust
match extension {
    MintExtension::MintCloseAuthority(authority) => { /* ... */ }
    MintExtension::MetadataPointer { .. } => { /* ... */ }
    _ => {}
}
```

### `FieldRef::as_address`

The returned reference lives as long as the bytes the `FieldRef` was made
over, not as long as the `FieldRef`. Code that compiled before compiles
now; code that could not keep the address past the view now can.

## Behaviour

These do not change a signature. Each one fixes a defect; a program that
relied on the old behaviour was reading the wrong bytes or accepting data
the token program would refuse.

- `project_hopper` and `project_hopper_mut` project `T` at
  `HOPPER_HEADER_LEN` (16). They used offset 10, inside the header.
- The segment registry refuses an entry whose segment starts before the
  data region (`InvalidAccountData`). Such an entry exposed the header and
  the entry table as segment data.
- `mint_authority` and `mint_freeze_authority` refuse an option tag other
  than 0 or 1 (`InvalidAccountData`), as the token program does.
- `crypto::curve_validate_point` answers for Edwards points off chain. It
  returned `false` for every point before, so a host test that relied on
  that sees real answers now.
- The default allocator allocates forward from above Hopper's scratch
  region and grows the most recent block in place. The usable heap is the
  same size; a growing vector no longer leaves its earlier copies behind
  in it.

## New in 0.5

Nothing here needs a change to existing code:

- `default_allocator!(heap = N)`, `hopper::heap::{used, mark, release_to}`,
  and `hopper tx send --heap-frame <bytes>`.
- `sysvar::slot_hash` and `sysvar::slot_hash_lookup`.
- The `panic-location` and `panic-message` features.
- PDA derivation and verification on the host, and
  `find_program_address_const`.
- `pda::try_find_program_address` returns a `Result` for canonical search.
  Hook-list resolution uses it and rejects malformed TLV framing and reserved
  entry kinds. `HookError` has new `InvalidDiscriminator` and `InvalidSeeds`
  variants; update exhaustive matches.
- Token batches reject duplicate writable roles within an inner instruction.
  Reuse across instructions remains supported. Failed pushes preserve the
  existing batch, including custom encoders that fail after writing.
- `hopper_interface!`'s `pub struct View as Layout` form and `ORIGIN`.
