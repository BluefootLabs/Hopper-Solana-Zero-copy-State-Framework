# Hopper Lang

Hopper Lang is not a new programming language.

It is the canonical way to write Hopper programs -- a set of conventions,
patterns, and library primitives that together produce programs that are
safer, faster, and easier to audit than anything Anchor, Pinocchio, or
Quasar can produce today.

---

## Core Rule

Every Hopper instruction follows this sequence:

**Validate -> Load -> Mutate -> Emit**

Never mutate before validation. Never emit before mutation completes.

---

## The Two Validation Styles

Hopper provides two intentional validation styles. Both are in the prelude.
Use whichever reads best for the situation.

### Guards (free functions, fire-and-forget)

```rust
require_payer(payer)?;
require_owner(vault, program_id)?;
require_disc(vault, 1)?;
require_layout::<Vault>(vault)?;
```

Guards return `ProgramResult`. Use them for sequential top-of-handler
checks where you do not need to chain.

### Chainable checks (methods on AccountView)

```rust
vault.check_signer()?
     .check_writable()?
     .check_owned_by(program_id)?
     .check_disc(1)?;
```

Chainable checks return `Result<&Self, ProgramError>`. Use them when
you want to compose multiple validations on the same account in a single
expression.

### Compound checks (core check module)

```rust
check_account(vault, program_id, 1, Vault::SIZE)?;
check_has_one(&vault_data.authority, signer)?;
verify_pda_cached(vault, &seeds, Vault::BUMP_OFFSET, program_id)?;
```

For combined checks (owner + disc + size in one call), PDA verification
with cached bumps (~200 CU vs ~1500 CU), and cross-account assertions.

---

## Rules

### 1. Validate first

Always validate before loading state:
- Signer requirements (`require_signer`, `check_signer`)
- Writable requirements (`require_writable`, `check_writable`)
- Ownership (`require_owner`, `check_owned_by`)
- Layout / discriminator (`require_disc`, `require_layout`, `check_account`)
- PDA (`verify_pda`, `verify_pda_cached`)
- Account uniqueness (`require_unique_2`, `require_unique_3`)

### 2. Prefer typed overlays

```rust
// Good: typed, bounds-checked, versioned
let vault = Vault::load_mut(account, program_id)?;
let v = vault.get_mut();
v.balance = WireU64::new(amount);

// Avoid: manual byte slicing
let balance = u64::from_le_bytes(data[48..56].try_into()?);
```

Typed overlays give you: header validation, layout_id fingerprinting,
version checking, and correct field alignment. Manual byte slicing
gives you none of these.

### 3. Use phased execution for complex handlers

```rust
PhasedFrame::new(program_id, accounts, data)?
    .resolve(2, |accts, _| Ok((&accts[0], &accts[1])))?
    .validate(|ctx, pid| {
        require_payer(ctx.0)?;
        require_owner(ctx.1, pid)?;
        Ok(())
    })?
    .execute(|ctx| {
        let mut vault = Vault::load_mut(ctx.resolved().1, ctx.program_id())?;
        // ... mutate state ...
        Ok(())
    })
```

PhasedFrame enforces the Validate -> Load -> Mutate pipeline at the type
level. You cannot execute before validation, and you cannot validate
before resolution.

### 4. Prefer checked CPI

Use checked CPI by default. Hopper Native provides tiered CPI safety:

| Tier | Function | What it checks |
|------|----------|----------------|
| Safe | `CreateAccount.invoke()` | Account count, ownership, signer/writable |
| Expert | Bounded CPI | Verifiable + LamportSnapshot/DataFingerprint |
| Raw | `syscalls::sol_invoke_signed_rust` | Nothing -- caller owns all risk |

Only use raw CPI in audited expert paths or proven-safe hot loops.

### 5. Keep state mutation explicit

Mutation should happen inside a clearly bounded block. Do not mix
validation, mutation, and event emission into one monolithic function.

```rust
// Good: each phase is a separate closure
.validate(|ctx, pid| { ... })?
.execute(|ctx| { ... })

// Good: validation section, then mutation section
require_signer(authority)?;
require_writable(vault)?;
// -- validation above, mutation below --
let mut state = Vault::load_mut(vault, program_id)?;
state.get_mut().balance = WireU64::new(new_balance);
```

### 6. Emit meaningful receipts

State changes should produce auditable receipts:

```rust
let mut receipt = StateReceipt::<256>::begin(&Vault::LAYOUT_ID, account_data);
// ... mutate state ...
receipt.commit(account_data);
emit_receipt(&receipt.to_bytes())?;
```

Receipts create an on-chain audit trail. Use `emit_tagged_receipt` for
categorized events, `set_return_data` for CPI return values, and
`emit_typed_receipt` with the `Receipt` trait for typed emission.

### 7. Avoid hidden assumptions

Do not rely on:
- Implicit account ordering without validation
- Hidden init state
- Hidden ownership expectations
- Unchecked PDA derivation

Make every assumption explicit in code. Hopper provides the tools
(`check_account`, `require_layout`, `verify_pda_cached`) to validate
every assumption cheaply.

### 8. Use ValidationGraph for complex programs

```rust
let mut graph = ValidationGraph::new();
graph.add("signer", check_signer(depositor));
graph.add("owner", check_owner(pool, program_id));
graph.add("writable", check_writable(pool));
graph.run_all()?;
```

ValidationGraph collects all validation results before failing, giving
auditors a complete picture of what was checked and where failures occur.

---

## Memory Access Tiers

| Tier | Path | Safety | Cost |
|------|------|--------|------|
| A | `Vault::load(account, pid)?` | Full: header + fingerprint + ownership | ~40 CU |
| B | `pod_from_bytes::<Vault>(data)?` | Partial: bounds + alignment only | ~10 CU |
| C | `unsafe { Vault::load_unchecked(data) }` | None: raw pointer cast | ~2 CU |

Most programs use Tier A. Drop to Tier B for hot inner loops on already-
validated accounts. Tier C is for substrate code only.

---

## Account Loading Tiers

| Tier | Method | Trust Level |
|------|--------|-------------|
| T1 | `load()` | Your program's accounts: full validation |
| T2 | `load_foreign()` | Cross-program reads: ABI proof required |
| T3 | `load_compatible()` | Migration: accept older versions |
| T4 | `load_unchecked()` | Expert: skip layout_id check |
| T5 | `load_unverified()` | Raw: no checks at all |

On macro-generated layout types, the migration-friendly helper is
`load_compatible()`. On a raw `AccountView`, the runtime-first equivalent is
`account.load_versioned::<T>()`.

---

## What Makes Hopper Lang Different

| Feature | Anchor | Pinocchio | Quasar | Hopper |
|---------|--------|-----------|--------|--------|
| Validation style | Derive macros | Manual | Manual | Guards + Chainable + Graph |
| Execution phases | None | None | None | PhasedFrame (type-enforced) |
| Layout evolution | None | None | None | Versioned headers + fingerprints |
| CU cost of state access | ~200 CU (deser) | ~10 CU (cast) | ~10 CU (cast) | ~10 CU (cast) + optional validation |
| Receipts | None | None | None | 64-byte StateReceipt |
| Collections | None | None | None | FixedVec, RingBuffer, PackedMap, Journal, Slab |
| Cross-program reads | IDL sharing | Raw casts | Interfaces | hopper_interface! with ABI proof |
| PDA optimization | find_program_address | Manual | BUMP_OFFSET | verify_pda_cached (~200 CU) |
| CLI tooling | anchor cli | None | None | explain, inspect, diff, plan, receipt, manager, fetch, client gen |
| Backend portability | solana-program only | pinocchio only | pinocchio only | 3 backends, Hopper Native default |

---

## Hopper Style Goal

Hopper programs should read like a specification:
- Every assumption validated
- Every phase separated
- Every state change recorded
- Every account proven

Clear enough for an auditor. Fast enough for a market maker.
Strict enough that bugs cannot hide.
