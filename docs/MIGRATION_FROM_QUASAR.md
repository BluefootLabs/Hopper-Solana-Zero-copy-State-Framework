# Migrating from Quasar to Hopper

If you have a Quasar program, the mechanical port is smaller than the Anchor port. Quasar and Hopper share the same zero-copy model, `#![no_std]` posture, and pointer-cast account access. What changes is the macro spelling, the safety surface, and the constraint vocabulary.

## The 30-second summary

| Quasar | Hopper |
| --- | --- |
| `#[program]` | `#[program]` |
| `#[account]` | `#[account]` |
| `#[derive(Accounts)]` | `#[derive(Accounts)]` |
| `#[instruction(discriminator = [0x1])]` | `#[instruction(discriminator = [0x1])]` or `#[instruction(1)]` |
| `#[derive(QuasarSerialize)]` | `#[hopper::args]` |
| `emit_event_cpi!` | `hopper_emit_cpi!` |
| `Ctx<'info, T>` | `Ctx<T>` in handlers |
| `ctx.accounts.field` | `ctx.accounts.field` |
| `Pod` primitives in `quasar-pod` | Wire types in `hopper-runtime` |
| bounded dynamic fields | `String<'a, N>` / `Vec<'a, T, N>` in `#[hopper::account]`, explicit `#[hopper::dynamic_account]`, or `#[hopper::state(dynamic_tail = T)]` |
| `QuasarError::RemainingAccountDuplicate` | `hopper_runtime::remaining::RemainingError::DuplicateAccount` |

## First-touch equivalence table

Keep the port on Hopper's framework surface first. The lower-level
raw-account and systems APIs are still available for audited escape hatches,
but they should not be the starting point for a Quasar migration.

| Quasar concept | Hopper equivalent |
| --- | --- |
| `Account<T>` | `Account<'info, T>` |
| `Signer` | `Signer<'info>` |
| `Program<T>` | `Program<'info, T>` |
| `Interface<T>` | `Interface<'info, T>` |
| `InterfaceAccount<T>` | `InterfaceAccount<'info, T>` for Hopper-header layouts owned by a declared program set |
| `UncheckedAccount` / foreign byte adapters | `UncheckedAccount<'info>` for raw access, or `ExternalAccount<'info, T: ExternalZeroCopy>` for known non-Hopper accounts |
| `set_inner()` | generated `set_inner(...)` |
| `String<'a, N>` | `String<'a, N>` in `#[hopper::account]`, or `#[tail(string<N>)]` in explicit systems-mode spelling |
| `Vec<'a, T, N>` | `Vec<'a, T, N>` in `#[hopper::account]`, or `#[tail(vec<T, N>)]` where `T: TailElement` |
| `ctx.bumps.foo` | `ctx.bumps.foo` |

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
#[account(discriminator = 1)]
#[repr(C)]
pub struct Vault {
    pub authority: [u8; 32],
    pub balance: WireU64,
    pub bump: u8,
}
```

Quasar's `PodU64` is Hopper's `WireU64`. Both are `#[repr(transparent)]` alignment-1 wrappers with identical byte layout. The accessors are `.get()` / `.set()` in both.

Quasar's explicit `discriminator = 1` maps to Hopper's layout header: Hopper stamps a header byte at offset 0 containing the user-chosen `disc` from the macro (defaults to a fingerprint of the type name if not set). To match Quasar's behavior exactly, use `#[account(disc = 1)]` or the macro attribute form `#[account(discriminator = 1)]`.

For Quasar bounded dynamic fields (`String<'a, N>`, `Vec<'a, Address, N>`), keep
the fields inside `#[hopper::account]`. Hopper keeps hot fixed fields in the
layout and lowers variable data into a compact dynamic tail. The macro generates
the tail struct, borrowed view, editor, extension trait, and allocation
constants. Use `#[hopper::dynamic_account]` with `#[tail(...)]` when you want the
systems-mode split visible in source. For custom dynamic-tail payloads, use
`hopper_dynamic_fields!` plus `#[hopper::state(dynamic_tail = Tail)]`. See
[DYNAMIC_TAILS_FROM_QUASAR.md](DYNAMIC_TAILS_FROM_QUASAR.md) for side-by-side
code and tail-access examples.

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
#[derive(Accounts)]
pub struct Swap<'info> {
    #[account(mut, seeds_fn = Vault::seeds(&nonce), bump)]
    pub vault: Account<'info, Vault>,
    #[account(mut)]
    pub authority: Signer<'info>,
    // remaining accounts exposed as ctx.remaining_accounts() on the
    // bound context, no field needed.
}
```

Key differences:

1. `seeds = Type::seeds(...)` becomes `seeds_fn = Type::seeds(...)`. The underscore disambiguates typed seeds from the inline array form (`seeds = [...]`), which Hopper also supports. For fixed byte-array seeds, pass an expression that borrows as bytes, for example `seeds = [hash.as_ref()]`; signer seeds also accept `Seed::from(&my_seed_array)` for `&[u8; N]`.
2. Remaining accounts are exposed on the bound context, not as a struct field. Call `ctx.remaining_accounts()` for strict mode or `ctx.remaining_accounts_passthrough()` for the duplicate-preserving mode. Use `ctx.remaining_accounts_raw()` only when a raw slice is actually what you want.
3. Hopper still exposes the raw account substrate for systems code, but first-touch ports should use `Account<'info, T>` and `Signer<'info>`.

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
pub fn swap(ctx: Ctx<Swap>, nonce: u64) -> ProgramResult {
    let mut vault = ctx.accounts.vault.get_mut()?;
    vault.balance.checked_add_assign(1)?;
    Ok(())
}
```

The discriminator syntax is identical. Hopper also accepts the short `#[instruction(0)]` form for single-byte discriminators when you do not need a multi-byte prefix.

Quasar's `ctx.accounts.vault.load_mut()?` becomes Hopper's `ctx.accounts.vault.get_mut()?` for the same full-struct zero-copy borrow. Hopper also emits segment-level accessors such as `ctx.vault_balance_mut()?` for systems-mode code that wants disjoint field borrows.

Both surfaces coexist on the bound context.

Hopper wire integers also support direct arithmetic operators and assignment
operators with native or wire RHS values. Safety-first code should keep using
`checked_add_assign`, `checked_sub_assign`, and `checked_mul_assign`; direct
operators follow normal Rust integer behavior, including wrapping in release
builds and overflow panics in debug builds.

## Interface accounts

For Quasar-style multi-owner protocols, declare the accepted program set once
and bind remote Hopper layouts through `InterfaceAccount<'info, T>`:

```rust
pub struct VaultPrograms;

impl InterfaceSpec for VaultPrograms {
    const IDS: &'static [Address] = &[VAULT_PROGRAM_A, VAULT_PROGRAM_B];
}

impl InterfaceAccountLayout for RemoteVault {
    type Interface = VaultPrograms;
}

#[derive(Accounts)]
pub struct ReadRemoteVault<'info> {
    pub vault_program: Interface<'info, VaultPrograms>,
    pub remote_vault: InterfaceAccount<'info, RemoteVault>,
}

impl<'info> ReadRemoteVault<'info> {
    pub fn read_balance(&self) -> Result<u64> {
        Ok(self.remote_vault.get()?.balance.get())
    }
}
```

`Interface<'info, I>` checks that the program account is executable and its key
is in `I::IDS`. `InterfaceAccount<'info, T>` checks that the account owner is
in `T::Interface::IDS`, then validates and loads `T` through the cross-program
Hopper layout loader.

For one account slot that can hold several Hopper layouts, generate a bounded
resolver and use `resolve()`:

```rust
hopper::interface_account_set! {
    pub struct AnyRemoteVault: VaultPrograms;
    pub enum RemoteVaultVersion {
        V1(RemoteVaultV1),
        V2(RemoteVaultV2),
    }
}

#[derive(Accounts)]
pub struct ReadAnyRemoteVault<'info> {
    pub remote_vault: InterfaceAccount<'info, AnyRemoteVault>,
}

match ctx.accounts.remote_vault.resolve()? {
    RemoteVaultVersion::V1(vault) => {
        let _balance = vault.balance.get();
    }
    RemoteVaultVersion::V2(vault) => {
        let _balance = vault.balance.get();
    }
}
```

The generated marker validates the owner set and accepts only the listed layout
fingerprints. `is::<T>()` and `get_as::<T>()` are available for targeted reads
within the same interface set.

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
#[hopper::error_code]
#[repr(u32)]
pub enum VaultError {
    #[invariant = "balance_nonzero"]
    InsufficientBalance = 0x1001,
}
```

Hopper exposes the error code explicitly. If you want Anchor-style auto-assignment starting at 6000, pick your own base and increment.
The source today emits code and invariant tables for Hopper errors; Anchor IDL export still writes an empty `errors` array, so do not document `#[msg]` strings as exported IDL metadata yet.

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
For the common multisig tail, `ctx.remaining_accounts().signers::<N>()?` validates the tail as a bounded, duplicate-safe signer set before you iterate it.

For perps-style variable tails, use the typed sequential parser:

```rust
let mut rem = ctx.remaining_typed();
let oracle = rem.next_external::<PythPrice>()?;
let market = rem.next_account()?;
rem.assert_no_duplicates()?;
rem.assert_sorted_by(|account| Ok(account.address().as_bytes()[0]))?;
```

`ExternalAccount<'info, T>` is for accounts with known non-Hopper bytes, such as oracle feeds, SPL/plugin state, governance add-ins, or another protocol's account layout. Hopper headers remain specific to Hopper-owned accounts; Hopper can still read, validate, route, and explain non-Hopper accounts.

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
8. Manifest, IDL, Codama, and client-generation tooling from the same layout metadata.

## Checklist for the port

1. Keep `#[derive(Accounts)]`.
2. Change `Ctx<'info, T>` to `Ctx<T>` in handler signatures.
3. Rename `Account<MyLayout>` to `Account<'info, MyLayout>` on context fields.
4. Replace `ctx.accounts.field.load()` and `.load_mut()` with `ctx.accounts.field.get()` and `.get_mut()`.
5. Rename `QuasarSerialize` to `hopper::args`. Swap `OptionZc` for `OptionByte` (or `Option` where you want the niche-optimized form).
6. Replace `seeds = Type::seeds(...)` with `seeds_fn = Type::seeds(...)`.
7. Move `RemainingAccounts` field references to `ctx.remaining_accounts()` on the bound context.
8. Rename `emit_event_cpi!` to `hopper_emit_cpi!` and thread the event-authority pubkey plus stored bump through.
9. Run `hopper build`. Fix whatever compiler errors surface. They will mostly be naming.
10. Read the `docs/TOKEN_2022_GUIDE.md` if your program touches Token-2022; Hopper's constraint surface is a clear superset of what you were using before.
