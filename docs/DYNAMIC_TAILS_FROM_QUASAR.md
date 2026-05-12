# Dynamic tails from Quasar dynamic fields

Quasar lets a zero-copy account include bounded dynamic fields such as
`String<'a, 32>` or `Vec<'a, Address, 10>` inside the account declaration.
Hopper takes a different route: keep the fixed body strictly zero-copy, then
attach one explicitly encoded dynamic tail after the fixed body.

That split is intentional:

- Fixed fields stay alignment-1, offset-stable, and segment-borrowable.
- Code that never reads the tail pays zero dynamic overhead.
- Tail reads and writes are explicit, so reviewers can grep them.
- Larger repeated regions can graduate to named extension segments when they
  need independent borrow tracking or migration metadata.

## Wire format

For `#[hopper::state(dynamic_tail = T)]`, the bytes after the fixed body are:

```text
[ fixed Hopper body ][ tail_len: u32 LE ][ tail_payload: tail_len bytes ]
```

The generated layout uses `Self::LEN` as `TAIL_PREFIX_OFFSET`, so the payload
starts at `Self::TAIL_PREFIX_OFFSET + 4`.

## Quasar field to Hopper tail

Quasar-style shape:

```rust
// Quasar-style sketch
#[account(discriminator = 7)]
#[repr(C)]
pub struct Multisig {
    pub threshold: PodU64,
    pub label: String<'static, 32>,
    pub signers: Vec<'static, Address, 10>,
}
```

Hopper shape:

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[hopper::state(disc = 7, dynamic_tail = MultisigTail)]
#[repr(C)]
pub struct Multisig {
    pub threshold: WireU64,
}

hopper_dynamic_fields! {
    pub struct MultisigTail {
        label: string<32>,
        signers: vec<Address, 10>,
    }
}
```

`threshold` remains a zero-copy field. `label` and `signers` move into the
single tail payload and are decoded only when a handler asks for them.

## Bounded helper types

`TailCodec` is a minimal Borsh-subset trait. Hopper implements it for integers,
`bool`, `[u8; N]`, `Option<T>`, `Address`, `BoundedString<N>`, and
`BoundedVec<T, N>`. `hopper_dynamic_fields!` lowers the common Quasar-shaped
`string<N>` and `vec<T, N>` spellings into the `HopperString<N>` and
`HopperVec<T, N>` aliases, keeping ported layouts concise while preserving
explicit bounded storage.

```rust
use hopper::prelude::*;

hopper_dynamic_fields! {
    pub struct MultisigTail {
        label: string<32>,
        signers: vec<Address, 10>,
    }
}
```

`HopperVec<T, N>` also includes small set-like helpers for signer-list style
tails: `contains`, `push_unique`, `remove_first`, `pop`, `clear`, and capacity
inspection.

## Generated helpers

A dynamic-tail layout emits:

- `HAS_DYNAMIC_TAIL: bool`
- `TAIL_PREFIX_OFFSET: usize`
- `tail_len(data: &[u8]) -> Result<u32, ProgramError>`
- `tail_read(data: &[u8]) -> Result<T, ProgramError>`
- `tail_write(data: &mut [u8], tail: &T) -> Result<usize, ProgramError>`

Example handler flow:

```rust
pub fn rename(ctx: Context<Rename>, new_label: BoundedString<32>) -> ProgramResult {
    let mut data = ctx.multisig.try_borrow_mut()?;
    let mut tail = Multisig::tail_read(&data)?;
    tail.label = new_label;
    Multisig::tail_write(&mut data, &tail)?;
    Ok(())
}
```

`tail_write` returns `AccountDataTooSmall` if the existing account cannot hold
the encoded payload. Grow the account first through Hopper's lifecycle helpers
when the new tail can exceed the currently allocated space.

## When to choose a tail vs an extension segment

Use a dynamic tail when:

- The variable data belongs to one fixed layout.
- The whole tail is usually read or written together.
- The maximum encoded size is small enough to bound rent and realloc decisions.
- Independent borrow tracking for individual tail elements is not required.

Use extension segments when:

- You need multiple independently borrowed variable regions.
- The data has a separate migration lifecycle.
- You need a segment registry entry with role/intent metadata.
- The region is large enough that whole-tail decode/writeback is wasteful.

## Migration checklist

1. Keep the hot fixed fields in `#[hopper::state]` as `Wire*`, `[u8; N]`, or
   other alignment-1 Hopper wire types.
2. Group Quasar dynamic fields into one tail struct.
3. Implement `TailCodec` with a deterministic bounded encoding.
4. Allocate account space for `Fixed::LEN + 4 + Tail::MAX_ENCODED_LEN` when the
   account is initialized.
5. Use generated segment accessors for fixed fields and `tail_read` /
   `tail_write` only at handlers that need dynamic data.
6. Move to extension segments if tail updates become too large or need separate
   borrow leases.
