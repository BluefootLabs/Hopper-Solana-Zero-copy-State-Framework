# Migrating from Anchor to Hopper

This is the side-by-side. If you know Anchor, you can port a program in an afternoon. The macro spelling is almost identical; the mental model is different in two specific ways (zero-copy throughout, and segment-level borrow tracking), and knowing that up front saves the "why won't my `Account<T>` compile" moment.

## The 30-second summary

| Anchor | Hopper |
| --- | --- |
| `#[program] mod my_program { ... }` | `#[program] mod my_program { ... }` |
| `#[account(zero_copy)] pub struct Vault { ... }` | `#[account] #[repr(C)] pub struct Vault { ... }` |
| `#[derive(Accounts)] pub struct Deposit<'info> { ... }` | `#[accounts] pub struct Deposit { ... }` |
| `AccountLoader<'info, Vault>` | `Vault` (the account wrapper is implicit) |
| `#[account(mut)] pub vault: Account<'info, Vault>` | `#[account(mut)] pub vault: Vault` |
| `ctx.accounts.vault.load_mut()?.balance` | `ctx.vault_balance_mut()?` |
| `ctx.bumps.vault` | `ctx.bumps().vault` |
| `emit!(Event { .. })` | `emit!(Event { .. })` |
| `require!(x, ErrorCode::Foo)` | `require!(x, ErrorCode::Foo)` |
| `Pubkey` | `Address` (same 32-byte shape) |

Read that table once. Most mechanical edits are on it.

## Account layouts

Anchor's `#[account(zero_copy)]` forces `#[repr(C)]`, `Pod`, `Zeroable`, and an 8-byte discriminator. Hopper's `#[account]` does the same plus writes a 16-byte Hopper header that carries a layout fingerprint, version byte, and schema epoch. Every Hopper account starts at byte 16 of payload; the discriminator lives in byte 0.

```rust
// Anchor
#[account(zero_copy)]
#[repr(C)]
pub struct Vault {
    pub authority: Pubkey,
    pub balance: u64,
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

Use the `WireU64` / `WireU32` / `WireI64` wrappers for multi-byte integers. They are `#[repr(transparent)]` alignment-1 Pod types; accessing them is a plain `.get()` / `.set()` pair. The reason: zero-copy on SBF means every struct is alignment-1, and `u64` itself has alignment 8. The wire types close that gap without macro magic.

## Accounts struct

Anchor's `#[derive(Accounts)]` becomes Hopper's `#[accounts]`. The field-level constraint syntax is the same in both frameworks.

```rust
// Anchor
#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut, seeds = [b"vault", authority.key().as_ref()], bump = vault.load()?.bump)]
    pub vault: AccountLoader<'info, Vault>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// Hopper
#[accounts]
pub struct Deposit {
    #[account(mut, seeds = [b"vault", authority_key.as_ref()], bump = vault.load()?.bump)]
    pub vault: Vault,
    #[account(signer, mut)]
    pub authority: AccountView,
    pub system_program: Program<'_, System>,
}
```

Three differences:

1. No lifetime on the context struct. Hopper binds lifetimes at the bound-context level (`DepositCtx<'ctx, 'a>`), not the declaration.
2. `AccountLoader<'info, Vault>` becomes `Vault`. The zero-copy load is implicit in the segment accessors the macro generates.
3. `Signer<'info>` is replaced by `AccountView` with `signer` on its constraint. You can also use the `Signer` wrapper if you prefer; both work.

## Handler

```rust
// Anchor
pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    let mut vault = ctx.accounts.vault.load_mut()?;
    vault.balance += amount;
    Ok(())
}

// Hopper
#[instruction(0)]
pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
    let mut balance = ctx.vault_balance_mut()?;
    balance.set(balance.get() + amount);
    Ok(())
}
```

Two things to know:

1. Handlers carry an `#[instruction(N)]` attribute that declares the discriminator byte. Anchor uses an 8-byte SHA-256 prefix of the function name; Hopper uses the user-chosen byte (or a `discriminator = [bytes]` array for multi-byte prefixes when you want Anchor-style uniqueness).
2. Segment-level accessors like `vault_balance_mut()` replace the full `load_mut()` plus field access. You borrow exactly the u64 slot, not the whole struct. Two handlers can legally borrow `vault_balance_mut` and `vault_authority_mut` concurrently because Hopper tracks borrows at the segment level.

## Bumps

`ctx.bumps().field_name` instead of `ctx.bumps.field_name`. Same semantics, just a method call.

## Errors

Anchor's `#[error_code]` maps directly to Hopper's `#[error]`:

```rust
// Anchor
#[error_code]
pub enum VaultError {
    #[msg("Insufficient balance")]
    InsufficientBalance,
    #[msg("Unauthorized")]
    Unauthorized,
}

// Hopper
#[hopper::error]
#[repr(u32)]
pub enum VaultError {
    #[invariant = "balance_nonzero"]
    InsufficientBalance = 0x1001,
    #[invariant = "authority_match"]
    Unauthorized = 0x1002,
}
```

Hopper adds the `#[invariant = "..."]` tag that ties an error to a named runtime check. When your program fails, the off-chain SDK surfaces "Invariant `balance_nonzero` failed" instead of "Error: 0x1001". You do not need to use invariants; the plain form `InsufficientBalance` without the tag still works.

## Events

```rust
// Anchor
emit!(Deposited { amount, depositor });

// Hopper
emit!(Deposited { amount, depositor });
```

Identical call site. For self-CPI events (what Anchor spells `emit_cpi!`), Hopper's path is `hopper_emit_cpi!`. Same contract, same reliability guarantee.

## Token-2022

This is where Hopper opens up space Anchor's zero-copy path does not cover.

Anchor's `InterfaceAccount<Mint>` and `Account<TokenAccount>` are Borsh-deserialized wrappers. Every `extensions::transfer_hook::*`, `extensions::metadata_pointer::*`, and friends constraint runs against those Borsh types, which means a zero-copy program pays a deserialize tax every time it touches a Token-2022 account.

Hopper ships the same constraints on the zero-copy path. The lowering is a direct TLV byte scan, not a deserialize.

```rust
#[accounts]
pub struct Collect {
    #[account(
        mut,
        token::mint = mint,
        token::token_program = ::hopper_runtime::token::TOKEN_2022_PROGRAM_ID,
        extensions::transfer_hook::authority = hook_authority,
        extensions::transfer_hook::program_id = hook_program_id,
    )]
    pub source: AccountView,
    pub mint: AccountView,
    pub hook_authority: AccountView,
    pub hook_program_id: AccountView,
}
```

Every extension listed in the final zero-copy matrix has an equivalent constraint.

## Testing

`anchor test` becomes `hopper test`. Both delegate to `cargo test` in the project root. Hopper adds `--watch` for automatic re-runs on save.

## Deploying

`anchor deploy` becomes `hopper deploy`. Both build an SBF artifact and upload it. Hopper reads cluster URL and keypair paths from `~/.hopper/config.toml` when the flags are omitted. Use `hopper config set cluster_url devnet` once and `hopper deploy` works everywhere.

## What does not translate

1. `init_if_needed` has no Hopper equivalent. The reinitialization-attack surface is wide enough that we chose to make users be explicit. Use `init` plus an explicit branch on the account's existing-account flag if you really need the pattern.
2. Anchor's `#[derive(Accounts)]` struct-level `validate(&self)` hook is spelled `#[validate]` in Hopper with the same semantic. You opt in at the struct level; the bound context then calls your method after every built-in constraint passes.
3. Anchor's `InterfaceAccount<T>` polymorphism is replaced by the `token::token_program` / `mint::token_program` constraint overrides and the direct TLV readers. One less wrapper type to reason about.

## Checklist for the port

1. Swap `#[account(zero_copy)] #[repr(C)]` to `#[account] #[repr(C)]` on each layout type.
2. Replace `u64` fields with `WireU64` (and friends for other widths).
3. Rename `#[derive(Accounts)]` to `#[accounts]`.
4. Change `AccountLoader<'info, T>` to plain `T` on context fields.
5. Replace `ctx.accounts.field.load_mut()?.subfield` with `ctx.field_subfield_mut()?`.
6. Change `ctx.bumps.field` to `ctx.bumps().field`.
7. Replace `Pubkey` with `Address`.
8. Give each handler an `#[instruction(N)]` attribute with a distinct discriminator byte.
9. Run `hopper build`. Fix whatever shows up. The errors will be clear.
10. Port your tests last. They are almost unchanged.
