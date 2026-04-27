# hopper-macros-proc

Optional proc macro DX layer for [Hopper](https://hopperzero.dev). Generates
the parsing, validation, and dispatch code for the `#[hopper::state]`,
`#[hopper::context]`, `#[hopper::program]` authoring path.

## Not required

Every feature these macros provide is achievable through Hopper's
declarative `macro_rules!` macros (in [`hopper-macros`](../hopper-macros)) or
hand-written code. They exist purely for developer velocity. Generated code
compiles to the same pointer arithmetic as raw Pinocchio.

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
| `#[hopper::dynamic]` | Dynamic-tail field metadata for ring-buffer bookkeeping |
| `hopper::declare_program!` | IDL-driven CPI surface with compile-time `FINGERPRINT` const |
| `#[derive(HopperInitSpace)]` | Anchor-parity `INIT_SPACE` derive for hand-authored Pod structs |

## Enable

```toml
[dependencies]
hopper = { version = "0.1", features = ["proc-macros"] }
```

License: Apache-2.0.
