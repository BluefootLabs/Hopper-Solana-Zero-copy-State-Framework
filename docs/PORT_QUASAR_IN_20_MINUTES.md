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
metadata call the generated `tail_read` and `tail_write` helpers, making the
dynamic path easy to audit.

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

## Bounded Tail

Use `hopper_dynamic_tail!` for a compact tail payload with bounded strings and
vectors:

```rust
hopper_dynamic_tail! {
    pub struct MultisigTail {
        label: BoundedString<32>,
        signers: BoundedVec<Address, 10>,
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 7, version = 1, dynamic_tail = MultisigTail)]
pub struct Multisig {
    #[role(threshold)]
    pub threshold: WireU64,
}

impl Multisig {
    pub const ALLOC_SPACE: usize = Self::INIT_SPACE + 4 + MultisigTail::MAX_ENCODED_LEN;
}
```

`BoundedString<N>` stores UTF-8 bytes with a `u16` length prefix.
`BoundedVec<T, N>` stores a `u16` element count followed by each element's
`TailCodec` encoding. `Address` implements `TailCodec`, so bounded signer lists
work without a custom codec.

## Read and Write the Tail

The dynamic-tail state macro emits:

- `HAS_DYNAMIC_TAIL`
- `TAIL_PREFIX_OFFSET`
- `tail_len(data)`
- `tail_read(data)`
- `tail_write(data, tail)`

Example update handlers:

```rust
pub fn rename_multisig(multisig: &AccountView, label: &str) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    let mut tail = Multisig::tail_read(&data)?;
    tail.label.set_str(label)?;
    Multisig::tail_write(&mut data, &tail)?;
    Ok(())
}

pub fn add_signer(multisig: &AccountView, signer: Address) -> ProgramResult {
    multisig.require_writable()?;
    let mut data = multisig.try_borrow_mut()?;
    let mut tail = Multisig::tail_read(&data)?;
    tail.signers.push(signer)?;
    Multisig::tail_write(&mut data, &tail)?;
    Ok(())
}
```

Use this pattern when the dynamic fields are small and logically owned by the
same account. If the dynamic region needs independent borrow tracking,
independent migrations, or large append-only history, split it into an explicit
segment or companion account instead.

## Full Example

See [`examples/quasar-port-20-min`](../examples/quasar-port-20-min/src/lib.rs)
for a complete compile-checked example containing the fixed vault, bounded
multisig tail, and update helpers.