# Dynamic tails from Quasar dynamic fields

Quasar lets a zero-copy account include bounded dynamic fields such as
`String<'a, 32>` or `Vec<'a, T, 10>` inside the account declaration.
Hopper takes a different route internally: keep the fixed body strictly
zero-copy, then attach one compact encoded dynamic tail after the fixed body.
You can write Quasar-pretty fields directly in `#[hopper::account]`, or use
`#[hopper::dynamic_account]` with explicit `#[tail(...)]` markers when a review
needs the fixed/tail split spelled out in source.

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

#[hopper::account(discriminator = 7, version = 1)]
pub struct Multisig<'a> {
    pub threshold: u64,
    pub label: String<'a, 32>,
    pub signers: Vec<'a, Address, 10>,
    pub weights: Vec<'a, u16, 10>,
}
```

`#[hopper::account]` keeps this authoring shape but lowers fixed multi-byte
scalars to wire-safe wrappers in the emitted type (`u64` -> `WireU64`).
This keeps zero-copy overlays alignment-safe while preserving Quasar-style
source ergonomics.

The macro emits a fixed `Multisig` body, a `MultisigTail`, `ALLOC_SPACE`,
borrowed `tail_view` helpers, and an owned `tail_editor` for writeback.
`threshold` remains a zero-copy field. `label`, `signers`, and `weights` move
into the single compact tail payload and are decoded only when a handler asks
for them. `Address` / `Pubkey` vectors expose borrowed slices; other
`TailElement` vectors expose `HopperVec<T, N>` values through the same view and
editor helpers.

The lifetime in `Multisig<'a>` is authoring syntax for the macro. The emitted
layout type is concrete, so account contexts use `Account<'info, Multisig>`.

Systems-mode spelling stays available when you want every tail decision visible
at the field site:

```rust
#[hopper::dynamic_account(disc = 7, version = 1)]
pub struct Multisig {
    pub threshold: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,
}
```

For explicit systems-mode declarations, prefer `WireU64` / other wire wrappers
for fixed multi-byte scalar fields.

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

The prelude uses the Option-A shape directly: `String<'a, N>` and
`Vec<'a, T, N>` are the account-authoring forms. For ordinary owned Rust
values, use `Text<N>` and `List<T, N>` or the explicit `HopperString<N>` and
`HopperVec<T, N>` names.

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

`#[hopper::account]` auto-upgrades to the dynamic account lowering when it sees
bounded `String<'a, N>`, `Text<N>`, `Vec<'a, T, N>`, or `List<T, N>` fields.
`#[hopper::dynamic_account]` supports the same pretty types plus
`#[tail(string<N>)]` and `#[tail(vec<T, N>)] where T: TailElement` with
`tail_policy = "compact"` (the default). Use the explicit
`hopper_dynamic_fields!` path when you want to name a custom `TailCodec` payload
directly or when an indexed/segmented tail policy is a better fit than one
compact payload.

Hopper also supports named bare final tails for protocols that intentionally
want remaining-bytes semantics:

```rust
#[hopper::account(discriminator = 9, version = 1)]
pub struct Note<'a> {
    pub author: Address,
    pub content: TailStr<'a>,
}
```

`TailStr<'a>` and `TailBytes<'a>` must be the final account field. They consume
the remaining dynamic-tail payload without an inner field-level prefix and are
included in the layout fingerprint as `tail_str` or `tail_bytes`. The account
still has Hopper's outer `u32` dynamic-tail payload length. `TailStr` validates
UTF-8 on `as_str()`; `TailBytes` returns raw bytes.

Bare final tails can follow bounded compact fields. In that case Hopper stores
the bounded field headers/payload first and the raw field consumes the rest:

```rust
#[hopper::account(discriminator = 10, version = 1)]
pub struct Note<'a> {
    pub author: Address,
    pub label: String<'a, 32>,
    pub reviewers: Vec<'a, Address, 4>,
    pub content: TailStr<'a>,
}
```

That is the Quasar-style compact tail with Hopper's extra contract layer: the
raw field is named, final-only, layout-fingerprinted, and length-bounded by the
outer Hopper tail prefix.

## Generated helpers

A dynamic-tail layout emits:

- `HAS_DYNAMIC_TAIL: bool`
- `TAIL_PREFIX_OFFSET: usize`
- `tail_len(data: &[u8]) -> Result<u32, ProgramError>`
- `tail_read(data: &[u8]) -> Result<T, ProgramError>`
- `tail_write(data: &mut [u8], tail: &T) -> Result<usize, ProgramError>`

For raw final-tail accounts, `tail_read` returns a borrowed `NameTail<'a>`,
`tail_write_parts(data, &NameTailHead, raw)` writes the bounded head plus raw
tail bytes, and `space_for_tail(raw_len)` computes allocation for a chosen raw
tail length.

`#[hopper::dynamic_account]` additionally emits:

- `ALLOC_SPACE: usize`
- `tail_capacity(data: &[u8]) -> Result<usize, ProgramError>`
- `tail_view(data: &[u8]) -> Result<NameTailView<'_>, ProgramError>`
- `tail_editor(data: &mut [u8]) -> Result<NameTailEditor<'_>, ProgramError>`
- borrowed string/list accessors such as `label(data)` and `signers(data)`;
    generic vectors return `HopperVec<T, N>`
- setter/editor helpers such as `set_label`, `push_unique_signer`, and
    `remove_signer`
- a local extension trait named `NameAccountTailExt` for `Account<'info, Name>`
    and `InitAccount<'info, Name>`; getters return owned bounded values so
    account-data borrows do not escape the wrapper method

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
        self.multisig.set_label(new_label)
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
2. Spell compact dynamic fields as `String<'a, N>` or `Vec<'a, T, N>` inside
    `#[hopper::account]`. Use `#[tail(string<N>)]` or `#[tail(vec<T, N>)]`
    inside `#[hopper::dynamic_account]` when the systems-mode split should be
    explicit.
3. Allocate account space with `Multisig::ALLOC_SPACE` for the façade path, or
    `Fixed::LEN + 4 + Tail::MAX_ENCODED_LEN` for the explicit path.
4. Use generated segment accessors for fixed fields and tail view/editor helpers
    only in handlers that need dynamic data.
5. Move to extension segments if tail updates become too large or need separate
    borrow leases.
