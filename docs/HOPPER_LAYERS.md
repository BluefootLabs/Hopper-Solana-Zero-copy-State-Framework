# Hopper Layers

Hopper is one framework with progressive disclosure. The framework path, the
Quasar-shaped migration path, and the systems-mode path all use the same account
headers, layout fingerprints, runtime checks, and CPI machinery.

## Level 1: Framework Mode

Use this for new programs, tutorials, and ports that should feel like a normal
Solana framework before they expose the deeper machinery.

```rust
use hopper::prelude::*;

#[account(disc = 1, version = 1)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vault {
    pub authority: Address,
    pub balance: WireU64,
    pub bump: u8,
}
```

Framework mode centers these pieces:

- `#[account]`, `#[accounts]`, `#[program]`, and `#[derive(Accounts)]`.
- `Account<'info, T>`, `InitAccount<'info, T>`, `Signer<'info>`, and
  `Program<'info, P>`.
- `hopper::account`, `hopper::context`, `hopper::cpi`, `hopper::system`,
  `hopper::token`, `hopper::token_2022`, `hopper::associated_token`, and
  `hopper::memo`.
- `hopper_dynamic_fields!` for bounded `string<N>` and `vec<T, N>` fields when
  porting Quasar-shaped programs.

This path is published as `hopper-lang` and imported as `hopper`:

```toml
hopper = { package = "hopper-lang", version = "0.1.0", features = ["proc-macros"] }
```

There is no separate beginner crate. The main framework crate is the canonical
facade, and it grows into the same systems layer when needed.

## Level 2: Structured State

Use this when account compatibility, schema publication, dynamic tails, or
client generation matters.

- `hopper::layout` exposes headers, layout contracts, fingerprints, field maps,
  and wire types.
- `hopper::schema` exposes manifests, IDL projection, resolver metadata, and
  generated client inputs.
- `hopper_dynamic_tail!` and `hopper_dynamic_fields!` attach bounded dynamic
  payloads to fixed zero-copy layouts.
- `declare_program!` consumes a manifest and generates typed CPI builders plus
  static account/effect specs.

This is the right layer for serious applications that still want one-account
state contracts and predictable upgrade paths.

## Level 3: Systems Mode

Use this when the program needs field leases, segmented layouts, policy receipts,
explicit migrations, or foreign layout contracts.

- `hopper::systems::*` is the branded one-import path for protocol-grade work.
- `hopper::segment` exposes segment registries, segment roles, field-level borrow
  tracking, and typed segment references.
- `hopper::receipt` exposes execution receipts and tagged event receipts.
- `hopper::policy` exposes capability policies and policy packs.
- `hopper::migration` exposes layout migration edges and pending migration
  application.
- `hopper::interface` exposes foreign manifest and interface helpers for
  cross-program reads.
- `hopper::internal` is an explicit lower-crate escape hatch for authors who are
  deliberately working below the framework facade.

The systems layer publishes as `hopper-systems`. The source still lives in
`crates/hopper-core` for now to preserve history and keep this rename focused;
the package name is the public ecosystem name.

## Mental Mapping

| Concept | Anchor-shaped mental model | Quasar-shaped mental model | Hopper path |
|---|---|---|---|
| Program state | `#[account]` struct | fixed account struct | `#[hopper::account]` or `#[hopper::state]` |
| Context/accounts | `#[derive(Accounts)]` | account list plus checks | `#[derive(Accounts)]`, `#[hopper::accounts]`, typed wrappers |
| Signer | `Signer<'info>` | signer account check | `hopper::account::Signer<'info>` |
| Typed account | `Account<'info, T>` | fixed account view | `hopper::account::Account<'info, T>` |
| Dynamic string/vector | `String`, `Vec<T>` with serialization | bounded dynamic fields | `hopper_dynamic_fields! { name: string<N>, items: vec<T, N> }` |
| CPI | generated CPI clients | manual/generated CPI | `declare_program!`, `hopper::cpi`, SPL facade modules |
| Upgrade/migration | discriminator/version conventions | layout evolution discipline | layout fingerprints, `hopper::migration`, schema manifests |
| Advanced safety | constraints and runtime checks | zero-copy constraints | segment leases, receipts, policy graphs, Kani-checked invariants |

## Import Guide

Prefer the smallest import surface that matches the job:

```rust
use hopper::prelude::*;              // main app framework surface
use hopper::systems::*;              // protocol-grade state architecture
use hopper::{layout, segment};       // explicit systems modules
use hopper::{schema, cpi, token};     // tooling, CPI, and token facades
```

The public modules are additive. New users start with the main prelude and only
open the systems modules when the design calls for them.
