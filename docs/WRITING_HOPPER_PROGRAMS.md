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

Hopper should read like one coherent system, not a menu of competing modes:

1. **Instruction surface**
    - Define dispatch and arguments.
2. **Validation**
    - Prove signer, writable, ownership, and layout expectations.
3. **Access**
    - Use `load()` / `load_mut()` for whole layouts, `segment_ref()` /
      `segment_mut()` for precise regions, and raw access only as an explicit
      unsafe escape hatch.
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

* `load()` / `load_mut()` for Hopper-owned whole-layout access
* `load_foreign()` when the guarantee changes because the account is foreign
* `load_versioned()` / `load_compatible()` when the guarantee changes because you are in a migration window

Avoid raw or unchecked access unless you are inside a proven safe initialization
path or a deliberately audited hot path.

# Canonical Segment Pattern

When you want byte-range precision instead of whole-layout projection, keep the
same access story and change only the guarantee level:

```rust
let core = ctx.account(0)?;
let balance = core.segment_mut::<WireU64>(ctx.borrows_mut(), 32, 8)?;
balance.set(balance.get() + amount);
```

Generated proc-macro accessors such as `ctx.vault_balance_mut()?` are just a
typed front-end over that same runtime call.

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
