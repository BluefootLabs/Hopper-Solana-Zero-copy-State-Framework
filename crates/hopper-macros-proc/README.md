# hopper-derive

In 0.3.2, `#[account(cells(slot; spent, revisions))]` generates
`book_spent_cell_mut()` and `book_spent_cell_ref()` on the bound context for an
account named `book`. The accessor captures the instruction selector, infers
the array element type, and refuses an out-of-range index. Existing byte
policy and borrow checks still apply. Explicit
`#[accounts(strict_writes, lamports())]` now works through `derive(Accounts)`
as well as the direct context attribute. See the repository's on-chain quota
example and byte-policy guide.

[![Crates.io](https://img.shields.io/crates/v/hopper-derive.svg)](https://crates.io/crates/hopper-derive)
[![Docs.rs](https://img.shields.io/docsrs/hopper-derive)](https://docs.rs/hopper-derive)

Optional proc-macro DX layer, published as `hopper-derive`, for
[Hopper](https://hopperzero.dev). Its source lives in the
`crates/hopper-macros-proc` workspace directory. It generates parsing,
validation, and dispatch code for the `#[hopper::state]`,
`#[derive(Accounts)]`, and `#[hopper::program]` authoring path.

The proc-macro layer is optional. Programs can also use Hopper's declarative
macros and runtime APIs directly. Generated code lowers to Hopper's typed
layout, validation, and dispatch surfaces.

## What's emitted

| Macro | Purpose |
|---|---|
| `#[hopper::state]` | Zero-copy account layout with header + fingerprint + load/load_mut helpers |
| `#[hopper::account]` | Framework account layout; auto-upgrades bounded dynamic `String<'a, N>` / `Vec<'a, T, N>` fields into compact tails |
| `#[derive(Accounts)]` | Account-context binding with Hopper's documented constraints and wrappers |
| `#[hopper::context]` (aliases `#[context]`, `#[accounts]`) | Attribute-form account-context binding for lower-level migrations and segment-level borrow vocabulary |
| `#[hopper::program]` (alias `#[program]`) | Entrypoint bridge plus instruction dispatcher; supports `#[receipt]`, `#[invariant]`, `#[pipeline]`, `#[access_control]` handler attributes |
| `#[hopper::migrate]` | Schema-epoch migration edges |
| `#[hopper::event]` | Event types with discriminator + segment lineage |
| `#[hopper::error_code]` | Error enums with `code()` / `invariant_idx()` + `CODE_TABLE` / `INVARIANT_TABLE`, `From<E> for ProgramError` |
| `#[hopper::constant]` | Anchor-compatible constants surfaced for IDL generation |
| `#[hopper::args]` | Borrowing zero-copy instruction-arg parser with optional CU hint |
| `#[hopper::pod]` (alias `#[pod]`) | Pod marker derive with align-1 / no-padding compile-time assertions |
| `#[hopper::crank]` | Keeper-bot autonomous-marker descriptor |
| `#[hopper::dynamic_account]` | Explicit systems-mode bounded `#[tail(...)]` fields lowered into fixed body + compact dynamic tail |
| `#[hopper::dynamic]` | Dynamic-tail field metadata for ring-buffer bookkeeping |
| `hopper::declare_program!` | Manifest-driven CPI surface with compile-time `FINGERPRINT`, borrowed Hopper instruction parts, and resolver/effect specs |
| `hopper::canonical_pda!` | Canonical address and bump from an explicit program-ID string and literal byte-string seeds, computed on the build host |
| `#[derive(HopperInitSpace)]` | `INIT_SPACE` derive for hand-authored Pod structs |

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

The framework account macro accepts bounded dynamic fields and lowers
them into Hopper's fixed-body + compact-tail layout:

```rust
#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    pub threshold: hopper::prelude::WireU64,
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
    pub threshold: hopper::prelude::WireU64,

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

## Canonical PDAs

```rust
const CONFIG: (hopper::prelude::Address, u8) = hopper::canonical_pda!(
    "F4Um7PWsnZfN7y8WFzu1aPYJwqGduJTa4zuCGY9EUqMy",
    [b"config", b"v1"]
);
```

The macro searches bumps from 255 down and checks the curve at build time.
It emits address bytes and a bump, with no on-chain derivation. Inputs are
explicit literals: at most 15 base seeds, each at most 32 bytes. Changing
the program ID requires rebuilding the constant. Account ownership, layout,
signer and writable requirements remain separate account constraints.

For dynamic seeds, bare `bump` and `seeds_fn` require the canonical address,
including for typed accounts. Direct required fields retain their validated bumps during binding: bare
`bump`, supplied and stored bumps, and `seeds_fn` helpers. Optional fields
and nested-context gathering still derive separately.
`bump = stored` and explicitly supplied bumps verify the selected address;
they do not prove that the bump is canonical. Establish canonicality during
initialization when the application requires a unique address per seed set.

## Enable the proc macros

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.3.2", features = ["proc-macros"] }
```

Docs: <https://docs.rs/crate/hopper-derive>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.

New in 0.3.1: direct required bindings also retain stored,
supplied, and seed-helper bumps, avoiding a second expression evaluation or
search. Extension constraints establish Token-2022 ownership before inspecting
TLV bytes. Optional/composite bump gathering remains a separate path. The
`#[bump]` marker names a stored byte; initialization must write the validated
bump explicitly and later handlers must preserve the intended invariant.
