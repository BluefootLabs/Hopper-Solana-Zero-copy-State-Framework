# Hopper vs Quasar

Quasar has an excellent first-touch story: write Anchor-shaped Solana programs, cast account bytes directly, and keep the program small. Hopper keeps that authoring shape while adding a stronger contract layer.

> Write like Quasar. Hopper verifies the bytes before it casts them.

## The Difference

Quasar optimizes for direct account access. Hopper optimizes for direct account access after the program proves the bytes match the declared contract.

That contract includes:

- owner, signer, writable, PDA, and `has_one` validation through `#[derive(Accounts)]`;
- discriminator, version, and layout fingerprint checks before typed account access;
- schema-fingerprinted bounded dynamic fields;
- final-only raw tails through `TailStr<'a>` and `TailBytes<'a>`;
- optional segment-level borrows, receipts, policies, migrations, and interface pins when a protocol needs them.

## Same First-Touch Shape

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

#[program(profile = "tiny")]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        ctx.accounts
            .counter
            .with_mut(|counter| counter.value.checked_add_assign(1))
    }
}
```

`profile = "tiny"` keeps dispatch compact: one-byte discriminators and no handler-level modifier instrumentation.

## Dynamic Fields

Quasar-style bounded fields stay inline in source:

```rust
#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
}
```

Hopper lowers that to fixed body plus `[u32 len][compact tail payload]`. The dynamic tail schema is included in the layout fingerprint, so changing a capacity or element type is an ABI change that tools can detect.

For deliberate remaining-bytes semantics, Hopper uses named final tails:

```rust
#[hopper::account(discriminator = 21, version = 1)]
pub struct Note<'a> {
    pub authority: Address,
    pub label: String<'a, 32>,
    pub reviewers: Vec<'a, Address, 4>,
    pub body: TailStr<'a>,
}
```

`TailStr<'a>` and `TailBytes<'a>` must be final. They are fingerprinted as `tail_str` / `tail_bytes`, and the outer Hopper tail length still bounds the account region.

## Where Hopper Adds More

- Segment leases let systems-mode code borrow disjoint byte ranges instead of whole accounts.
- Token-2022 TLV constraints validate extension state without deserializing into owned structs.
- `hopper solana-check`, `publish-check`, and the SBF workflow keep deployable crate shape and direct-runtime assumptions honest.
- Actions, mobile, and security-test generators have a manifest-backed foundation for product scaffolding.

Use Quasar mental models to read Hopper programs. Use Hopper contracts when account bytes, upgrades, and long-lived protocol state need to be auditable.