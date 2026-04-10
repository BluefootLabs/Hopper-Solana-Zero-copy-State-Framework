# Bad Evolution: What Not To Do

This document catalogs the most common layout evolution mistakes and shows
exactly how Hopper catches each one. Every anti-pattern here has been
encountered in production Solana programs.

## Anti-Pattern 1: Removing a Field

```rust
// V1 -- original
hopper_layout! {
    pub struct Token, disc = 1, version = 1 {
        mint:      TypedAddress<Mint>      = 32,
        authority: TypedAddress<Authority>  = 32,
        balance:   WireU64                 = 8,
        bump:      u8                      = 1,
    }
}

// V2 -- WRONG: removed authority
hopper_layout! {
    pub struct TokenV2, disc = 1, version = 2 {
        mint:    TypedAddress<Mint> = 32,
        balance: WireU64           = 8,
        bump:    u8                = 1,
    }
}
```

**What happens**: The `LAYOUT_ID` changes (it is a SHA-256 hash of all field
names, types, and sizes). Existing V1 accounts will fail `load()` because the
stored layout_id no longer matches. The schema compatibility checker returns
`Incompatible` -- not `AppendSafe`, not `MigrationRequired`, but fully
incompatible.

**Why it is dangerous**: V1 accounts have 73 bytes of data laid out as
`[header:16][mint:32][authority:32][balance:8][bump:1]`. If you overlay V2
on those bytes, `balance` now reads from the authority field (bytes 48-55),
returning garbage. Authority checks vanish silently. Funds drain.

**What Hopper does**:

- `hopper_assert_compatible!(Token, TokenV2, append)` fails at compile time
  (V2 is smaller, not a superset)
- `CompatibilityVerdict::between()` returns `Incompatible`
- `hopper compat <v1-hex> <v2-hex>` prints `INCOMPATIBLE: layout_id mismatch,
  field removal detected`
- The CLI migration planner refuses to generate a plan

**The fix**: Never remove fields. If a field is no longer needed, keep it in the
layout and stop writing to it. Mark it with `FieldIntent::Deprecated` in the
schema manifest.

## Anti-Pattern 2: Reordering Fields

```rust
// V1
hopper_layout! {
    pub struct Pool, disc = 2, version = 1 {
        authority: TypedAddress<Authority> = 32,
        mint_a:    TypedAddress<Mint>      = 32,
        mint_b:    TypedAddress<Mint>      = 32,
        balance_a: WireU64                 = 8,
        balance_b: WireU64                 = 8,
    }
}

// V2 -- WRONG: swapped mint_a and mint_b
hopper_layout! {
    pub struct PoolV2, disc = 2, version = 2 {
        authority: TypedAddress<Authority> = 32,
        mint_b:    TypedAddress<Mint>      = 32,
        mint_a:    TypedAddress<Mint>      = 32,
        balance_a: WireU64                 = 8,
        balance_b: WireU64                 = 8,
    }
}
```

**What happens**: The LAYOUT_ID changes because field names at each offset are
different. Field names are part of the hash. `mint_a` at offset 16 vs `mint_b`
at offset 16 produces a completely different fingerprint.

**Why it is dangerous**: All existing accounts have mint_a in bytes 16-47 and
mint_b in 48-79. V2 reads them backwards. Swaps execute against the wrong
mints. Every trade is a loss.

**What Hopper does**: Same as removing a field -- `Incompatible` verdict, compile
time assertion failure, CLI refusal.

**The fix**: Never reorder fields. Append new fields at the end. If you need a
different logical ordering for readability, add a comment -- the physical layout
must stay stable.

## Anti-Pattern 3: Changing a Field's Size

```rust
// V1
hopper_layout! {
    pub struct Config, disc = 3, version = 1 {
        authority: TypedAddress<Authority> = 32,
        fee_bps:   WireU16                = 2,
    }
}

// V2 -- WRONG: widened fee_bps from 2 bytes to 8 bytes
hopper_layout! {
    pub struct ConfigV2, disc = 3, version = 2 {
        authority: TypedAddress<Authority> = 32,
        fee_bps:   WireU64                = 8,
    }
}
```

**What happens**: V2 overlay reads 8 bytes starting at offset 48. On a V1
account, only 2 of those bytes contain the fee. The other 6 are either zero
(if the account was zero-initialized) or garbage (if the account was realloc'd
with stale data). The fee value is corrupted.

**What Hopper does**:

- LAYOUT_ID changes (field type and size are part of the hash)
- Schema diff shows `fee_bps: size 2 -> 8 (TYPE CHANGE)`
- Verdict: `Incompatible`

**The fix**: Append the wider field as a new name:

```rust
hopper_layout! {
    pub struct ConfigV2, disc = 3, version = 2 {
        authority:    TypedAddress<Authority> = 32,
        fee_bps:      WireU16                = 2,   // keep old field
        fee_bps_wide: WireU64                = 8,   // new field appended
    }
}
```

During migration, read the old `fee_bps`, widen it, write to `fee_bps_wide`,
and update your instruction handlers to use the new field. The old field stays
in place for backward compatibility.

## Anti-Pattern 4: Changing the Discriminator

```rust
// V1
hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
    }
}

// V2 -- WRONG: changed disc from 1 to 5
hopper_layout! {
    pub struct VaultV2, disc = 5, version = 2 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}
```

**What happens**: Every existing Vault account has disc=1 in byte 0. Loading
with `VaultV2::load()` checks for disc=5 and rejects every existing account
with a discriminator mismatch error.

**What Hopper does**:

- `hopper_assert_compatible!` fails (disc values differ)
- `CompatibilityVerdict::between()` returns `Incompatible` with reason
  "discriminator mismatch"

**The fix**: The disc is the account type identifier. It never changes between
versions of the same account type. Increment `version`, not `disc`.

## Anti-Pattern 5: Skipping the Header

```rust
// Manually overlaying without the 16-byte header
fn bad_load(data: &[u8]) -> &MyStruct {
    // WRONG: cast starts at byte 0, but bytes 0-15 are the header
    unsafe { &*(data.as_ptr() as *const MyStruct) }
}
```

**What happens**: The struct's first field reads from the header bytes. The
authority field contains the disc, version, flags, and layout_id -- not a
public key. Every check passes against garbage.

**What Hopper does**: All generated `overlay()` and `load()` methods
automatically offset past the 16-byte header. `overlay(data)` returns a
reference to `&data[HEADER_LEN..]` cast to `&T`. There is no way to
accidentally overlay at byte 0 through the standard API.

**The fix**: Always use `MyLayout::load()` or `MyLayout::overlay()`. If you
need raw access, use Tier C (`load_unchecked`) which still accounts for the
header offset.

## Anti-Pattern 6: Not Bumping the Version

```rust
// V1
hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
    }
}

// V2 -- WRONG: same version number
hopper_layout! {
    pub struct VaultV2, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}
```

**What happens**: Both layouts claim version 1 but have different LAYOUT_IDs
(because the field set differs). `load_compatible(account, program_id, 1)`
accepts both, but the layout fingerprint check fails for the wrong one. The
failure mode depends on which version you load:

- Loading V2 on a V1 account: reads 1 byte past the end of stored data
- Loading V1 on a V2 account: silently ignores the bump byte (less dangerous,
  but still wrong)

**What Hopper does**:

- `hopper_assert_compatible!` catches the version collision at compile time
- Schema diffing reports "same version, different layout_id"
- The lint engine flags this as a structural inconsistency

**The fix**: Always increment the version number when you change the field set.
The version byte is cheap (1 byte in the header) and makes dual-version loading
safe.

## Anti-Pattern 7: Initializing Without `hopper_init!`

```rust
fn bad_init(payer: &AccountView, account: &AccountView, system: &AccountView, pid: &Address) -> ProgramResult {
    // Create the account
    pinocchio_system::instructions::CreateAccount { ... }.invoke()?;

    // WRONG: write data without writing the header first
    let data = unsafe { account.borrow_unchecked_mut() };
    let vault = unsafe { &mut *(data.as_mut_ptr().add(16) as *mut Vault) };
    vault.authority = TypedAddress::from_account(payer);

    Ok(())
}
```

**What happens**: The 16-byte header is all zeros. The disc is 0 (no known
type), the version is 0, the layout_id is `[0; 8]`. Any subsequent `load()`
call rejects the account because the disc and layout_id do not match.

Worse: if another layout happens to use disc=0, the account gets loaded as the
wrong type.

**What Hopper does**: `hopper_init!` always calls `zero_init()` followed by
`write_init_header()` which stamps the correct disc, version, flags, and
layout_id. All subsequent `load()` calls validate these fields.

**The fix**: Always use `hopper_init!` or manually call
`YourLayout::write_init_header(data)` after creating the account.

## Summary

| Mistake | Hopper detection | When |
|---------|-----------------|------|
| Remove a field | LAYOUT_ID change, Incompatible verdict | Compile time + CLI |
| Reorder fields | LAYOUT_ID change, Incompatible verdict | Compile time + CLI |
| Change field size | LAYOUT_ID change + type mismatch | Compile time + CLI |
| Change disc | Disc mismatch, assertion failure | Compile time |
| Skip header | Wrong memory access | Runtime (load rejects) |
| Same version, different fields | Version collision detected | Compile time + CLI |
| No header init | Zero header, load rejects | Runtime (load rejects) |

The rule is simple: **append only, increment version, never touch existing
field offsets**. Hopper enforces this at every layer.
