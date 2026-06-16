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

Instructions: `0` initialize, `1` deposit, `2` withdraw, `3` close.

## Build

```sh
cargo build-sbf            # emits target/deploy/hopper_smoke.so (~20 KiB)
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
