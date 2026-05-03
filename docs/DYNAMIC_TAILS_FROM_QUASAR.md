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

#[hopper::state(disc = 7, dynamic_tail = MultisigTail)]
#[repr(C)]
pub struct Multisig {
    pub threshold: WireU64,
}

pub struct MultisigTail {
    pub label: BoundedString<32>,
    pub signers: BoundedSigners<10>,
}
```

`threshold` remains a zero-copy field. `label` and `signers` move into the
single tail payload and are decoded only when a handler asks for them.

## A small bounded tail codec

`TailCodec` is a minimal Borsh-subset trait. Hopper implements it for integers,
`bool`, `[u8; N]`, and `Option<T>`. Programs implement it for richer shapes.

```rust
use hopper::prelude::*;

pub struct BoundedString<const N: usize> {
    pub len: u8,
    pub bytes: [u8; N],
}

impl<const N: usize> TailCodec for BoundedString<N> {
    const MAX_ENCODED_LEN: usize = 1 + N;

    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        let len = self.len as usize;
        if len > N || out.len() < 1 + len {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[0] = self.len;
        out[1..1 + len].copy_from_slice(&self.bytes[..len]);
        Ok(1 + len)
    }

    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        let len = *input.first().ok_or(ProgramError::InvalidAccountData)? as usize;
        if len > N || input.len() < 1 + len {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut bytes = [0u8; N];
        bytes[..len].copy_from_slice(&input[1..1 + len]);
        Ok((Self { len: len as u8, bytes }, 1 + len))
    }
}

pub struct BoundedSigners<const N: usize> {
    pub len: u8,
    pub keys: [[u8; 32]; N],
}

impl<const N: usize> TailCodec for BoundedSigners<N> {
    const MAX_ENCODED_LEN: usize = 1 + 32 * N;

    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        let len = self.len as usize;
        let bytes = 1 + 32 * len;
        if len > N || out.len() < bytes {
            return Err(ProgramError::AccountDataTooSmall);
        }
        out[0] = self.len;
        for i in 0..len {
            let start = 1 + 32 * i;
            out[start..start + 32].copy_from_slice(&self.keys[i]);
        }
        Ok(bytes)
    }

    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        let len = *input.first().ok_or(ProgramError::InvalidAccountData)? as usize;
        let bytes = 1 + 32 * len;
        if len > N || input.len() < bytes {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut keys = [[0u8; 32]; N];
        for i in 0..len {
            let start = 1 + 32 * i;
            keys[i].copy_from_slice(&input[start..start + 32]);
        }
        Ok((Self { len: len as u8, keys }, bytes))
    }
}

impl TailCodec for MultisigTail {
    const MAX_ENCODED_LEN: usize =
        BoundedString::<32>::MAX_ENCODED_LEN + BoundedSigners::<10>::MAX_ENCODED_LEN;

    fn encode(&self, out: &mut [u8]) -> Result<usize, ProgramError> {
        let n0 = self.label.encode(out)?;
        let n1 = self.signers.encode(&mut out[n0..])?;
        Ok(n0 + n1)
    }

    fn decode(input: &[u8]) -> Result<(Self, usize), ProgramError> {
        let (label, n0) = BoundedString::<32>::decode(input)?;
        let (signers, n1) = BoundedSigners::<10>::decode(&input[n0..])?;
        Ok((Self { label, signers }, n0 + n1))
    }
}
```

The example uses `u8` lengths because the bounds are small. For larger tails,
use `u32` length prefixes inside your codec or split the data into extension
segments.

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
