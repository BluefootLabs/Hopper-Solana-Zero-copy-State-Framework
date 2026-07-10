# hopper-smoke

A single macro-first program that exercises a broad slice of the Hopper
framework end-to-end and is verified live on devnet. It is the
"does the whole pipeline actually work on a real cluster?" smoke test.

## What it exercises

| Feature | Where |
|---|---|
| `#[account]` versioned zero-copy layout + 16-byte header | `Vault` |
| `#[derive(Accounts)]` constraints: `init`, `payer`, `space`, `mut`, `signer`, `has_one`, `close` | contexts |
| `#[program]` dispatch with typed handler args | `smoke_program` |
| System `CreateAccount` CPI (account init) | `initialize` |
| System `Transfer` CPI | `deposit` |
| Clock sysvar via the native syscall path | `get_clock()` |
| Typed, zero-alloc event through `sol_log_data` | `DepositEvent` |
| Checked arithmetic (no silent overflow) | all handlers |
| Program-owned lamport debit | `withdraw` |
| `close` constraint (zero data, refund lamports) | `close` |
| `strict_writes` static write policy from `mut(balance)` | `Withdraw` |
| Self-describing tx: Ok-only `emit_touch_map` opt-in (I7) | `Withdraw` |

Instructions: `0` initialize, `1` deposit, `2` withdraw, `3` close.

## Build

```sh
cargo build-sbf            # emits target/deploy/hopper_smoke.so (~27 KiB)
```

The binary carries the opt-in touch-map machinery (`touch-map` feature +
segment-lease write policy), which is why it is larger than the earlier
20 KiB build — the emission is per-context opt-in precisely so programs
that do not ask for it never pay this size or the per-record CU.

## Self-describing withdraw (touch map)

`Withdraw` opts in via `#[accounts(strict_writes, emit_touch_map)]`. On
every **successful** withdraw the generated dispatcher emits the
instruction's touch map as one `sol_log_data` record (`Program data:`
log line, magic `0x7A`, version `0x01`): `W slot 1 [48..56)` — a Write
of the vault's `balance` field bytes, which is exactly what the handler
does through the `mut(balance)` segment lease. A **failed** withdraw
emits nothing (the emit is routed on the handler's Ok path only), so a
rolled-back instruction never advertises effects it did not keep.
`hopper tx explain <signature>` decodes the record from the transaction
log stream with field-level joins.

The loop is proven end-to-end by `tests/touch_map_e2e.rs`, which runs
the compiled `.so` in an in-process SVM (Mollusk via `hopper-test`),
captures the log stream, and decodes it the way `hopper tx explain`
does:

```sh
cargo build-sbf                 # build the artifact first
cargo test -p hopper-smoke      # e2e: one record on Ok, zero on Err
```

## Live devnet evidence

Deployed from authority `HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT`:

- **Program id:** `2YPBvKJ8h37bUEFBrmytzNuKfUJ5Q2o2tkTiqRCZdjme`
- **SBF size:** 20 280 bytes

A full `initialize → deposit → withdraw` run confirmed on devnet
(vault `5wa4qMWu1TTtbnxQdnwNRpEaAyuK6PvWQzUCKGSD2DBQ`):

| Instruction | Result | Signature |
|---|---|---|
| initialize | header `disc=7 ver=1`, layout_id `60 62 67 a6 bb e3 18 f7`, `created_at=1781532581` | `4Q16FuwMASHsZazmubGUcLX8M8b53vbV4f7Kq3sxbo8YNP9XjmLa8wAZ9tpkoW7gpsEhkwReXeGP7XueVQ8GnMYU` |
| deposit 0.01 SOL | `balance=10000000 deposits=1` | `517g1QF95FaZuQZCejc9d5yhc5FYWAL6PczQD5HxbLd2f9Q3GJZKM9rbc1nZhJFhowWZMsdsWJeBZcAQXzdHNHk3` |
| withdraw 0.004 SOL | `balance=6000000` | `5EF3pcbBZNpoSxE5hSRch9zaMd4iYRW2xgM8DZhcahpLPtAYTqEF3Eqpmjt1jnExfqCjoHoNLw1v7egAwhCPcZHq` |

The header's `layout_id` is the SHA-256 layout fingerprint Hopper stamps on
every account, so a client can verify it is decoding the exact struct shape
the program was compiled against before reading any field.
