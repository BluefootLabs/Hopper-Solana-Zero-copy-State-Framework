# Cross-Program Read Example

Demonstrates Hopper's **interface pinning** pattern: reading another program's
account with zero crate dependencies, secured by deterministic layout fingerprints.

## Architecture

```
cross-program-read/
├── program-a/    ← Defines and owns a Vault account (hopper_layout!)
└── program-b/    ← Reads Program A's Vault (hopper_interface!), NO dependency on A
```

## How It Works

1. **Program A** uses `hopper_layout!` to define `Vault` with fields
   `authority`, `balance`, and `bump`.

2. **Program B** uses `hopper_interface!` to declare an identical `Vault`
   struct with the same field spec. Because the SHA-256 hash of the field
   descriptors is deterministic, both structs produce the **same `LAYOUT_ID`**.

3. When Program B calls `Vault::load_foreign(account, &PROGRAM_A_ID)`:
   - Verifies `account.owner == PROGRAM_A_ID`
   - Verifies `account.layout_id == VaultView::LAYOUT_ID`
   - Verifies `account.data.len() == VaultView::LEN`
   - Returns a `VerifiedAccount<Vault>` typed overlay

4. If Program A ever changes its Vault layout (adds/removes/reorders fields),
   the `LAYOUT_ID` changes and Program B's `load_foreign()` fails, preventing
   silent schema drift.

## Key Differences vs Import-Based Reads

| Approach | Coupling | Trust Model | Schema Drift Risk |
|----------|---------|-------------|-------------------|
| Import Program A's crate | Compile-time | Full crate dependency | Caught at compile time |
| **hopper_interface! (this)** | **None** | **Runtime ABI proof** | **Caught at runtime via LAYOUT_ID** |
| Raw byte slicing | None | No safety | **Undetected** |

## Usage Patterns

### Basic Cross-Program Read
```rust
let verified = Vault::load_foreign(account, &PROGRAM_A_ID)?;
let balance = verified.get().balance.get();
```

### With TrustProfile (configurable validation)
```rust
let profile = TrustProfile::strict(&PROGRAM_A_ID, &Vault::LAYOUT_ID, Vault::LEN);
let verified = Vault::load_with_profile(account, &profile)?;
```

### Multi-Owner (Token vs Token-2022)
```rust
let (verified, owner_idx) = Vault::load_foreign_multi(account, &[&OWNER_A, &OWNER_B])?;
```

### Fingerprint Pinning (explicit ABI contract)
```rust
// Pin to a known fingerprint -- fails at compile time if layout changes
hopper_assert_fingerprint!(Vault, [0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81]);
```
