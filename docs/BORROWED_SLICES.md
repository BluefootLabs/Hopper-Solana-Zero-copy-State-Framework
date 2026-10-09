# Borrow bounded batches

`BoundedSlice<'a, T, N>` accepts a variable number of checked wire values
without allocating or copying them into a capacity-sized array. It is an
**unreleased** addition; use matching workspace crates.

Use it for instruction batches, bounded authority lists, or fixed-stride
records. The input carries a little-endian `u16` count followed by that many
consecutive `T` values. `N` limits the number of elements accepted. The parser
checks the count and available bytes, then validates every element before
returning a borrowed slice. Even an invalid last element refuses the batch.

```rust
use hopper::prelude::*;

#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Payment {
    pub recipient: Address,
    pub amount: WireU64,
}

#[hopper::program(entrypoint = false)]
mod payments {
    use super::*;

    #[instruction(0)]
    pub fn validate(_ctx: &mut Context<'_>, payments: BoundedSlice<'_, Payment, 16>) -> ProgramResult {
        for payment in payments.as_slice() {
            if payment.amount.get() == 0 {
                return Err(ProgramError::InvalidInstructionData);
            }
        }
        Ok(())
    }
}
```

This handler validates amounts. It does not authorize payments or transfer
funds. A payment implementation must bind the correct authority, validate
recipient accounts, and perform the appropriate System or token CPI.

## The wire contract

| Property | Behavior |
| --- | --- |
| Count | Two bytes, little-endian, at most `N` |
| Element layout | Nonzero-sized, alignment-1 `T: Pod` |
| Element validation | Recursive `Pod::validate_value` checks before admission |
| Storage | A shared slice into the original instruction buffer |
| Empty batch | Accepted for nonzero-sized element types; application may reject it |
| Following arguments | Start immediately after the last element |
| Extra bytes | Refused by generated dispatch unless an explicit final byte tail consumes them |
| Errors | Malformed lengths and representations produce `InvalidInstructionData` |

`BoundedSlice::parse_prefix` returns `(view, remaining_bytes)` for manual
entrypoints. `parse_exact` additionally requires an empty suffix. Both retain
the input lifetime. Native multi-byte integers cannot be overlaid at arbitrary
offsets; use `WireU64`, other alignment-1 wire primitives, or `#[hopper::pod]`
layouts. A custom `Pod` implementation must uphold its safety contract and
provide validation for any representation restrictions.

The view has no mutable access. A declared capacity does not reserve `N`
elements on the program's stack. Validation still visits each present element,
so compute use grows with the work performed; this is not an O(1) validation
claim or a universal CU advantage. The instruction must also fit Solana's
transaction and compute limits.

## Compatibility and clients

This is the same count prefix as Hopper's `BoundedVec`, but compatibility
requires that its element codec emits exactly the same fixed wire bytes as
`T`. It is not a reinterpretation of arbitrary Borsh vectors, Rust `Vec`
memory, or variable-stride codecs. Existing owned containers stay available.

Generated manifests retain the capacity and element width even when the
handler uses a type alias. Declared maximum wire length must fit the manifest's
`u16` size field; oversized declarations fail compilation instead of truncating.

TypeScript clients encode named `u8` elements as a `Uint8Array`; custom wire
elements and aliases use arrays of exact-width `Uint8Array` values. Generated
Rust clients represent sequence elements as `Vec<[u8; WIDTH]>`. Neither client
infers nested application encoders from opaque bytes. The program validates
their representations and application rules.

Rust instruction builders containing bounded arguments now return `Result`.
They check capacities before encoding; their decoders use checked offsets and
refuse trailing bytes, excessive counts, truncated elements, and malformed
UTF-8 strings. Regenerate these clients and handle the result when upgrading.
Their `*_DATA_LEN` constants are maximum lengths for dynamic instructions.
Fixed-only instruction builders retain their existing signatures.

Python builders accept a list of exact-width byte elements and encode bounded
strings as UTF-8, checking byte capacity. C, Go and Kotlin builders explicitly
refuse bounded encodings before producing instruction data. See the
[0.6 migration guide](MIGRATION_0_6.md) for the client support matrix and custom
owned element codec requirements.

## Verify the boundary

The [batch fixture](../bench/borrowed-slices/README.md) runs manual and generated
dispatch with no allocator. It verifies pointer identity on Solana, validates
all elements, and writes its test state only after the complete batch passes.
Compiled VM tests check malformed second elements, every tag byte, batch
limits, truncations, arithmetic overflow, and complete account preservation
on refusal. The client check compiles generated Rust and executes generated
TypeScript against independently assembled bytes.
