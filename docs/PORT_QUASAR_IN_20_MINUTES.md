# Port Bounded Dynamic Account Fields in 20 Minutes

This guide shows how to move a fixed account body plus bounded dynamic fields
into Hopper without giving up the zero-copy hot path. The pattern is useful for
programs that have small labels, signer lists, memo payloads, or other bounded
metadata attached to an otherwise fixed account layout.

Hopper keeps the fixed body as an alignment-1 account overlay and places the
variable-sized data in one explicit dynamic tail:

```text
[ Hopper header ][ fixed account body ][ tail_len: u32 LE ][ encoded tail ]
```

Handlers that only touch fixed fields never decode the tail. Handlers that need
metadata use generated borrowed views or an owned editor/writeback helper,
making the dynamic path easy to audit.

## Porting Map

Use the Hopper framework wrappers for the first pass, then drop to systems/raw
access only when the program has a measured reason.

| Quasar concept | Hopper equivalent |
| --- | --- |
| `Account<T>` | `Account<'info, T>` |
| `Signer` | `Signer<'info>` |
| `Program<T>` | `Program<'info, T>` |
| `Interface<T>` | `Interface<'info, T>` |
| `InterfaceAccount<T>` | `InterfaceAccount<'info, T>` for Hopper-header layouts owned by a declared program set |
| `set_inner()` | generated `set_inner(...)` |
| `String<'a, N>` | `String<'a, N>` in `#[hopper::account]`, or `#[tail(string<N>)]` in systems-mode spelling |
| `Vec<'a, T, N>` | `Vec<'a, T, N>` in `#[hopper::account]`, or `#[tail(vec<T, N>)]` where `T: TailElement` |
| `ctx.bumps.foo` | `ctx.bumps.foo` |

## Fixed Vault

Start with the fixed state that should remain segment-borrowable:

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    #[role(authority)]
    pub authority: Address,

    #[role(balance)]
    pub balance: WireU64,

    #[role(bump)]
    pub bump: u8,
}
```

The fixed fields still receive the normal Hopper constants, layout ID,
validated loads, and generated segment accessors.

## Bounded Dynamic Account

Use `#[hopper::account]` when porting Quasar-style bounded fields. Hopper
detects bounded `String` and `Vec` fields and lowers them into the compact
dynamic tail while keeping fixed fields in the account body.

```rust
#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    #[role(threshold)]
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
    pub weights: Vec<'a, u16, 10>,
}
```

The macro generates a fixed `Multisig` body, a `MultisigTail`, borrowed
`MultisigTailView`, owned `MultisigTailEditor`, and `Multisig::ALLOC_SPACE`.
Native `u64` in the fixed body is stored as `WireU64` and exposed through the
generated `threshold()` getter. `string<N>` lowers to `HopperString<N>` and
`vec<T, N>` lowers to `HopperVec<T, N>` inside the generated tail. `Address` /
`Pubkey` vectors keep borrowed-slice views; other `TailElement` vectors return
`HopperVec<T, N>` values through generated view helpers.
The lifetime in the source declaration is authoring syntax only; contexts use
`Account<'info, Multisig>`.

When a review should see the split directly, use the systems-mode spelling:
`#[hopper::dynamic_account]` plus `#[tail(string<N>)]` or
`#[tail(vec<T, N>)]`. For custom named tail payloads, keep using the explicit
lower-level pair: `hopper_dynamic_fields!` plus
`#[hopper::state(dynamic_tail = Tail)]`.

## Read and Write the Tail

The generated dynamic account emits:

- `HAS_DYNAMIC_TAIL`
- `TAIL_PREFIX_OFFSET`
- `ALLOC_SPACE`
- `tail_len(data)`
- `tail_view(data)`
- `tail_editor(data)`
- `tail_read(data)`
- `tail_write(data, tail)`
- field helpers such as `label(data)`, `signers(data)`, `set_label(data, ...)`,
  `push_unique_signer(data, ...)`, and `remove_signer(data, ...)`

Example update handlers keep the Quasar-shaped `ctx.accounts.*` flow. The
program handler decodes a bounded Hopper string from instruction data, then the
accounts method works with `&str` and the generated dynamic-tail helpers:

```rust
#[derive(Accounts)]
pub struct RenameMultisig<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct AddSigner<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    pub authority: Signer<'info>,
}

impl<'info> RenameMultisig<'info> {
    pub fn rename(&self, label: &str) -> ProgramResult {
        self.multisig.set_label(label)
    }
}

impl<'info> AddSigner<'info> {
    pub fn add_signer(&self, signer: Address) -> ProgramResult {
        self.multisig.push_unique_signer(signer).map(|_| ())
    }
}

#[program]
mod multisig_program {
    use super::*;

    #[instruction(1)]
    pub fn rename(ctx: Ctx<RenameMultisig>, label: HopperString<32>) -> ProgramResult {
        ctx.accounts.rename(label.as_str()?)
    }

    #[instruction(2)]
    pub fn add_signer(ctx: Ctx<AddSigner>, signer: Address) -> ProgramResult {
        ctx.accounts.add_signer(signer)
    }
}
```

The raw account substrate is still available in systems-mode code, but a
Quasar migration should start with `Account<'info, T>` wrappers and only drop to
raw views for audited escape hatches.

Use this pattern when the dynamic fields are small and logically owned by the
same account. If the dynamic region needs independent borrow tracking,
independent migrations, or large append-only history, split it into an explicit
segment or companion account instead.

## Interface Accounts

For Quasar-style multi-owner protocols, declare the accepted program set once
and bind remote Hopper layouts with `InterfaceAccount<'info, T>`:

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

`Interface<'info, I>` verifies the executable program account is one of
`I::IDS`. `InterfaceAccount<'info, T>` verifies the account owner is in
`T::Interface::IDS` and then validates the Hopper layout header with the
cross-program loader.

When a single logical slot can contain one of several Hopper layouts, declare a
bounded resolver and call `resolve()` in the handler:

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

The generated marker validates owner membership first, then matches only the
listed layout fingerprints before returning a borrowed enum variant. Use
`is::<RemoteVaultV2>()` or `get_as::<RemoteVaultV2>()` when you only need a
targeted branch. Token and Token-2022 still use the specialized
`TokenProgramKind`, `InterfaceTokenAccount`, `InterfaceMint`, and
`interface_transfer_checked` helpers because SPL base layouts are not Hopper
header layouts.

## Full Example

See [`examples/quasar-port-20-min`](../examples/quasar-port-20-min/src/lib.rs)
for a workspace example containing the fixed vault, dynamic-account multisig,
initialization helper, signer-list mutation helpers, and threshold check. The
release checklist runs `cargo check -p hopper-quasar-port-20-min` and
`cargo test -p hopper-quasar-port-20-min` before the guide is treated as
compile-checked release material.