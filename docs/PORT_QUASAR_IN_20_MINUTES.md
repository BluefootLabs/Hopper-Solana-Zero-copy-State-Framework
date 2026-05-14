# Port Bounded Dynamic Account Fields in 20 Minutes

This guide shows how to move a fixed account body plus bounded dynamic fields
into Hopper without giving up the zero-copy hot path. The pattern is useful for
programs that have small labels, signer lists, memo payloads, or other bounded
metadata attached to an otherwise fixed account layout.

Hopper keeps the fixed body as an alignment-1 account overlay and places the
variable-sized data in one explicit dynamic tail:

```text
[ Hopper header ][ fixed account body ][ tail_len: u32 LE ][ encoded tail ]
```

Handlers that only touch fixed fields never decode the tail. Handlers that need
metadata use generated borrowed views or an owned editor/writeback helper,
making the dynamic path easy to audit.

## Fixed Vault

Start with the fixed state that should remain segment-borrowable:

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    #[role(authority)]
    pub authority: Address,

    #[role(balance)]
    pub balance: WireU64,

    #[role(bump)]
    pub bump: u8,
}
```

The fixed fields still receive the normal Hopper constants, layout ID,
validated loads, and generated segment accessors.

## Bounded Dynamic Account

Use `#[hopper::dynamic_account]` when porting Quasar-style bounded fields. The
fixed fields remain in the account body; fields marked with `#[tail(...)]` move
into the compact dynamic tail.

```rust
#[hopper::dynamic_account(disc = 7, version = 1)]
pub struct Multisig {
    #[role(threshold)]
    pub threshold: u64,

    #[tail(string<32>)]
    pub label: String,

    #[tail(vec<Address, 10>)]
    pub signers: Vec<Address>,

    #[tail(vec<u16, 10>)]
    pub weights: Vec<u16>,
}
```

The macro generates a fixed `Multisig` body, a `MultisigTail`, borrowed
`MultisigTailView`, owned `MultisigTailEditor`, and `Multisig::ALLOC_SPACE`.
Native `u64` in the fixed body is stored as `WireU64` and exposed through the
generated `threshold()` getter. `string<N>` lowers to `HopperString<N>` and
`vec<T, N>` lowers to `HopperVec<T, N>` inside the generated tail. `Address` /
`Pubkey` vectors keep borrowed-slice views; other `TailElement` vectors return
`HopperVec<T, N>` values through generated view helpers.

For custom named tail payloads, keep using the explicit lower-level pair:
`hopper_dynamic_fields!` plus `#[hopper::state(dynamic_tail = Tail)]`.

## Read and Write the Tail

The generated dynamic account emits:

- `HAS_DYNAMIC_TAIL`
- `TAIL_PREFIX_OFFSET`
- `ALLOC_SPACE`
- `tail_len(data)`
- `tail_view(data)`
- `tail_editor(data)`
- `tail_read(data)`
- `tail_write(data, tail)`
- field helpers such as `label(data)`, `signers(data)`, `set_label(data, ...)`,
  `push_unique_signer(data, ...)`, and `remove_signer(data, ...)`

Example update handlers:

```rust
pub fn rename_multisig(multisig: &AccountView, label: &str) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    Multisig::set_label(&mut data, label)
}

pub fn add_signer(multisig: &AccountView, signer: Address) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    Multisig::push_unique_signer(&mut data, signer).map(|_| ())
}
```

Use this pattern when the dynamic fields are small and logically owned by the
same account. If the dynamic region needs independent borrow tracking,
independent migrations, or large append-only history, split it into an explicit
segment or companion account instead.

## Full Example

See [`examples/quasar-port-20-min`](../examples/quasar-port-20-min/src/lib.rs)
for a workspace example containing the fixed vault, dynamic-account multisig,
initialization helper, signer-list mutation helpers, and threshold check. The
release checklist runs `cargo check -p hopper-quasar-port-20-min` and
`cargo test -p hopper-quasar-port-20-min` before the guide is treated as
compile-checked release material.