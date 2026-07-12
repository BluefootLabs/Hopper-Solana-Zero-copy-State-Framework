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
| `pub referral: Option<Account<'info, Vault>>` | `pub referral: Option<Account<'info, Vault>>` (same absence convention: pass the program's own id in the slot; clients port unchanged) |
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

### Composite (nested) accounts structs

Anchor lets one accounts struct embed another; Hopper spells the same
thing with an explicit `#[composite]` marker (Anchor infers it from any
non-wrapper field type — Hopper refuses to guess):

```rust
// Anchor
#[derive(Accounts)]
pub struct Operate<'info> {
    pub payer: Signer<'info>,
    pub check: VaultCheck<'info>,      // composite, inferred
    #[account(mut)]
    pub tail: Account<'info, Vault>,
}

// Hopper
#[derive(Accounts)]
pub struct Operate<'info> {
    pub payer: Signer<'info>,
    #[composite]
    pub check: VaultCheck<'info>,      // composite, declared
    #[account(mut)]
    pub tail: Account<'info, Vault>,
}
```

Slots flatten in declaration order exactly like Anchor
(`payer, check.authority, check.vault, tail`), `ctx.bumps.check` nests
the inner context's bumps, and clients pass the same flat account list.

Since composite v2 the CONTAINER's options compose across the nesting
boundary: `strict_writes` (and its `lamports(...)` dimension) splice the
inner context's declared `mut` / `mut(seg)` ranges into the outer's
enforced write-set at compile time with account indices rebased to the
flattened slots; `event_cpi`'s two auto-appended slots trail the
flattened set; `emit_touch_map` and `auto_lifecycle` work unchanged.

Remaining restrictions (compile errors, not silent gaps): the composite
field cannot be `Option<..>` or carry its own `#[account(...)]`
constraints; the INNER context must stay a plain validation context (no
`#[instruction(...)]` args, no `strict_writes` / `lamports(...)` /
`emit_touch_map` / `event_cpi` options, no lifecycle or `migrate(...)`,
no nested `#[composite]` of its own); and `lamports(...)` on the outer
can only name the outer's own leaf fields — an account inside an
embedded context cannot be granted lamport permission from the outer
(flatten the inner context if one of its accounts must move lamports).

### Lazy migration at bind

anchor-next's borsh `Migration<A, B>` design stops at a dedicated
migration instruction. Hopper goes one step further: declare the
previous layout version on the field, and EVERY instruction that binds
the context becomes a migration crank — accounts upgrade as they are
touched, no dedicated instruction, no separate rollout:

```rust
fn v1_to_v2(old: &VaultV1, new: &mut VaultV2) -> Result<(), ProgramError> {
    new.authority = old.authority;
    new.total = WireU64::new(old.total_u32.get() as u64); // widen
    Ok(())                     // unset V2 fields default to zeroed bytes
}

#[derive(Accounts)]
pub struct Touch<'info> {
    pub authority: Signer<'info>,
    #[account(mut, migrate(from = VaultV1, with = v1_to_v2))]
    pub vault: Account<'info, VaultV2>,
}
```

`bind()` probes the slot for a **fully-valid** `VaultV1` header (the
complete disc/version/layout-id/epoch identity, never a sniff) and only
then runs `hopper::migration::migrate_layout::<VaultV1, VaultV2, _>` —
typed on both sides, in place, header re-stamped LAST with account
flags preserved — before any validator runs. An already-migrated
account skips the probe; any other header fails with the normal
`VaultV2` validation error, unchanged. The standalone read-only
`validate()` accepts either version without writing: its layout-header
check for the field becomes "valid `VaultV2`, or fully-valid `VaultV1`
whose allocation already fits `VaultV2`" — the same sets `bind()`
accepts. A migration error fails the instruction, so the runtime rolls
every byte back (the same transaction-abort atomicity the runtime
migration helpers document).

v1 restrictions, plainly:

- `migrate(...)` requires `mut` on the same field, and is a compile
  error combined with `init` / `init_if_needed` / `zero` / `close` /
  `realloc` / `sweep`, on `Option<..>` fields, on `#[composite]`
  fields, or with `from` naming the field's own layout.
- In-place only: the new shape must already fit the existing
  allocation. Too-small V1 accounts are refused by both `bind()` and
  `validate()`; `realloc` in a prior instruction when V2 is wider.
- A context carrying a migrate field is **not embeddable** as a
  `#[composite]` inner (compile error at the embedding site): the
  pre-step lives in that context's own `bind()`, which an outer
  composite bind never invokes, so embedding would silently stop the
  crank — Hopper refuses instead. Using it as the OUTER container (or
  standalone) is fine.
- Only the field's layout-header check is version-widened. Constraints
  that read *through* the layout (`has_one`, custom `constraint`
  expressions) still evaluate against the NEW shape, so standalone
  `validate()` on a not-yet-migrated account can fail such a
  constraint even though `bind()` (which migrates first) succeeds.
- One `from` version per field: chains (V1→V2→V3) mean the field
  declares only the immediately-previous version; older accounts need
  the intermediate crank first.

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

Identical call site. Self-CPI events (what Anchor spells `#[event_cpi]` + `emit_cpi!`) are the same shape in Hopper — one attribute option, one call:

```rust
// Anchor
#[event_cpi]
#[derive(Accounts)]
pub struct Deposit<'info> { /* ... */ }

pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    emit_cpi!(Deposited { amount });
    Ok(())
}

// Hopper
#[hopper::context(event_cpi)]
pub struct Deposit { /* ... */ }

#[instruction(0)]
fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
    ctx.emit_event_cpi(&Deposited { amount: WireU64::new(amount) })?;
    Ok(())
}
```

Both append the same two trailing accounts (event-authority PDA + the program account) and both authenticate the self-CPI in the dispatcher, so ported clients pass the same account shape. Differences worth knowing: Hopper's wire is `[0xE0, 0x1E, tag, payload]` — 3 bytes of instruction-data overhead per event against Anchor's 16 (8-byte instruction tag + 8-byte event discriminator) — and the event-authority seed is `b"__hopper_event_authority"` (not Anchor's `b"__event_authority"`), so indexers must derive the Hopper PDA. Anchor pins the authority against a compile-time constant; Hopper has no compile-time program id, so bind and the sink verify at runtime via a sha256-only compare loop (~200 CU at bump 255). The manual escape hatch `hopper_emit_cpi!` remains for raw handlers.

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

1. `init_if_needed` DOES translate — same spelling, same shape
   (`#[account(init_if_needed, payer = ..., space = ...)]`): an empty
   slot takes the full init lifecycle CPI; a nonempty slot skips the
   CPI and must already pass the owner + layout-header checks, so a
   foreign or half-written account is refused rather than adopted. The
   security posture carries over from Anchor's own feature-gate
   warning: the reinitialization-attack surface is yours to reason
   about — prefer plain `init` unless the create-or-open pattern is
   genuinely required.
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
