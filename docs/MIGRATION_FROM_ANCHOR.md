# Migrating from Anchor to Hopper

This is the side-by-side. If you know Anchor, you can port a program in an afternoon. The macro spelling is almost identical; the mental model is different in two specific ways (zero-copy throughout, and segment-level borrow tracking), and knowing that up front saves the "why won't my `Account<T>` compile" moment.

## The 30-second summary

| Anchor | Hopper |
| --- | --- |
| `#[program] mod my_program { ... }` | `#[program] mod my_program { ... }` |
| `#[account(zero_copy)] pub struct Vault { ... }` | `#[account] #[repr(C)] pub struct Vault { ... }` |
| `#[derive(Accounts)] pub struct Deposit<'info> { ... }` | `#[derive(Accounts)] pub struct Deposit<'info> { ... }` |
| `AccountLoader<'info, Vault>` | `Account<'info, Vault>` |
| `#[account(mut)] pub vault: Account<'info, Vault>` | `#[account(mut)] pub vault: Account<'info, Vault>` |
| `ctx.accounts.vault.load_mut()?.balance` | `ctx.accounts.vault.get_mut()?.balance` |
| `ctx.bumps.vault` | `ctx.bumps.vault` |
| `emit!(Event { .. })` | `emit!(Event { .. })` |
| `require!(x, ErrorCode::Foo)` | `require!(x, ErrorCode::Foo)` |
| `Pubkey` | `Address` (same 32-byte shape) |

Read that table once. Most mechanical edits are on it.

## Anchor to Hopper in five minutes

Keep the handler shape familiar, then move the state borrow behind an accounts
method so every instruction reads as `ctx.accounts.*`:

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Vault {
    pub authority: Address,
    pub balance: WireU64,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
    pub authority: Signer<'info>,
}

impl<'info> Deposit<'info> {
    pub fn deposit(&self, amount: u64) -> ProgramResult {
        let mut vault = self.vault.get_mut()?;
        vault.balance.checked_add_assign(amount)
    }
}

#[program]
mod vault_program {
    use super::*;

    #[instruction(0)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }
}
```

That is the whole first port: `AccountLoader` becomes `Account`, `load_mut()`
becomes `get_mut()`, native integers become wire types, and the public handler
stays small.

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

Anchor's `#[derive(Accounts)]` stays `#[derive(Accounts)]`. The field-level constraint syntax is the same in both frameworks, and Hopper also keeps the lower-level `#[accounts]` attribute for systems-style declarations.

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
#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut, seeds = [b"vault", authority_key.as_ref()], bump = vault.load()?.bump)]
    pub vault: Account<'info, Vault>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}
```

Three differences:

1. `AccountLoader<'info, Vault>` becomes `Account<'info, Vault>`.
2. `load_mut()` becomes `get_mut()` on Hopper's wrapper, returning the same zero-copy borrow.
3. `System` is a Hopper marker for the canonical System Program ID.

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
pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
    let mut vault = ctx.accounts.vault.get_mut()?;
    vault.balance.checked_add_assign(amount)?;
    Ok(())
}
```

Two things to know:

1. Handlers carry an `#[instruction(N)]` attribute that declares the discriminator byte. Anchor uses an 8-byte SHA-256 prefix of the function name; Hopper uses the user-chosen byte (or a `discriminator = [bytes]` array for multi-byte prefixes when you want Anchor-style uniqueness).
2. `ctx.accounts.vault.get_mut()?` is the Anchor-feeling default. Segment-level accessors such as `vault_balance_mut()` are still available in systems-mode code when you want disjoint field borrows instead of a full-struct borrow.

## Bumps

Use `ctx.bumps.field_name`, the same shape Anchor users expect. Hopper also retains `ctx.bumps().field_name` for older code.

## Errors

Anchor's `#[error_code]` maps directly to Hopper's `#[error_code]`:

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
#[hopper::error_code]
#[repr(u32)]
pub enum VaultError {
    #[invariant = "balance_nonzero"]
    InsufficientBalance = 0x1001,
    #[invariant = "authority_match"]
    Unauthorized = 0x1002,
}
```

Just like Anchor, you return the error straight into a `ProgramResult`:

```rust
return Err(VaultError::Unauthorized.into()); // -> ProgramError::Custom(0x1002)
```

The derive emits both `From<VaultError> for u32` and `From<VaultError> for ProgramError`, so `.into()` lands the stable code in `ProgramError::Custom(code)`.

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
#[derive(Accounts)]
pub struct Collect<'info> {
    #[account(
        mut,
        token::mint = mint,
        token::token_program = ::hopper_runtime::token::TOKEN_2022_PROGRAM_ID,
        extensions::transfer_hook::authority = hook_authority,
        extensions::transfer_hook::program_id = hook_program_id,
    )]
    pub source: UncheckedAccount<'info>,
    pub mint: UncheckedAccount<'info>,
    pub hook_authority: UncheckedAccount<'info>,
    pub hook_program_id: UncheckedAccount<'info>,
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
3. Anchor's Borsh-backed SPL `InterfaceAccount<T>` path splits in Hopper: use `InterfaceAccount<'info, T>` for Hopper-header layouts owned by a declared program set, and use `TokenProgramKind`, `InterfaceTokenAccount`, `InterfaceMint`, or direct TLV readers for SPL Token and Token-2022 bytes.

## Checklist for the port

1. Swap `#[account(zero_copy)] #[repr(C)]` to `#[account] #[repr(C)]` on each layout type.
2. Replace `u64` fields with `WireU64` (and friends for other widths).
3. Keep `#[derive(Accounts)]`; Hopper also supports `#[accounts]` for systems-style contexts.
4. Change `AccountLoader<'info, T>` to `Account<'info, T>` on context fields.
5. Replace `ctx.accounts.field.load_mut()?.subfield` with `ctx.accounts.field.get_mut()?.subfield`.
6. Keep `ctx.bumps.field`; Hopper also accepts `ctx.bumps().field` for compatibility with older Hopper examples.
7. Replace `Pubkey` with `Address`.
8. Give each handler an `#[instruction(N)]` attribute with a distinct discriminator byte.
9. Run `hopper build`. Fix whatever shows up. The errors will be clear.
10. Port your tests last. They are almost unchanged.
