# Why Hopper

Hopper is a zero-copy framework for Solana programs. You write a program in
Rust with typed accounts, generated checks, and one-call CPIs, and it runs at
the cost of hand-written code.

## The choice it removes

Solana programs have had to choose. A framework is easy to write and pays for
its abstractions on every instruction: accounts are deserialized into structs,
copied, and written back. Hand-written code on Pinocchio is as cheap as a
program gets and leaves every check to you. Hopper is built so you do not have
to pick.

## What Hopper does

- **Your accounts are never copied.** An account is a view into the memory the
  runtime hands your program. Reading a typed field costs what reading those
  bytes costs, whether the account holds ten bytes or ten megabytes.
- **The checks are written for you.** Signer, owner, seed, and layout checks
  come from your account struct. Every borrow is tracked, a resize is held to
  what the runtime allows, and a CPI refuses an account your code still holds.
  The unchecked versions exist, marked `unsafe`, where a reviewer will see them.
- **One framework, top to bottom.** Write most of a program with `#[program]`
  and `#[derive(Accounts)]`. When one instruction needs every compute unit,
  write it against the raw layer in the same crate, with the same account type
  and the same errors.
- **Every claim comes with evidence.** Each unsafe block is mapped to the test
  that reaches it, the public API is locked between releases, the account
  parser has machine-checked proofs, and every release runs on devnet with its
  transactions and program hashes published.

## What it costs

pina's framework-comparison fixtures, built with its release recipe and run
once per instruction in Mollusk, measured on `main` on 2026-09-30, ahead of
the 0.5 release:

| Program | Hand-written Pinocchio | Hopper raw | Hopper framework |
|---|---:|---:|---:|
| Hello world | 111 CU, 3,160 B | 111 CU, 1,456 B | 127 CU, 1,824 B |
| PDA counter, create | 1,490 CU | 1,514 CU | 1,471 CU |
| PDA counter, update | 1,721 CU | 1,722 CU | 325 CU |

The raw counter is the same program as Pinocchio's. The framework counter
checks the signer, the owner, the layout, and the PDA before its handler runs,
and it checks the PDA with one hash from the bump stored in the account. The
full table is in
[`bench/framework-comparison/results`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/bench/framework-comparison/results/RESULTS.txt),
and `scripts/bench-framework-comparison.py` reproduces it.

## Start

- [Your first program](FIRST_FIVE_MINUTES.md), then a funded example:
  [token escrow](../examples/hopper-escrow/README.md),
  [SOL vault](../examples/hopper-vault/README.md), or
  [governance and treasury programs](GOVERNANCE_PROGRAMS.md).
- [Coming from Pinocchio](FROM_PINOCCHIO.md): the port, name by name.
- [Moving a program to Hopper](PROGRAM_MIGRATION.md): from an Anchor-style
  program.
