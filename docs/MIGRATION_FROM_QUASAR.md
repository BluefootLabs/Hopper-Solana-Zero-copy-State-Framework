# Migrating from Quasar to Hopper

If you have a Quasar program, the mechanical port is smaller than the Anchor port. Quasar and Hopper share the same zero-copy model, `#![no_std]` posture, and pointer-cast account access. What changes is the macro spelling, the safety surface, and the constraint vocabulary.

## The 30-second summary

| Quasar | Hopper |
| --- | --- |
| `#[program]` | `#[program]` |
| `#[account]` | `#[account]` |
| `#[derive(Accounts)]` | `#[accounts]` |
| `#[instruction(discriminator = [0x1])]` | `#[instruction(discriminator = [0x1])]` or `#[instruction(1)]` |
| `#[derive(QuasarSerialize)]` | `#[hopper::args]` |
| `emit_event_cpi!` | `hopper_emit_cpi!` |
| `Ctx<'info, T>` | `Context<'info>` with bound `TCtx` |
| `ctx.accounts.field` | `ctx.field_*()` (segment-level accessors) |
| `Pod` primitives in `quasar-pod` | Wire types in `hopper-runtime` |
| `QuasarError::RemainingAccountDuplicate` | `hopper_runtime::remaining::RemainingError::DuplicateAccount` |

## Account layouts

Same shape, different top-level macro.

```rust
// Quasar
#[account(discriminator = 1)]
#[repr(C)]
pub struct Vault {
    pub authority: Address,
    pub balance: PodU64,
    pub bump: u8,
}

// Hopper
#[account]
#[repr(C)]
pub struct Vault {
    pub authority: [u8; 32],
    pub balance: WireU64,
    pub bump: u8,
}
```

Quasar's `PodU64` is Hopper's `WireU64`. Both are `#[repr(transparent)]` alignment-1 wrappers with identical byte layout. The accessors are `.get()` / `.set()` in both.

Quasar's explicit `discriminator = 1` maps to Hopper's layout header: Hopper stamps a header byte at offset 0 containing the user-chosen `disc` from the macro (defaults to a fingerprint of the type name if not set). To match Quasar's behavior exactly, use `#[account(disc = 1)]` or the macro attribute form `#[account(discriminator = 1)]`.

## Accounts struct

Same constraint vocabulary with three Hopper-only additions.

```rust
// Quasar
#[derive(Accounts)]
pub struct Swap {
    #[account(mut, seeds = Vault::seeds(&nonce), bump)]
    pub vault: Account<Vault>,
    #[account(mut, signer)]
    pub authority: Account<Signer>,
    pub remaining: RemainingAccounts,
}

// Hopper
#[accounts]
pub struct Swap {
    #[account(mut, seeds_fn = Vault::seeds(&nonce), bump)]
    pub vault: Vault,
    #[account(mut, signer)]
    pub authority: AccountView,
    // remaining accounts exposed as ctx.remaining_accounts() on the
    // bound context, no field needed.
}
```

Key differences:

1. `seeds = Type::seeds(...)` becomes `seeds_fn = Type::seeds(...)`. The underscore disambiguates typed seeds from the inline array form (`seeds = [...]`), which Hopper also supports.
2. Remaining accounts are exposed on the bound context, not as a struct field. Call `ctx.remaining_accounts()` for strict mode or `ctx.remaining_accounts_passthrough()` for the duplicate-preserving mode.
3. Hopper's `AccountView` is the raw-byte counterpart to Quasar's `Account<Signer>`. Identical semantic.

## Handler

```rust
// Quasar
#[instruction(discriminator = [0])]
pub fn swap(ctx: Ctx<Swap>, nonce: u64) -> Result<()> {
    let mut vault = ctx.accounts.vault.load_mut()?;
    vault.balance.set(vault.balance.get() + 1);
    Ok(())
}

// Hopper
#[instruction(discriminator = [0])]
pub fn swap(ctx: Context<Swap>, nonce: u64) -> ProgramResult {
    let balance = ctx.vault_balance_mut()?;
    balance.set(balance.get() + 1);
    Ok(())
}
```

The discriminator syntax is identical. Hopper also accepts the short `#[instruction(0)]` form for single-byte discriminators when you do not need a multi-byte prefix.

Quasar's `ctx.accounts.vault.load_mut()?` returns a `RefMut<Vault>` covering the entire account. Hopper's `ctx.vault_balance_mut()?` returns a `RefMut<WireU64>` covering only the balance slot. Two handlers can legally borrow different slots on the same account concurrently; Hopper's segment-level borrow registry tracks the disjoint regions.

If you want the Quasar-style full-struct borrow, use `ctx.vault_mut()?` (which Hopper also emits). Both coexist on the bound context.

## Instruction args

Quasar's `#[derive(QuasarSerialize)]` is replaced by `#[hopper::args]`.

```rust
// Quasar
#[derive(QuasarSerialize)]
pub struct SwapArgs {
    pub amount: u64,
    pub referrer: Option<[u8; 32]>,
}

// Hopper
#[hopper::args]
#[repr(C)]
pub struct SwapArgs {
    pub amount: WireU64,
    pub referrer: OptionByte<[u8; 32]>,
}
```

`OptionByte<T>` is Hopper's equivalent of Quasar's `OptionZc<T>`. Same semantic: one tag byte, one payload, tag validation rejects anything other than 0 or 1 on `parse_checked()`.

## CPI events

```rust
// Quasar
emit_event_cpi!(ctx, Deposited { amount, depositor });

// Hopper
hopper_emit_cpi!(
    ctx.program_id(),
    ctx.event_authority_account()?,
    bumps.event_authority,
    Deposited { amount, depositor },
);
```

Hopper takes the signer seeds explicitly so the macro does not need to know about a canonical event-authority account layout at expansion time. Conceptually the same invoke_signed pattern; Hopper exposes the moving parts.

## Errors

```rust
// Quasar
#[error_code]
pub enum VaultError {
    #[msg("Insufficient balance")]
    InsufficientBalance,
}

// Hopper
#[hopper::error]
#[repr(u32)]
pub enum VaultError {
    #[invariant = "balance_nonzero"]
    InsufficientBalance = 0x1001,
}
```

Hopper exposes the error code explicitly. If you want Anchor-style auto-assignment starting at 6000, pick your own base and increment.

## Remaining accounts

The shape you used in Quasar maps one-for-one:

```rust
// Quasar
for acct in ctx.accounts.remaining.iter() {
    let acct = acct?;
    // ...
}

// Hopper
for acct in ctx.remaining_accounts().iter() {
    let acct = acct?;
    // ...
}
```

Strict mode is the default in both. Hopper's passthrough mode is `ctx.remaining_accounts_passthrough()`; Quasar spells it with a constructor argument.

## Profile and tooling

`quasar profile` is `hopper profile`. Two subcommands:

- `hopper profile bench` - primitive benchmark lab against a live cluster. JSON and CSV regression artifacts.
- `hopper profile elf <path.so>` - static ELF + symbol-size analysis with flamegraph output.

`hopper build --watch` and `hopper test --watch` match Quasar's `--watch` flags.

## What Hopper adds on top of Quasar

Things Quasar does not have that your port gets for free:

1. Schema-epoch migrations (`#[hopper::migrate(from = 1, to = 2)]`).
2. Provable `StateReceipt` wire format with invariant-linked error codes and failure-stage indices.
3. Compile-time layout compatibility (`hopper_assert_compatible!`) and fingerprint pinning (`hopper_assert_fingerprint!`).
4. Full Token-2022 extension constraint block (`extensions::transfer_hook::*`, `metadata_pointer::*`, `permanent_delegate`, `non_transferable`, `immutable_owner`, `mint_close_authority`, `transfer_fee_config::*`, `interest_bearing::*`, `default_account_state`).
5. Segment-level mutable and read-only borrows on the same account.
6. Policy levers (`strict`, `sealed`, `raw`) at the program and per-handler grain.
7. Python and Kotlin client generators in addition to TypeScript.
8. On-chain manifest PDA so indexers can fetch the schema without source access.

## Checklist for the port

1. Rename `#[derive(Accounts)]` to `#[accounts]`.
2. Change `Ctx<'info, T>` to `Context<'info>` in handler signatures (the bound type is implicit).
3. Rename `Account<MyLayout>` to plain `MyLayout` on context fields.
4. Replace `ctx.accounts.field.load()` and `.load_mut()` with the segment accessors Hopper emits (`ctx.field_sub()` / `ctx.field_sub_mut()`).
5. Rename `QuasarSerialize` to `hopper::args`. Swap `OptionZc` for `OptionByte` (or `Option` where you want the niche-optimized form).
6. Replace `seeds = Type::seeds(...)` with `seeds_fn = Type::seeds(...)`.
7. Move `RemainingAccounts` field references to `ctx.remaining_accounts()` on the bound context.
8. Rename `emit_event_cpi!` to `hopper_emit_cpi!` and thread the event-authority pubkey plus stored bump through.
9. Run `hopper build`. Fix whatever compiler errors surface. They will mostly be naming.
10. Read the `docs/TOKEN_2022_GUIDE.md` if your program touches Token-2022; Hopper's constraint surface is a clear superset of what you were using before.
