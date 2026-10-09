# Borrowed instruction arguments

The current source tree connects borrowed wire layouts to generated program
handlers, recursive value validation, and explicit payload boundaries. These additions are **unreleased**;
use matching workspace crates until the next coordinated publication.

`#[hopper::args]` overlays fixed-size instruction bytes with an alignment-1 Rust
struct. Checked parsers validate the fields while keeping the result borrowed
from the caller's buffer. They do not allocate or copy the whole argument struct.

## Compose a wire type once

```rust
use hopper::prelude::*;

#[hopper::unit_enum]
pub enum Side {
    Bid = 1,
    Ask = 2,
}

type OptionalSide = OptionByte<EnumByte<Side>>;

#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Quote {
    pub amount: WireU64,
    pub side: OptionalSide,
}

#[hopper::args]
#[repr(C)]
pub struct QuoteArgs {
    pub quotes: [Quote; 2],
}

pub fn validate_quotes(data: &[u8]) -> ProgramResult {
    let args = QuoteArgs::parse_exact_checked(data)?;
    for quote in &args.quotes {
        let _side = quote.side.get()?;
        let _amount = quote.amount.get();
        // Apply the application's price, amount, and authorization rules here.
    }
    Ok(())
}
```

An alias preserves its underlying type's validation. Each array element and
nested `#[hopper::pod]` field is checked in declaration order. An `OptionByte`
tag must be 0 or 1; a present payload is checked recursively. An `EnumByte` must
name a declared variant. The unused payload of an absent option is ignored.

For a manual entrypoint, pass the payload after your instruction discriminator
to the checked parser. Generated handlers can accept the same declaration directly.

## Borrow the layout in a handler

The program dispatcher now understands a reference to a declared argument
layout. This validation-only handler uses the `QuoteArgs` declaration above:

```rust
#[hopper::program(entrypoint = false)]
mod quotes {
    use super::*;

    #[instruction(0)]
    pub fn validate(_ctx: &mut Context<'_>, args: &QuoteArgs) -> ProgramResult {
        for quote in &args.quotes {
            if quote.amount.get() == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        Ok(())
    }
}
```

`&QuoteArgs` consumes exactly its fixed footprint and validates nested values
before the handler runs. It also works with `Ctx<MyAccounts>`: decoding precedes
account binding, then the context's checks and handler's business rules run.
The reference points into the original instruction buffer; the argument struct
needs neither `Clone` nor `Copy`. Scalar parameters continue to decode by value.

Several borrowed layouts and scalars can follow one another in wire order.
An explicit final `tail: &[u8]` consumes the remaining bytes, which the handler
must bound and interpret. Without that tail, the dispatcher refuses extra
bytes. `#[args(tail)]` controls the manual tail helpers; a generated handler's
parameter list determines its own payload shape.

Typed-handler manifests record the borrowed layout's fixed wire size, including
through a layout alias. Generated clients treat that argument as opaque fixed
bytes rather than synthesizing a nested client struct: Rust uses `[u8; N]`, and
TypeScript uses a `Uint8Array` with an exact-length check. A byte tail needs an
application encoder; it has no fixed width or automatically inferred schema.
Account authorization remains separate. This example validates inputs and
does not create accounts, move assets, or establish a custody policy.

The macro supplies `DecodeInstructionArg` for `&MyArgs`. Remove a hand-written
implementation for that same reference type when adopting this source version
to avoid conflicting implementations.

## Choose the payload boundary

For a variable number of fixed-stride wire values, use
[`BoundedSlice<'_, T, N>`](BORROWED_SLICES.md). It validates a length-prefixed
batch and borrows its elements; unlike a bare byte tail, it has a declared
element width and capacity. Later arguments can follow the batch.

| Method | Length policy | Value validation |
|---|---|---|
| `parse_exact_checked` | Exactly `PACKED_SIZE` | Recursive |
| `parse_checked` | At least `PACKED_SIZE`; trailing bytes allowed | Recursive |
| `parse_with_tail_checked` | Fixed prefix plus a returned borrowed suffix | Recursive on the prefix |
| `parse` | At least `PACKED_SIZE` | Raw overlay only |
| `parse_with_tail` | Fixed prefix plus a returned borrowed suffix | Raw overlay only |

The two tail methods require `#[hopper::args(tail)]`. For example:

```rust
use hopper::prelude::*;

#[hopper::args(tail)]
#[repr(C)]
pub struct MemoArgs {
    pub recipient: Address,
    pub priority: OptionByte<u8>,
}

pub fn validate_memo(data: &[u8]) -> ProgramResult {
    let (_args, memo) = MemoArgs::parse_with_tail_checked(data)?;
    core::str::from_utf8(memo).map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok(())
}
```

The tail stays a byte slice. Its format, length bound, and application rules are
the handler's responsibility. Checked parsers return `InvalidInstructionData`
for a short payload, invalid nested representation, or an exact-length mismatch.
An already borrowed argument struct can call `validate_values()`;
`validate_tags()` remains an alias for the same checks.

## Validation and memory safety are separate

`Pod::validate_value(&value)` is the shared representation check. Arrays,
`OptionByte`, `EnumByte`, `#[hopper::pod]`, `#[hopper::state]` (including compact
layouts), `hopper_layout!`, and `hopper_interface!` compose it. It leaves bytes,
layout sizes, and fingerprints unchanged.

A `Pod` type must still be a valid Rust value for **every** byte pattern, even
when representation validation returns an error. Raw overlays and account
loaders retain their existing behavior; they do not automatically invoke this
new hook. Validation does not establish account ownership, a layout header,
authorization, or application `#[check]` rules. Check those at their respective
boundaries.

Existing hand-written `Pod` implementations inherit a validator that accepts
every value. If a custom type has protocol restrictions, implement
`validate_value` in its `Pod` implementation, or use a layout macro to delegate
to its fields. Hopper's boolean wire types continue to interpret every nonzero
byte as true; this change does not impose a new canonical boolean encoding.

Previously accepted malformed nested option or enum values now fail checked
parsing. Valid encodings and the existing prefix behavior of `parse_checked`
remain unchanged. Unknown, malformed, or duplicate `#[hopper::args]` options
also produce a compile-time error instead of being ignored.

## Exercise the boundary on Solana

The [borrowed-argument fixture](../bench/borrowed-args/README.md) validates an
unaligned instruction prefix, checks that both references still point into the
input, and updates a test account only after all checks pass. Its compiled VM
test covers every byte in four option/enum positions and checks whole-account
preservation on failure. The devnet runner also checks the deployed binary and
return-data producer. Use these tests as a pattern for your own application's
authorization, tail semantics, and state-transition checks.
Its manual and generated modes use the same state transition and rejection
tests, so parsing ergonomics can be compared without removing validation from
one path. Fixture measurements do not establish a universal CU advantage.
