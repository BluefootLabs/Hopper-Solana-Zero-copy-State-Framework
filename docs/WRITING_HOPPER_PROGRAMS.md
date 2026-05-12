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
#[account(disc = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[accounts]
pub struct Increment {
    #[account(mut)]
    pub counter: Counter,

    #[signer]
    pub authority: AccountView,
}

#[program]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Context<Increment>) -> ProgramResult {
        let authority = *ctx.authority_account()?.address();
        let mut counter = ctx.counter_load_mut()?;

        require_keys_eq!(counter.authority, authority, ProgramError::IncorrectAuthority);

        let next = counter
            .value
            .get()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        counter.value = WireU64::new(next);
        Ok(())
    }
}
```

That is the canonical Hopper application model:

- `#[account]` declares account state.
- `#[accounts]` or `#[derive(Accounts)]` declares account roles and constraints.
- `#[program]` declares instruction handlers.
- `Context<T>` gives typed accessors generated from the context.
- `require!`, `require_keys_eq!`, and `ProgramError` keep handler checks clear.

The compiled code still uses Hopper's zero-copy runtime. The author does not
need to name headers, fingerprints, segment maps, or manifests to write a normal
program.

## Account Access

Prefer typed accessors generated from the context:

```rust
let mut counter = ctx.counter_load_mut()?;
counter.value = WireU64::new(counter.value.get() + 1);
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

## Example Order

Read examples in this order:

1. `examples/hopper-counter`
2. `examples/hopper-vault`
3. `examples/hopper-escrow`
4. `examples/hopper-token-2022-vault`
5. `examples/hopper-proc-vault`
6. `examples/hopper-showcase`

The first examples teach success first. The later examples expose why Hopper can
scale into protocol-grade state systems without changing frameworks.