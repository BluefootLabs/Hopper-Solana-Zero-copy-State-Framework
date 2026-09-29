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

Two mutable roles in one context (`from` and `to`, an offer and its vault)
are two different accounts by contract: `bind` refuses a transaction that
passes one account for both with `ERR_ALIASED_MUTABLE_ACCOUNTS`
(`Custom(0xB002)`), one record-pointer compare per pair, before any other
check runs. When one account legitimately plays two roles, declare it with
`dup = other_field` on the second field; that pair is then required to
alias and is left out of the check. Optional slots never take part.

## Enums And Optional Values In Layouts

A Rust enum is not a zero-copy type: a three-variant `#[repr(u8)]` enum has
253 byte values that are not a value of the type, so overlaying it on
account bytes is undefined behaviour as soon as an account holds one of
them. Declare the enum with `#[hopper::unit_enum]` and store it as
`EnumByte<E>`: one byte, alignment 1, and the enum comes back through
`get()`, which refuses a byte that names no variant.

```rust
#[hopper::unit_enum]
pub enum Status {
    Open = 1,
    Settled = 2,
    Cancelled = 3,
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 5, version = 1)]
pub struct Order {
    pub maker: Address,
    pub status: EnumByte<Status>,
    pub referrer: OptionByte<[u8; 32]>,
    pub amount: WireU64,
}

if order.status.get()? == Status::Open {
    order.status.set(Status::Settled);
}
```

`#[hopper::unit_enum]` forces `#[repr(u8)]`, adds `Clone, Copy, PartialEq,
Eq, Debug` when the enum declares no derive of its own, and generates the
byte mapping from the variants, so it cannot drift from the declaration.
`OptionByte<T>` is the optional counterpart (`tag` then `T`; a tag other
than 0 or 1 is an error). Both work in `#[hopper::args]` too, where
`parse_checked` refuses an unknown variant or tag with
`InvalidInstructionData` before the handler runs. A field type that has no
zero-copy form is a compile error that names the wire type to use instead.

## Value Rules And Native Accessors

Zero-copy means the bytes in the account are the state. Nothing decodes
them, so nothing gets a chance to say "a tier of 200 is not a tier". Put
the rule on the field and Hopper checks it for you.

```rust
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 7, version = 1, accessors)]
pub struct Pool {
    pub authority: Address,
    #[check(value >= 1 && value <= 10)]
    pub tier: u8,
    #[check(value <= 1_000, error = PoolError::FeeTooHigh)]
    pub fee_bps: WireU16,
    #[check(value <= self.cap.get())]
    pub deposited: WireU64,
    pub cap: WireU64,
}
```

`value` is the field's native value (`u16` for a `WireU16`), and the rule
may read other fields through `self`. Without `error = ..` a failed rule is
`InvalidAccountData`. What you get:

- `pool.check_rules()` runs every rule in field order and returns the first
  failure. Call it after you fill a new account.
- `pool.try_set_tier(11)` checks the new value first and leaves the field
  alone when the rule refuses it.
- `#[derive(Accounts)]` checks the stored values of every existing account
  it binds, before your handler runs. Accounts being created or migrated
  are skipped, and `#[account(skip_rules)]` lets a repair instruction bind
  state that is already broken. A layout with no rules compiles to exactly
  what it did before.
- `Pool::FIELD_RULES` publishes each rule as text plus the integer bounds
  it decides (`tier`: 1 to 10). `FieldRule::change_to` compares two
  releases and tells you whether a rule was tightened or widened. A widened
  bound means stored values your old program could never produce are now
  accepted, which is the kind of change you want to see in review.

A rule that no value can satisfy is a compile error.

`accessors` is separate and opt-in: it generates `pool.fee_bps()` and
`pool.set_cap(20_000)` for the wire scalar fields so handlers speak native
values. A field with a rule gets the getter and `try_set_<field>`, not an
unchecked setter. The fields stay public; a direct write skips the rule,
so write ruled fields through `try_set_`.

Seeds get the same treatment at compile time. `seeds = [..]` refuses a
literal seed longer than 32 bytes and a list with no room left for the
bump, because both fail every derivation at run time:

```text
error: this seed is 33 bytes and a seed takes at most 32; every derivation
       with it fails with `MaxSeedLengthExceeded`. Shorten the literal, or
       hash it and pass the 32-byte digest.
```

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
`push_unique_signer` edit that one field in place and move only the bytes
behind it. Use explicit
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