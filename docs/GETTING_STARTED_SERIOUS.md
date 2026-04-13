# Getting Started with Hopper

This guide walks you through building a complete Solana program with Hopper.
Not "hello world" -- a real program with typed state, phased execution, policy
enforcement, state receipts, and CLI tooling. By the end you will have a working
SOL vault that demonstrates every layer of the Hopper pipeline.

## Prerequisites

- Rust stable (1.75+)
- Solana CLI 1.18+ (for deploying)
- `cargo-build-sbf` (comes with the Solana tool suite)

## Project Setup

Create a new SBF program crate:

```bash
cargo init --lib my-vault
cd my-vault
```

Edit `Cargo.toml`:

```toml
[package]
name = "my-vault"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "lib"]

[dependencies]
hopper = { path = "../Hopper-Solana-Zero-copy-State-Framework" }

[features]
no-entrypoint = []
```

> **Note**: Once Hopper is published to crates.io, replace the path dependency
> with `hopper = "0.1"`.

## Step 1: Define Your Layout

Every Hopper program starts by declaring its account state as a fixed-layout
struct. This is the single source of truth for your on-chain data shape.

```rust
#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

hopper_layout! {
    /// A simple SOL vault.
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}
```

What this does:

- Generates a `#[repr(C)]` struct with exact byte-level layout
- Computes a deterministic `LAYOUT_ID` (SHA-256 of field names, types, and sizes)
- Provides `Vault::LEN` (total size including the 16-byte header)
- Creates the canonical whole-layout accessors (`load`, `load_mut`) plus
    specialized variants for foreign reads, compatibility windows, and explicit
    escape hatches (`load_foreign`, `load_compatible`, `load_unchecked`,
    `load_unverified`)
- Asserts correct size and alignment at compile time

On the runtime `AccountView` API, the migration-friendly equivalent is
`account.load_versioned::<Vault>()`.

### Field Types

Hopper uses wire-safe types that guarantee alignment of 1 and little-endian
encoding:

| Type | Size | Purpose |
|------|------|---------|
| `WireU64` | 8 bytes | Unsigned 64-bit integer (use `.new(v)` / `.get()`) |
| `WireU32` | 4 bytes | Unsigned 32-bit integer |
| `WireU16` | 2 bytes | Unsigned 16-bit integer |
| `WireU128` | 16 bytes | Unsigned 128-bit integer |
| `WireI64` | 8 bytes | Signed 64-bit integer |
| `WireBool` | 1 byte | Boolean |
| `TypedAddress<T>` | 32 bytes | Pubkey with phantom type tag (Authority, Mint, Token, etc.) |
| `u8` | 1 byte | Raw byte (bump seeds, flags, enum tags) |
| `[u8; N]` | N bytes | Fixed byte arrays |

Wire integers have no arithmetic operators by design. Convert to native,
compute, convert back:

```rust
let old = vault.balance.get();          // u64
let new = old.checked_add(amount)
    .ok_or(ProgramError::ArithmeticOverflow)?;
vault.balance = WireU64::new(new);      // back to wire
```

## Step 2: Define Errors

```rust
hopper_error! {
    base = 6000;
    Unauthorized,          // 6000
    InsufficientBalance,   // 6001
    ZeroAmount,            // 6002
}
```

Each variant becomes a unit struct with a `CODE` constant and a
`From<ErrorName> for ProgramError` impl. Use them with `hopper_require!`:

```rust
hopper_require!(amount > 0, ZeroAmount);
```

## Step 3: Entrypoint and Dispatch

```rust
#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init,
        1 => process_deposit,
        2 => process_withdraw,
    }
}
```

`hopper_dispatch!` reads a 1-byte tag from `instruction_data[0]` and routes to
the matching handler. The rest of the data is passed as the `data` argument.

## Step 4: Initialize an Account

```rust
fn process_init(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer          = &accounts[0];
    let vault_account  = &accounts[1];
    let system_program = &accounts[2];

    require_payer(payer)?;
    require_writable(vault_account)?;

    // Create account via CPI, write the 16-byte header
    hopper_init!(payer, vault_account, system_program, program_id, Vault)?;

    // Write initial state
    // SAFETY: Just created -- exclusive access guaranteed.
    let data = unsafe { vault_account.borrow_unchecked_mut() };
    let vault = Vault::overlay_mut(data)?;
    vault.authority = TypedAddress::from_account(payer);
    vault.balance = WireU64::new(0);

    Ok(())
}
```

`hopper_init!` does three things:

1. Calculates the rent-exempt minimum for `Vault::LEN`
2. CPIs `CreateAccount` with the correct size and owner
3. Zero-initializes the buffer and writes the 16-byte Hopper header
   (disc, version, flags, layout_id)

After init, `overlay_mut` is the intentional low-level write path because you
just created the account and own the whole buffer. For normal authored program
flow after initialization, prefer `load_mut()` for whole-layout access or
segment access through the runtime context.

## Step 5: Phased Execution

For instructions that mutate state, use `PhasedFrame` to enforce the
Resolve-Validate-Execute pipeline at compile time:

```rust
fn process_deposit(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    // Parse arguments
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);

    // Phased frame: Resolve -> Validate -> Execute
    PhasedFrame::new(program_id, accounts, data)?
        .resolve(2, |accts, _pid| {
            Ok((&accts[0], &accts[1]))  // (depositor, vault)
        })?
        .validate(|ctx, pid| {
            require_payer(ctx.0)?;       // depositor must sign + pay
            require_owner(ctx.1, pid)?;  // vault owned by this program
            require_writable(ctx.1)?;    // vault must be writable
            Ok(())
        })?
        .execute(|ctx| {
            // Load the typed overlay
            let mut vault = Vault::load_mut(
                ctx.resolved().1,
                ctx.program_id(),
            )?;
            let v = vault.get_mut();

            // Transfer SOL: depositor -> vault
            let dep = ctx.resolved().0.lamports();
            ctx.resolved().0.set_lamports(
                dep.checked_sub(amount)
                    .ok_or(ProgramError::InsufficientFunds)?,
            );
            let vl = ctx.resolved().1.lamports();
            ctx.resolved().1.set_lamports(
                vl.checked_add(amount)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            );

            // Update balance
            let new = v.balance.get()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            v.balance = WireU64::new(new);

            Ok(())
        })
}
```

The typestate pattern means you cannot call `.execute()` without first calling
`.validate()`, and you cannot call `.validate()` without `.resolve()`. The
compiler enforces this ordering.

## Step 6: Authority-Gated Withdrawal

```rust
fn process_withdraw(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);

    PhasedFrame::new(program_id, accounts, data)?
        .resolve(2, |accts, _pid| {
            Ok((&accts[0], &accts[1]))
        })?
        .validate(|ctx, pid| {
            require_signer(ctx.0)?;
            require_owner(ctx.1, pid)?;
            require_writable(ctx.1)?;
            Ok(())
        })?
        .execute(|ctx| {
            let mut vault = Vault::load_mut(
                ctx.resolved().1,
                ctx.program_id(),
            )?;
            let v = vault.get_mut();

            // Authority check
            v.authority.require_eq_account(ctx.resolved().0)?;

            // Balance check
            let balance = v.balance.get();
            hopper_require!(balance >= amount, InsufficientBalance);
            v.balance = WireU64::new(balance - amount);

            // Transfer SOL: vault -> authority
            let vl = ctx.resolved().1.lamports();
            ctx.resolved().1.set_lamports(
                vl.checked_sub(amount)
                    .ok_or(ProgramError::InsufficientFunds)?,
            );
            let al = ctx.resolved().0.lamports();
            ctx.resolved().0.set_lamports(
                al.checked_add(amount)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            );

            Ok(())
        })
}
```

`TypedAddress::require_eq_account` compares the stored 32 bytes against the
account's key and returns `ProgramError::IllegalOwner` on mismatch.

## Step 7: Add Policy Enforcement

Policies declare what each instruction is allowed to do and what validation
requirements those capabilities trigger:

```rust
const WITHDRAW_CAPS: CapabilitySet = CapabilitySet::new()
    .with(Capability::MutatesState)
    .with(Capability::MutatesTreasury);

const WITHDRAW_POLICY: InstructionPolicy<3> = InstructionPolicy::new()
    .when(Capability::MutatesState, PolicyRequirement::Authority)
    .when(Capability::MutatesState, PolicyRequirement::InvariantCheck)
    .when(Capability::MutatesTreasury, PolicyRequirement::LamportConservation);
```

Resolve the policy to get the required checks:

```rust
let reqs = WITHDRAW_POLICY.resolve(&WITHDRAW_CAPS);
// reqs.has(PolicyRequirement::Authority)           == true
// reqs.has(PolicyRequirement::InvariantCheck)       == true
// reqs.has(PolicyRequirement::LamportConservation)  == true
```

Hopper ships 9 named policy packs for common patterns: `TREASURY_WRITE_POLICY`,
`JOURNAL_TOUCH_POLICY`, `EXTERNAL_CALL_POLICY`, `AUTHORITY_CHANGE_POLICY`,
`ACCOUNT_INIT_POLICY`, `ACCOUNT_CLOSE_POLICY`, and more. Use them directly or
compose your own.

## Step 8: Emit State Receipts

Receipts capture a structured proof of what changed during a mutation. They
record before/after fingerprints, changed byte counts, policy flags, and
segment masks in a fixed 64-byte wire format:

```rust
// Before mutation
let data = vault_account.try_borrow_data()?;
let mut receipt = StateReceipt::<256>::begin(&Vault::LAYOUT_ID, &data);

// ... perform mutations ...

// After mutation
receipt.commit(&data);
receipt.set_policy_flags(WITHDRAW_CAPS.bits());
receipt.set_invariants(true, 2);  // 2 invariants checked, all passed

// Emit as a Solana log entry
emit_slices(&[&receipt.to_bytes()]);
```

The CLI can decode these receipts from transaction logs and explain them in
plain English.

## Step 9: Close an Account

```rust
hopper_close!(vault_account, destination)?;
```

This writes a sentinel value (`[0xFF; 8]`) to prevent stale reads, transfers
all remaining lamports to the destination, and zeros account data.

## Step 10: Build and Deploy

```bash
# Build for Solana BPF
cargo build-sbf

# Deploy (requires a funded keypair)
solana program deploy target/deploy/my_vault.so
```

## Step 11: Inspect with the CLI

Build the Hopper CLI:

```bash
cargo build --bin hopper -p hopper-cli
```

Inspect any account by its hex-encoded data:

```bash
# Decode header
hopper inspect <hex-data>

# Human-readable explanation
hopper explain <hex-data>

# Check compatibility between two versions
hopper compat <hex-old> <hex-new>

# Generate a migration plan
hopper plan <hex-old> <hex-new>
```

## The Full Pipeline

Here is the mental model: every Hopper program follows seven steps. Simple
programs use steps 1-4. Complex protocols use all seven.

```
1. Define     hopper_layout! declares your state
2. Resolve    PhasedFrame::resolve() parses accounts
3. Validate   PhasedFrame::validate() checks signatures, ownership, policy
4. Execute    PhasedFrame::execute() mutates state in a controlled phase
5. Record     StateReceipt captures before/after proof
6. Verify     Invariants assert post-mutation correctness
7. Inspect    CLI decodes, explains, diffs, plans migrations
```

## Next Steps

| Where to go | What you learn |
|-------------|---------------|
| [`hopper-showcase`](../examples/hopper-showcase/src/lib.rs) | Full pipeline: all 7 steps in one program |
| [`hopper-escrow`](../examples/hopper-escrow/src/lib.rs) | Token escrow with multi-field layouts |
| [`hopper-treasury`](../examples/hopper-treasury/src/lib.rs) | Multi-segment accounts with permissions |
| [`hopper-migration`](../examples/hopper-migration/src/lib.rs) | V1 to V2 layout evolution |
| [`cross-program-read`](../examples/cross-program-read/) | `hopper_interface!` for cross-program reads |
| [The Hopper Model](THE_HOPPER_MODEL.md) | Complete framework reference |
| [Memory Access Doctrine](MEMORY_ACCESS.md) | Three-tier memory access with CU data |
| [Schema Architecture](SCHEMA_ARCHITECTURE.md) | Schema model, IDL spec, Codama projection |

## Common Patterns

### Parsing instruction arguments

Implement `InstructionArgs` and `ValidateArgs` for structured parsing:

```rust
struct DepositArgs {
    amount: u64,
}

impl<'a> InstructionArgs<'a> for DepositArgs {
    fn parse(data: &'a [u8]) -> Result<Self, ProgramError> {
        if data.len() < 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(Self {
            amount: u64::from_le_bytes([
                data[0], data[1], data[2], data[3],
                data[4], data[5], data[6], data[7],
            ]),
        })
    }
}

impl ValidateArgs for DepositArgs {
    fn validate(&self) -> Result<(), ProgramError> {
        hopper_require!(self.amount > 0, ZeroAmount);
        Ok(())
    }
}
```

Then use with `validate_with_args` and `execute_with_args`:

```rust
PhasedFrame::new(program_id, accounts, data)?
    .resolve(2, |accts, _pid| Ok((&accts[0], &accts[1])))?
    .validate_with_args(&args, |ctx, pid, _args| {
        // validation
        Ok(())
    })?
    .execute_with_args(&args, |ctx, args| {
        // use args.amount
        Ok(())
    })
```

### Cross-program reads (no crate dependency)

Program B can read Program A's accounts without importing Program A's crate.
Declare the same layout shape as an interface:

```rust
// In Program B -- no dependency on Program A
hopper_interface! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}

// Read Program A's vault account
let vault = Vault::load_foreign(account, &PROGRAM_A_ID)?;
let balance = vault.map(|v| v.balance.get());
```

The `LAYOUT_ID` is deterministic (derived from field names and types), so if
both programs declare the same struct name and fields, the fingerprints match
and cross-program reads work with zero coordination.

### Closing accounts safely

```rust
// Verify authority, then close
let vault = Vault::load(account, program_id)?;
vault.map(|v| v.authority.require_eq_account(closer))?;
hopper_close!(account, closer)?;
```

### Virtual state (multi-account entities)

Map a logical entity across multiple accounts:

```rust
let market = hopper_virtual! {
    slots = 3,
    map {
        0 => account_index: 1, owned, writable,  // core state
        1 => account_index: 2, owned,             // orderbook
        2 => account_index: 3,                    // oracle (foreign)
    }
};

market.validate(accounts, program_id)?;
let core: &MarketCore = market.overlay::<MarketCore>(accounts, 0)?;
```
