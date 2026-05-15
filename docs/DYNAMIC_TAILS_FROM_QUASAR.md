# Dynamic tails from Quasar dynamic fields

Quasar lets a zero-copy account include bounded dynamic fields such as
`String<'a, 32>` or `Vec<'a, T, 10>` inside the account declaration.
Hopper takes a different route internally: keep the fixed body strictly
zero-copy, then attach one compact encoded dynamic tail after the fixed body.
You can author that split directly, or use `#[hopper::dynamic_account]` to
write the dynamic fields inline and let the macro lower them into the same
layout.

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

Hopper ergonomic shape:

```rust
use hopper::prelude::*;

#[hopper::dynamic_account(disc = 7, version = 1)]
pub struct Multisig {
    pub threshold: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,

    #[tail(vec<u16, 10>)]
    pub weights: Vec<u16>,
}
```

The macro emits a fixed `Multisig` body, a `MultisigTail`, `ALLOC_SPACE`,
borrowed `tail_view` helpers, and an owned `tail_editor` for writeback.
`threshold` remains a zero-copy field. `label`, `signers`, and `weights` move
into the single compact tail payload and are decoded only when a handler asks
for them. `Address` / `Pubkey` vectors expose borrowed slices; other
`TailElement` vectors expose `HopperVec<T, N>` values through the same view and
editor helpers.

The explicit spelling is still available when you want a custom `TailCodec` or
a tail shape beyond the current `dynamic_account` façade:

```rust
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 7, version = 1, dynamic_tail = MultisigTail)]
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

`#[hopper::dynamic_account]` supports `#[tail(string<N>)]` and
`#[tail(vec<T, N>)] where T: TailElement` with `tail_policy = "compact"` (the
default). Use the explicit `hopper_dynamic_fields!` path when you want to name a
custom `TailCodec` payload directly or when a future indexed/segmented tail
policy is a better fit than one compact payload.

## Generated helpers

A dynamic-tail layout emits:

- `HAS_DYNAMIC_TAIL: bool`
- `TAIL_PREFIX_OFFSET: usize`
- `tail_len(data: &[u8]) -> Result<u32, ProgramError>`
- `tail_read(data: &[u8]) -> Result<T, ProgramError>`
- `tail_write(data: &mut [u8], tail: &T) -> Result<usize, ProgramError>`

`#[hopper::dynamic_account]` additionally emits:

- `ALLOC_SPACE: usize`
- `tail_capacity(data: &[u8]) -> Result<usize, ProgramError>`
- `tail_view(data: &[u8]) -> Result<NameTailView<'_>, ProgramError>`
- `tail_editor(data: &mut [u8]) -> Result<NameTailEditor<'_>, ProgramError>`
- borrowed string/list accessors such as `label(data)` and `signers(data)`;
    generic vectors return `HopperVec<T, N>`
- setter/editor helpers such as `set_label`, `push_unique_signer`, and
    `remove_signer`

Example handler flow:

```rust
#[derive(Accounts)]
pub struct Rename<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    pub authority: Signer<'info>,
}

impl<'info> Rename<'info> {
    pub fn rename(&self, new_label: &str) -> ProgramResult {
        let mut data = self.multisig.as_account().try_borrow_mut()?;
        Multisig::set_label(&mut data, new_label)
    }
}

#[program]
mod multisig_program {
    use super::*;

    #[instruction(1)]
    pub fn rename(ctx: Ctx<Rename>, new_label: HopperString<32>) -> ProgramResult {
        ctx.accounts.rename(new_label.as_str()?)
    }
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

1. Keep hot fields fixed. In `#[hopper::dynamic_account]`, native `u16`, `u32`,
    `u64`, and `bool` fixed fields are stored as Hopper wire types and exposed
    through generated native-value getters.
2. Mark Quasar dynamic fields with `#[tail(string<N>)]` or
    `#[tail(vec<T, N>)] where T: TailElement`, or use
    `hopper_dynamic_fields!` for explicit custom tails.
3. Allocate account space with `Multisig::ALLOC_SPACE` for the façade path, or
    `Fixed::LEN + 4 + Tail::MAX_ENCODED_LEN` for the explicit path.
4. Use generated segment accessors for fixed fields and tail view/editor helpers
    only in handlers that need dynamic data.
5. Move to extension segments if tail updates become too large or need separate
    borrow leases.
