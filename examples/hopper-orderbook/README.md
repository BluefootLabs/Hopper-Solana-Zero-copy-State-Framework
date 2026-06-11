# Hopper Orderbook

Showcases **segment-level borrows on a large (>100 KB) zero-copy
account**. A central limit orderbook keeps three independent regions
inside one account — `bids`, `asks`, and an `events` ring — and each
instruction borrows only the byte range it needs. Posting a bid touches
*only* the bids segment; matching touches asks + events; the crank reads
only events. The runtime `SegmentBorrowRegistry` rejects any accidental
overlap, so there is no second deserialize pass and no full-account copy:
a fill that mutates 64 bytes of a 140 KB account pays for 64 bytes, not
140 KB.

This is the property Anchor and Quasar cannot express: they model an
account as a single typed blob, so any mutation conceptually borrows the
whole thing.

## Account layout

```text
Orderbook account (segmented, ~140 KB):
  [Header: 16 bytes]
  [Segment registry: 4 + 3 x 16 bytes]
  [bids  segment  : 1024 orders x 56 B + framing]
  [asks  segment  : 1024 orders x 56 B + framing]
  [events segment : 512 events x 48 B + framing]
```

## Instruction Map

- `0` = `InitBook`    : create the segmented account and zero all regions
- `1` = `PostBid`     : append a resting buy order into the `bids` segment
- `2` = `PostAsk`     : append a resting sell order into the `asks` segment
- `3` = `Match`       : cross top-of-book, write a fill into `events`
- `4` = `CrankEvents` : read the `events` segment and advance the head

## Devnet

Deployed to devnet in this pass:

- Program id: `CK3XYYsbFducx9UEEWWLGAVnSAhGkMtM1TKLe8PDP6dJ`
- `.so` size: 18 408 bytes

```bash
hopper build -p hopper-orderbook
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_orderbook-keypair.json
```

Integration test (gated so the default `cargo test` stays offline):

```bash
HOPPER_DEVNET=1 \
HOPPER_ORDERBOOK_PROGRAM_ID=CK3XYYsbFducx9UEEWWLGAVnSAhGkMtM1TKLe8PDP6dJ \
HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
cargo test -p hopper-orderbook --test devnet -- --nocapture
```

The book account is larger than the 10 KB single-CPI allocation limit,
so `InitBook` runs on its own transaction. If a deployed build predates
large-account support, `InitBook` fails and the test surfaces the
program error rather than masking it.

## Verify

```bash
cargo test -p hopper-orderbook          # host-side layout/disjointness invariants
hopper build -p hopper-orderbook        # SBF artifact
```
