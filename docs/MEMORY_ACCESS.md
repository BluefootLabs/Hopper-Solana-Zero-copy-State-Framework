# Memory Access Doctrine

Hopper does not reject pointer casting. It civilizes it.

This document describes the three memory access tiers Hopper supports, what
each one guarantees, and when to use them.

## Tier A: Safe Overlay Path (default)

The default path for most programs. You define a layout with `hopper_layout!`,
load it through tiered validation, and access fields through typed overlays.

```rust
let vault = Vault::load(account, program_id)?;
let data = account.try_borrow_mut_data()?;
let v = overlay_mut::<Vault>(&mut data[HEADER_LEN..])?;
v.balance.set(v.balance.get() + amount);
```

### What you get

- Zero-copy field access. No serialization or deserialization cycle.
- ABI-safe overlays: `#[repr(C)]` structs over `[u8; N]` wire types.
- Deterministic layout fingerprints verified at load time.
- Version-aware headers (disc, version, flags, layout_id).
- Segment-aware access for variable-length accounts.
- Full compatibility with receipts, policy, CLI tooling, and migration planning.

### When to use it

Always, unless you have a measured reason not to. This path gives you the full
Hopper pipeline: define, resolve, validate, execute, record, verify, inspect.

## Tier B: Explicit Pod Path

For hot paths, fixed-layout inner loops, or when you need a direct typed view
without the full pipeline overhead.

```rust
let vault = pod_from_bytes::<VaultLayout>(data)?;
let balance = vault.balance.get();
```

### What you get

- Direct typed byte-backed view.
- No heap allocation.
- No serialization.
- Works for any type that implements `Pod` + `FixedLayout` (align-1, all bit
  patterns valid).
- Still respects Hopper's `#[repr(C)]` wire type discipline.

### What you give up

- No header validation unless you do it yourself.
- No version checking.
- No fingerprint verification.
- No segment awareness.
- No receipt tracking.

### When to use it

When you have already validated the account through Tier A and need to re-access
specific fields in a tight loop. Or when reading a known fixed-format region
(like a Token account through `hopper-solana` readers).

## Tier C: Unsafe Raw Path

Full control. Hopper gets out of your way.

```rust
let vault = unsafe {
    // SAFETY: caller has verified data.len() >= Vault::LEN and data is
    // aligned for Vault (always true: Vault is align-1 via wire types).
    &*(data.as_ptr() as *const Vault)
};
```

Or through the generated API:

```rust
let vault = unsafe { Vault::load_unchecked(data) };
```

Or the free function (no layout-specific method needed):

```rust
let vault = unsafe { cast_unchecked::<Vault>(data) };
```

### What you get

- Raw pointer-cast access.
- API shape only (the cast is well-typed by the macro).

### What you give up

- All runtime validation. Hopper guarantees nothing about the data content.
- Caller owns layout, compatibility, and upgrade risk.
- No receipts, no fingerprint checks, no version guards.

### When to use it

Init-time writes where you just created the account and will write the header
next. Specialized migration paths. Performance-critical code where you can
prove correctness through other means.

## Performance Reality

The cost difference between tiers:

| Operation | Approx CU | Notes |
|-----------|-----------|-------|
| Overlay cast (Tier A, after validation) | ~8 | Pointer arithmetic only |
| Pod cast (Tier B) | ~8 | Same underlying mechanism |
| Raw cast (Tier C) | ~8 | Same underlying mechanism |
| Full Tier A load with validation | ~120 | Owner + disc + version + layout_id + size |
| Header write | ~30 | One-time per init |
| Receipt begin + commit | ~40-60 | Depends on snapshot size |

The cast itself is the same cost at every tier. The difference is in what
validation you run before the cast and what tracking you run after mutation.
For most programs, the ~120 CU per account load is noise compared to CPI
costs (thousands of CU).

## Pointer-Cast Performance, Civilized

Hopper maps fixed-layout zero-copy views directly onto account bytes with no
heap allocation and no Borsh-style serialization cycle.

Unlike naive pointer-cast approaches, Hopper layers this on top of:

- ABI-safe overlays with compile-time size and alignment assertions
- Versioned headers with deterministic layout fingerprints
- Five-tier loading from full validation to raw unchecked
- Segmented accounts with typed roles
- State receipts tracking every mutation
- CLI tooling that can explain any account from raw hex

Hopper delivers pointer-cast class performance through a safer, more evolvable,
more inspectable state model. The raw path exists when you need it. The safe
path is what you should reach for first.

## Trust Hierarchy

Every tier carries an implicit level of trust. Know which one you are using
and why.

| Tier | Trust Level | Who Owns Correctness |
|------|-------------|---------------------|
| **A** | Framework-verified | Hopper validates owner, disc, version, layout_id, size, fingerprint. The framework rejects bad data before your code runs. |
| **B** | Caller-asserted | You. The caller has already validated the account through Tier A or equivalent logic and is re-accessing fields in a hot loop. Hopper provides the typed view but no runtime checks. |
| **C** | Caller-owned | You entirely. Hopper contributes only the `#[repr(C)]` type shape. No header check, no version check, no fingerprint check, no receipt tracking. You are responsible for proving the data is valid. |

**Tier A is the default and recommended Hopper path.**

Use it for every account load in normal program flow. The ~120 CU overhead
is negligible compared to CPI costs and provides full auditability through
receipts and CLI inspection.

**Tier B is for validated hot paths.**

Use it when you have already validated through Tier A and need to re-read a
field many times in the same instruction (e.g. iterating a collection). The
Pod cast is safe as long as the data was validated on first access.

**Tier C is for explicit expert-owned risk.**

Use it for init-time writes (no header yet to validate), specialized migration
code, or extreme-optimization paths where you can prove correctness through
other means. Document your safety invariants with `// SAFETY:` comments.
