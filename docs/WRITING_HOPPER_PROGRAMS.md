# Writing Hopper Programs

Hopper's first-contact path is framework mode:

```rust
use hopper::prelude::*;
```

Start with accounts, contexts, and instructions. Layout fingerprints, schema
metadata, receipt hooks, and migration data are generated underneath the app
surface and become explicit only when the program opts into systems mode.

## Framework Shape

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[derive(Accounts)]
pub struct Increment<'info> {
    #[account(mut, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

#[program]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        let mut counter = ctx.accounts.counter.get_mut()?;
        counter.value.checked_add_assign(1)?;
        Ok(())
    }
}
```

That is the canonical Hopper application model:

- `#[account]` declares account state.
- `#[derive(Accounts)]` declares account roles and constraints.
- `#[program]` declares instruction handlers.
- `Ctx<T>` gives wrapper-backed `ctx.accounts.*` access in handlers.
- `require!`, `require_keys_eq!`, and `ProgramError` keep handler checks clear.

The compiled code still uses Hopper's zero-copy runtime. The author does not
need to name headers, fingerprints, segment maps, or manifests to write a normal
program.

## Account Access

Prefer wrapper-backed `ctx.accounts.*` access:

```rust
let mut counter = ctx.accounts.counter.get_mut()?;
counter.value.checked_add_assign(1)?;
```

For wrapper-shaped contexts, the framework surface also exposes:

```rust
Account<'info, T>
InitAccount<'info, T>
Signer<'info>
Program<'info, P>
UncheckedAccount
```

Keep handlers boring: validate authority, load typed state, mutate, return.

## Token And CPI Work

Everyday program modules are available without entering systems mode:

```rust
use hopper::{associated_token, cpi, system, token, token_2022};
```

Token-2022 programs should lean on Hopper's typed extension readers and CPI
builders from `hopper::token_2022` and the unified token helpers from
`hopper::token`.

## Bounded Dynamic Fields

When a fixed account needs a small bounded label or signer list, use
`#[hopper::account]` with bounded dynamic fields. The macro keeps fixed fields
in the zero-copy body and lowers dynamic fields into Hopper's compact
`[u32 len][payload]` tail.

```rust
use hopper::prelude::*;

#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
    pub weights: Vec<'a, u16, 10>,
}
```

For `#[hopper::account]`, this source-level `u64` field is lowered to a
wire-safe fixed-body field (`WireU64`) in the emitted layout. Use wire wrappers
directly in explicit overlay code paths.

`Multisig::new(threshold)` constructs the fixed body, `Multisig::ALLOC_SPACE`
is the maximum body-plus-tail allocation, `Multisig::label(data)` and
`Multisig::signers(data)` borrow compact-tail fields. Generic vectors such as
`weights(data)` return `HopperVec<T, N>`. Setters such as `set_label` /
`push_unique_signer` decode, edit, and write back the tail. Use explicit
`#[hopper::dynamic_account]` plus `#[tail(...)]` when a review should see the
tail split directly. Use `hopper_dynamic_fields!` plus
`#[hopper::state(dynamic_tail = T)]` when you want to name a custom `TailCodec`
payload directly. The generated `MultisigAccountTailExt` trait adds safe owned
getters and mutating helpers on `Account<'info, Multisig>` and
`InitAccount<'info, Multisig>` when the trait is in scope.

## Systems Mode

When the protocol needs layout evolution, field leases, receipts, policy graphs,
foreign account interfaces, or schema-driven clients, opt in explicitly:

```rust
use hopper::systems::*;
```

Systems mode contains:

- `hopper::layout` for headers, layout contracts, fingerprints, and wire maps.
- `hopper::segment` for segment registries and field-level borrow leases.
- `hopper::receipt` for state mutation receipts.
- `hopper::migration` for append-only schema evolution.
- `hopper::interface` for cross-program layout pinning.
- `hopper::schema` for manifests, IDL projection, and generated clients.
- `hopper::policy` for capability policies and protocol-grade guard rails.

The old `hopper_layout!` path remains useful for no-proc-macro builds and
systems examples, but it is no longer the first thing new users need to learn.

## Substrate Mode

When a program needs raw control, opt into the substrate layer explicitly:

```rust
use hopper::substrate::*;
```

This exposes Hopper Runtime types plus Hopper Native account views, raw input
parsing, syscalls, hash helpers, PDA helpers, memory helpers, compute-budget
probes, and verification primitives. It is the right layer for audited hot
paths and benchmark targets. Framework code should stay in the prelude until a
specific instruction needs substrate control.

## Example Order

Read examples in this order:

1. `examples/hopper-counter`
2. `examples/hopper-vault`
3. `examples/hopper-escrow`
4. `examples/hopper-token-2022-vault`
5. `examples/hopper-proc-vault`
6. `examples/hopper-showcase`
7. `examples/hopper-devnet-audit`

The first examples teach success first. The later examples expose why Hopper can
scale into protocol-grade state systems without changing frameworks.