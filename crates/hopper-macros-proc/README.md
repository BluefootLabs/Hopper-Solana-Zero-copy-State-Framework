# hopper-macros-proc

Optional proc macro DX layer for [Hopper](https://hopperzero.dev). It generates
the parsing, validation, and dispatch code for the `#[hopper::state]`,
`#[hopper::context]`, and `#[hopper::program]` authoring path.

## Not required

Every feature these macros provide is achievable through Hopper's declarative
`macro_rules!` macros in [`hopper-macros`](../hopper-macros) or hand-written
code. They exist for developer velocity. Generated code still lowers to
Hopper's typed pointer and validation surface.

## What's emitted

| Macro | Purpose |
|---|---|
| `#[hopper::state]` (alias `#[account]`) | Zero-copy account layout with header + fingerprint + load/load_mut helpers |
| `#[hopper::context]` (aliases `#[context]`, `#[accounts]`) | Typed account-context binding with the full Anchor keyword set + Hopper-unique segment-level borrow vocabulary |
| `#[hopper::program]` (alias `#[program]`) | Instruction dispatcher, supports `#[receipt]`, `#[invariant]`, `#[pipeline]`, `#[access_control]` handler attributes |
| `#[hopper::migrate]` | Schema-epoch migration edges |
| `#[hopper::event]` | Event types with discriminator + segment lineage |
| `#[hopper::error]` | Error enums with `code()` / `invariant_idx()` + `CODE_TABLE` / `INVARIANT_TABLE` |
| `#[hopper::args]` | Borrowing zero-copy instruction-arg parser with optional CU hint |
| `#[hopper::pod]` (alias `#[pod]`) | Pod marker derive with align-1 / no-padding compile-time assertions |
| `#[hopper::crank]` | Keeper-bot autonomous-marker descriptor |
| `#[hopper::dynamic_account]` | Quasar-style bounded `String` / `Vec<T>` fields lowered into fixed body + compact dynamic tail |
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

## `#[hopper::dynamic_account]`

`dynamic_account` is the Quasar-porting façade. It accepts normal fixed fields
plus bounded tail fields:

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

## Enable

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.1.0", features = ["proc-macros"] }
```

Docs: <https://docs.rs/crate/hopper-derive/0.1.0>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
