# Large Zero-Copy Accounts

Large accounts are a first-class Hopper pattern. The rule is the same as the rest of the framework:

- Hopper-owned large accounts use Hopper layout contracts, headers, segment guards, and borrow-scoped access.
- Known foreign large accounts use `ExternalAccount<T>` adapters with guard-owned views and bounds-checked accessors.
- Raw account access stays explicit when a protocol needs Pinocchio-style control.

## Init Versus Zero

Normal account initialization can be constrained by CPI allocation limits. For large accounts, pre-create the account with the required space, then initialize the bytes in-place.

Hopper-owned flow:

```rust
#[derive(Accounts)]
pub struct InitQueue<'info> {
    #[account(zero)]
    pub queue: InitAccount<'info, EventQueue>,
}

pub fn init_queue(ctx: Ctx<InitQueue>) -> ProgramResult {
    let mut queue = ctx.accounts.queue.load_init()?;
    queue.initialize()?;
    Ok(())
}
```

`load_init()` is an Anchor-compatible alias for Hopper's `load_after_init()`.

## Segment-Safe Mutation

For large accounts that split hot/cold regions, use segments and field leases so mutation stays local and borrow conflicts are explicit.

```rust
let header = account.segment_ref::<QueueHeader>(&mut borrows, HEADER_OFFSET, HEADER_SIZE)?;
let mut ring = account.segment_mut::<RingHeader>(&mut borrows, RING_OFFSET, RING_SIZE)?;
```

Keep each accessor bounds-checked. Do not cast arbitrary byte offsets to aligned Rust references unless the layout type and account model prove the alignment contract.

## Ring Buffers And Queues

Large event queues and order books should use fixed-width records, checked indices, and wraparound arithmetic that returns `ProgramError` on overflow. A queue should validate:

- capacity is nonzero and fits the account data length;
- head/tail/count never exceed capacity;
- every record offset is computed with checked add/mul;
- writers hold a mutable segment lease for the affected region;
- readers hold only the narrow shared segment lease they need.

## External Large Accounts

Known foreign large accounts should not be forced into Hopper headers. Use `ExternalAccount<T>` and an adapter view that owns the data borrow guard:

```rust
pub struct ForeignOrderBook;

pub struct ForeignOrderBookView<'a> {
    data: Ref<'a, [u8]>,
}

impl ForeignOrderBookView<'_> {
    pub fn best_bid(&self) -> Result<u64> {
        // Bounds-checked field read or lens-backed accessor.
        Ok(0)
    }
}

impl ExternalZeroCopy for ForeignOrderBook {
    type View<'a> = ForeignOrderBookView<'a>;

    const OWNER: Option<Address> = Some(ORDERBOOK_PROGRAM_ID);
    const MIN_LEN: usize = ORDERBOOK_HEADER_LEN;

    fn view<'a>(data: Ref<'a, [u8]>) -> Result<Self::View<'a>> {
        Ok(ForeignOrderBookView { data })
    }
}
```

Use `lens::<T, OFFSET>()` for simple checked field reads and `snapshot_hash()` or domain-specific postconditions when CPI must not mutate the account unexpectedly.

## Validation Checklist

Before treating a large account as production-ready:

- compile the example for the target backend;
- run host tests for all index boundaries;
- run SBF build proof for the program crate;
- verify zero-copy access does not require heap allocation;
- check every offset with overflow-aware arithmetic;
- validate duplicate policy for remaining accounts that select queue/orderbook sidecars;
- add explain output for known external layouts and redact unknown bytes.
