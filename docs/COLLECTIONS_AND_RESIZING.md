# Zero-copy collections and resizing

Hopper offers familiar collection operations without adding a deserialize and
serialize boundary. The collection remains a view over account bytes, and the
instruction's normal ownership, writable, borrow, and `strict_writes` contracts
still apply.

## Choose the smallest fitting shape

| Need | Hopper type | Capacity source | Key property |
|---|---|---|---|
| Small bounded text | `String<'a, N>` | Type-level `N` | Compact tail payload with a fixed maximum |
| Small bounded list | `Vec<'a, T, N>` | Type-level `N` | Familiar bounded field syntax |
| List that grows with the account | `Seq<'a, T>` | Live account length | O(1) push, no capacity in the layout type |
| Stable IDs and O(1) reuse | `Slab<T>` / `TailSlab<T>` | Slab header and live region | Bitmap-backed alloc/free with double-free refusal |
| Fixed queue or ring semantics | `RingBuffer<T>` / `TailRing<T>` | Fixed or tail region | Strict or overwrite-on-full modes |

Use a bounded field when the protocol has a meaningful maximum. Use `Seq` when
capacity should follow allocation size. Use `Slab` when entries need stable
indices and deletion without shifting later entries.

## Growable `Seq`

```rust
#[hopper::account(discriminator = 10, version = 1)]
pub struct Roster<'a> {
    pub authority: Address,
    pub members: Seq<'a, Address>,
}
```

The wire is `[count: u32][elements...]`. Capacity is computed from the live
tail length, so growing the account increases capacity without changing the
layout type or fingerprint. The generated cursor offers `len`, `is_empty`,
`capacity`, `remaining_capacity`, `get`, `set`, `push`, `pop`, and iteration.
Under `strict_writes`, declare the tail as an open-ended tail range so the fixed
header remains protected.

## Stable-ID `Slab`

```rust
let bytes = Slab::<Order>::required_bytes(128);
Slab::<Order>::init(account_tail, 128)?;

let mut orders = Slab::<Order>::from_bytes_mut(account_tail)?;
let id = orders.alloc(order)?;
orders.get_mut(id)?.remaining = new_remaining;
orders.free(id)?;
```

Allocation and free are O(1). The occupancy bitmap refuses reads and writes to
freed slots, `free` refuses double-free, and corrupted free-list heads are
rejected before pointer arithmetic. The API exposes `len`, `is_empty`,
`capacity`, `remaining_capacity`, and `is_full`; `count` remains available when
the exact on-wire `u32` is useful.

For a compact account with a fixed head and slab tail, use
`CompactTail::tail_slab`, `tail_slab_mut`, `init_tail_slab`, and
`account_size_for_slab` rather than hand-computing offsets.

## Safe grow and shrink during migration

Bind-time migration can resize before transforming the typed state:

```rust
#[account(
    mut,
    migrate(from = VaultV1, with = v1_to_v2, resize = grow, payer = authority)
)]
pub vault: Account<'info, VaultV2>,
```

- `resize = grow` grows only when the new layout needs more bytes.
- `resize = fit` also opts into shrinking to the new layout size.
- The declared payer funds only the live-rent deficit.
- Shrinking refunds only the freed rent delta, never existing deposits.
- Newly allocated bytes are zero-filled before the transform.
- Owner, signer, writable, system-program, size, and rent checks fail closed.
- Solana transaction rollback restores the original account if the transform or
  later handler fails.

Resizing does not bypass mutation contracts. Migration-bearing roles remain
writable in generated clients and manifests, and strict write/effect tooling
continues to describe the instruction's actual mutation surface.

## Fuzz the declaration, not a second handwritten model

```sh
hopper compile --emit manifest --package my-program \
  --out target/program.manifest.json --force
hopper fuzz generate --program target/program.manifest.json \
  --out fuzz/plans/program.plan.json --corpus fuzz/corpus/program
hopper fuzz check --program target/program.manifest.json \
  --plan fuzz/plans/program.plan.json
hopper fuzz run --program target/program.manifest.json \
  --plan fuzz/plans/program.plan.json --adapter target/debug/my-fuzz-adapter \
  --require-invariant collection-model-equivalence
```

Generation covers truncated layouts and discriminators, field and write-range
boundaries, declared compatibility pairs, missing privileges, wrong identity,
Accounts-derived PDA, lifecycle, and relational constraints, every account
alias pair, declared lamport allow/deny cases, argument bounds, and
remaining-account ceilings. The plan is deterministic and content-addressed,
so manifest drift is a reviewable CI failure.

The generated plan and corpus are inputs for Hopper's adapters and fuzz targets.
They do not invent valid application-specific account fixtures or assert
business semantics that are absent from the manifest. `fuzz run` makes adapter
execution and required invariant hooks a fail-closed gate; the application
adapter still owns the valid starting fixture and collection reference model.
When a hostile structural primitive is unreachable from a production
instruction, the adapter must name and execute the equivalent enforced host
gate rather than claim a transaction occurred. Cicada follows this split with
698 no-skip host semantic cases and a separate compiled-SBF lifecycle suite.
