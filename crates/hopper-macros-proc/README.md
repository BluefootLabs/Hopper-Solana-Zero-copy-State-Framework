# hopper-derive

Optional proc macro DX layer for [Hopper](https://hopperzero.dev). It generates
the parsing, validation, and dispatch code for the `#[hopper::state]`,
`#[derive(Accounts)]`, and `#[hopper::program]` authoring path. The older
`#[hopper::context]` spelling remains available for lower-level migrations.

## Not required

Every feature these macros provide is achievable through Hopper's declarative
`macro_rules!` macros in [`hopper-macros`](../hopper-macros) or hand-written
code. They exist for developer velocity. Generated code still lowers to
Hopper's typed pointer and validation surface.

## What's emitted

| Macro | Purpose |
|---|---|
| `#[hopper::state]` | Zero-copy account layout with header + fingerprint + load/load_mut helpers |
| `#[hopper::account]` | Framework account layout; auto-upgrades bounded dynamic `String<'a, N>` / `Vec<'a, T, N>` fields into compact tails |
| `#[derive(Accounts)]` | First-touch account-context binding with the full Anchor keyword set and Hopper account wrappers |
| `#[hopper::context]` (aliases `#[context]`, `#[accounts]`) | Attribute-form account-context binding for lower-level migrations and segment-level borrow vocabulary |
| `#[hopper::program]` (alias `#[program]`) | Entrypoint bridge plus instruction dispatcher; supports `#[receipt]`, `#[invariant]`, `#[pipeline]`, `#[access_control]` handler attributes |
| `#[hopper::migrate]` | Schema-epoch migration edges |
| `#[hopper::event]` | Event types with discriminator + segment lineage |
| `#[hopper::error]` | Error enums with `code()` / `invariant_idx()` + `CODE_TABLE` / `INVARIANT_TABLE` |
| `#[hopper::constant]` | Anchor-compatible constants surfaced for IDL generation |
| `#[hopper::args]` | Borrowing zero-copy instruction-arg parser with optional CU hint |
| `#[hopper::pod]` (alias `#[pod]`) | Pod marker derive with align-1 / no-padding compile-time assertions |
| `#[hopper::crank]` | Keeper-bot autonomous-marker descriptor |
| `#[hopper::dynamic_account]` | Explicit systems-mode bounded `#[tail(...)]` fields lowered into fixed body + compact dynamic tail |
| `#[hopper::dynamic]` | Dynamic-tail field metadata for ring-buffer bookkeeping |
| `hopper::declare_program!` | Manifest-driven CPI surface with compile-time `FINGERPRINT`, borrowed Hopper instruction parts, and resolver/effect specs |
| `#[derive(HopperInitSpace)]` | Anchor-parity `INIT_SPACE` derive for hand-authored Pod structs |

## `#[hopper::state]` Copy contract

State structs are wire overlays and must be `Clone + Copy`. Write the derive
explicitly next to the layout:

```rust
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    pub balance: hopper::prelude::WireU64,
}
```

The macro verifies this contract instead of injecting its own derive, so the
README pattern above works without duplicate trait implementations.

## `#[hopper::account]` Dynamic Fields

The framework account macro accepts Quasar-pretty bounded fields and lowers
them into Hopper's fixed-body + compact-tail layout:

```rust
#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
    pub weights: Vec<'a, u16, 10>,
}
```

The lifetime is authoring syntax for the macro. The emitted account type is a
concrete fixed-body layout with generated tail helpers.

## `#[hopper::dynamic_account]`

`dynamic_account` is the explicit systems-mode spelling. It accepts normal
fixed fields plus bounded tail fields:

```rust
#[hopper::dynamic_account(disc = 7, version = 1)]
pub struct Multisig {
    pub threshold: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,

    #[tail(vec<u16, 10>)]
    pub weights: Vec<u16>,
}
```

It emits a fixed-body `Multisig`, generated `MultisigTail`, view/editor helpers,
`ALLOC_SPACE`, and compact-tail helpers. `Address` / `Pubkey` vectors use
borrowed-slice views; other `T: TailElement` vectors return `HopperVec<T, N>`.
The initial supported tail policy is `compact`; use explicit
`hopper_dynamic_fields!` with `#[hopper::state(dynamic_tail = T)]` when you want
to name a custom `TailCodec` payload directly.

For deliberate remaining-bytes semantics, use an explicit final raw tail:

```rust
#[hopper::account(discriminator = 21, version = 1)]
pub struct Note<'a> {
    pub authority: Address,
    pub label: String<'a, 32>,
    pub body: TailStr<'a>,
}
```

`TailStr<'a>` and `TailBytes<'a>` must be final fields. They consume the
remaining Hopper dynamic-tail payload without an inner field prefix and enter
the layout fingerprint as `tail_str` or `tail_bytes`.

## Enable

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.2.1", features = ["proc-macros"] }
```

Docs: <https://docs.rs/crate/hopper-derive/0.2.1>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
