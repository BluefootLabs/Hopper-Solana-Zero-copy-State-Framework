# Hopper Orderbook

A small on-chain example of storing orders in a large segmented account and
mutating individual records with Hopper's byte-range borrow guards. The account
contains **139,356 bytes**: bids, asks, and a bounded event ring. Posting an order
changes its side's four-byte count and one 56-byte record. It validates the
68-byte header/registry prefix and side metadata before acquiring those guards.

This is an **order-storage demonstration**, not a central limit orderbook or a
production exchange. Orders are uncollateralized. There is no price sorting,
price-time priority, crossing, balance accounting, custody, or token settlement.
The caller's optional sequence label is not an authenticated ordering or replay
counter. Bids remain stored until the example's capacity is exhausted; a general
cancellation or recovery API is outside this example's scope.

The program uses `AccountView::segment_ref` and `segment_mut` with
`SegmentBorrowRegistry` for its record and counter accesses. These guards check
byte ranges and borrow conflicts inside execution. Solana still loads and locks
the whole account. Bytes touched are not a compute-unit or transaction-fee
formula, and this example does not establish a speed advantage over another
framework. It does not install a declarative `strict_writes` policy; its tests
check the actual changed bytes in addition to the runtime borrow checks.

## Account layout

| Region | Offset | Bytes | Contents |
| --- | ---: | ---: | --- |
| Header | 0 | 16 | Discriminator 30, version 1, layout `OBDEMO01` |
| Registry | 16 | 52 | Three entries with exact offsets, sizes, flags and versions |
| Bids | 68 | 57,352 | Count, reserved zero word, 1,024 records of 56 bytes |
| Asks | 57,420 | 57,352 | Count, reserved zero word, 1,024 records of 56 bytes |
| Events | 114,772 | 24,584 | Tail/head cursors, 512 records of 48 bytes |

Every operational instruction checks the owner, writability, exact account size,
header, and registry against this layout. Malformed counts or queue cursors are
refused before mutation. This layout identifier differs from the earlier example;
initialize a fresh account when deploying this revision.

## Instructions

The first byte selects an instruction. Integers in the payload are little endian.
All account lists and payload lengths are exact.

| Tag | Instruction | Accounts | Payload after tag |
| ---: | --- | --- | --- |
| 0 | `InitBook` | Payer signer, book signer/writable, System Program | Empty |
| 1 | `PostBid` | Order owner signer, book writable | Price `u64`, size `u64`, optional sequence `u64` |
| 2 | `PostAsk` | Order owner signer, book writable | Same as `PostBid` |
| 3 | `RecordAskEvent` | Latest ask's maker signer, book writable | Empty |
| 4 | `CrankEvents` | Book writable | Empty |

`InitBook` initializes an already allocated, program-owned, fully zeroed account.
The account is larger than the single-CPI 10 KB growth limit: create it with a
top-level System Program instruction immediately before initialization in the
same transaction. Requiring the book signature prevents another caller from
claiming a prepared account. Reinitialization and dirty body bytes are refused;
initialization never clears existing state.

`PostBid` and `PostAsk` append positive-price, positive-size records. `RecordAskEvent`
removes only the most recently appended ask and requires that record's maker to
sign. It places the removed ask's owner, price, and size into the event ring; this
is a demonstration event, **not evidence of a fill**. Opcode 3 replaces the old
misnamed `Match` handler and now requires the maker account before the book.

A full ring refuses another event before removing an ask, preserving unread
records. `CrankEvents` permissionlessly logs one record and advances the head by
one. Cursor arithmetic handles `u32` wraparound. Consumed slots retain their bytes
until reused. Logs can be truncated and are not a durable or authenticated
settlement receipt. No off-chain service is needed for the state transitions;
clients may optionally consume the logs.

## Verify

```bash
cargo test -p hopper-orderbook --lib --locked
hopper build -p hopper-orderbook
```

Host tests cover initialization authorization, reinitialization, dirty accounts,
exact changed ranges, maker-only ask removal, event draining, full-ring refusal,
cursor wraparound, malformed layouts/counters, and invalid order input. These
host checks do not measure SBF compute or prove transaction rollback.

The compiled suite exercises initialization, posting, maker authorization,
event recording/draining, and refusal rollback against the built ELF. It compares
complete account state and requires initialization to stay within 200,000 CU.
Initialization checks every byte for zero using 32-byte blocks and a remainder
check; it does not skip the unused record capacity.

```bash
HOPPER_ORDERBOOK_SBF=/abs/path/hopper_orderbook.so \
cargo test --manifest-path bench/framework-comparison/verifier/Cargo.toml \
  --test orderbook_sbf --locked -- --ignored --nocapture
```

## Devnet evidence

The existing opt-in live test creates a fresh book, initializes it, and posts one
bid. It decodes the registry and verifies that header, registry, asks, events,
bids reserved word, and unused bid bytes remain unchanged. It emits a finalized
receipt on public devnet. This test does **not** exercise every instruction or
refusal path; the host suite covers the additional cases above.

Deploy this source to a fresh program ID before collecting new live evidence.
Earlier deployment IDs and artifact sizes do not establish the behavior of this
revision.

```bash
hopper build -p hopper-orderbook
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_orderbook-keypair.json

HOPPER_DEVNET=1 \
HOPPER_REQUIRE_DEVNET=1 \
HOPPER_ORDERBOOK_PROGRAM_ID=REPLACE_WITH_FRESH_PROGRAM_ID \
HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
HOPPER_DEVNET_RECEIPT=/abs/path/orderbook-receipt.json \
cargo test -p hopper-orderbook --test devnet -- --nocapture
```
