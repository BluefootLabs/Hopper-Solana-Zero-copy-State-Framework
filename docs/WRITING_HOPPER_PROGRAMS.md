# Writing Hopper Programs

Hopper is a Solana framework for serious builders who want:
- pointer-cast speed
- protocol-grade safety
- explicit state evolution
- no serialization tax
- no hidden runtime magic

This guide shows the canonical way to write Hopper programs.

---

# Hopper Model

A Hopper program has 4 layers:

1. **Instruction surface**
   - Define the instruction enum / dispatch.
2. **Account validation**
   - Validate signer, writable, ownership, and discriminator/layout.
3. **Typed state access**
   - Load typed overlays directly from account bytes.
4. **Mutation + receipts**
   - Mutate state and emit a structured receipt of what changed.

---

# Canonical Hopper Instruction Shape

```rust
use hopper::prelude::*;

hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        mint:      TypedAddress<Mint>      = 32,
        balance:   WireU64                 = 8,
        bump:      u8                      = 1,
    }
}

pub enum VaultIx {
    Deposit { amount: u64 },
    Withdraw { amount: u64 },
}

pub fn process(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    match VaultIx::decode(data)? {
        VaultIx::Deposit { amount } => process_deposit(program_id, accounts, amount),
        VaultIx::Withdraw { amount } => process_withdraw(program_id, accounts, amount),
    }
}
```

---

# Canonical Validation Pattern

Use Hopper's chainable validation methods:

```rust
let vault = &accounts[0];
let authority = &accounts[1];

vault
    .check_writable()?
    .check_owned_by(program_id)?
    .check_disc(Vault::DISC)?;

authority
    .check_signer()?;
```

This is the standard Hopper validation flow.

---

# Canonical Typed Load Pattern

```rust
let vault = Vault::load(vault_account, program_id)?;
let balance = vault.map(|v| v.balance.get());
```

Prefer:

* `load()` for your own program's accounts
* `load_foreign()` for ABI-pinned foreign accounts
* `load_compatible()` for migration-compatible reads

Avoid `_unchecked` unless you are inside a proven safe initialization path.

---

# Canonical Mutation Pattern

```rust
let mut receipt = StateReceipt::<256>::begin(&Vault::LAYOUT_ID, vault_account.data()?);

vault.map_mut(|v| {
    let next = v.balance.get().checked_add(amount).unwrap();
    v.balance.set(next);
})?;

receipt.commit(vault_account.data()?);
emit_slices(&[&receipt.to_bytes()]);
```

Every Hopper state mutation should have a clear validation boundary and a clear mutation boundary.

---

# Canonical CPI Pattern

Prefer checked CPI:

```rust
hopper_runtime::cpi::invoke_signed(
    &instruction,
    &[source, destination, authority],
    signer_seeds,
)?;
```

Checked CPI validates:
- account count matches instruction expectations
- address identity (order-dependent matching)
- signer requirements
- writable requirements
- borrow compatibility

Only use unchecked CPI in tightly audited expert paths.

---

# Hopper Style Rules

## Prefer:

* explicit validation
* typed overlays
* receipts for meaningful state changes
* compatibility-aware loaders
* safe/default APIs first

## Avoid:

* hidden account assumptions
* unchecked pointer casts unless required
* magic init flows
* "just trust the bytes" programming

---

# Hopper Philosophy

Hopper is not:

* a serialization framework
* a runtime wrapper
* a macro religion

Hopper is:

* a typed state pipeline for Solana
* a sovereign runtime surface
* a serious-builder framework
