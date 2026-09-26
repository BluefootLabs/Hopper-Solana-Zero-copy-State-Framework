# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

Hopper is a **zero-copy Solana program framework** for building applications
that hold assets, transfer tokens, settle trades, and enforce on-chain rules.
Write Rust handlers with typed accounts, access state in place, and invoke
other Solana programs through checked CPI helpers.

## Build the program your users need

| Application | Working starting point |
|---|---|
| Token trading and settlement | [Funded token escrow](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-escrow/README.md): deposit, atomic exchange, cancellation, surplus refunds, and rent recovery |
| SOL custody and payments | [SOL vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-vault/README.md): create, deposit through the System Program, and authorized withdrawal |
| Multisig administration | [Bounded multisig](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-bounded-multisig/README.md): member-approved payments, expiring single-use payouts, permissionless execution, and revocation |
| Delegated treasury spending | [Treasury](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-treasury/README.md): real SOL transfers, operator permissions, period budgets, freeze controls, and live-clock cooldowns |
| Token claims and rewards | Token CPI builders plus vesting, staking, and distribution math; add your eligibility, funded custody, and replay rules |
| NFT and cNFT markets | Token Metadata helpers and application accounts; cNFTs require a custom Bubblegum integration |

The escrow example supports classic SPL Token with explicit mint/account
restrictions. The [Token-2022 guide](https://hopperzero.dev/docs/token-2022)
covers the separate extension-aware APIs. The orderbook example stores orders;
a matching engine and exchange settlement are application logic.

## Start with ordinary Rust

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.4.0", features = ["proc-macros"] }
```

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub count: WireU64,
}

#[derive(Accounts)]
pub struct Increment<'info> {
    #[account(mut, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

#[program]
mod counter {
    use super::*;

    #[instruction(1)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        ctx.accounts.counter.get_mut()?.count.checked_add_assign(1)
    }
}
```

Read [your first program](https://hopperzero.dev/docs/first-five), then follow a
funded example through its account creation, authorization, transfers, and tests.

## One execution stack, from handlers to syscalls

- **Framework:** typed accounts, constraints, dispatch, initialization, account closure,
  migrations, bounded collections, and generated clients.
- **Runtime:** checked account borrows, PDA helpers, CPI validation, and optional
  policies restricting tracked data and lamport writes.
- **Native:** loader-memory parsing, duplicate-account resolution, entrypoints,
  Solana syscalls, and explicit low-level APIs.

`hopper-native` has no crate dependencies. `hopper-runtime` uses that native
layer directly. Application programs run on Solana's SVM; no off-chain service
is required to authorize their ordinary instructions or execute transfers.
Host tools generate clients, inspect accounts, and collect evidence.

SOL wallet deposits invoke the System Program. SPL Token balances change through
the appropriate token program. A program may directly debit lamports only from
accounts it owns. Zero-copy account access does not bypass those runtime rules.

## Choose the state contract you need

Headered accounts validate owner, discriminator, version, and layout identity.
Compact accounts use an explicit discriminator and size contract. Bounded
strings and vectors keep variable data controlled; segment borrows let handlers
work with selected fields without copying an entire account.

Optional write policies constrain Hopper-tracked accesses within the program.
They do not sandbox arbitrary downstream programs, create byte-level transaction
parallelism, or guarantee a fee discount. Solana locks writable accounts.

## Tested programs and inspectable results

The funded escrow completed **33 finalized devnet transactions** on September 26,
2026, with full expected account-state checks and matching deployed ELF bytes
before and after the run. Its local compiled tests also exercise token CPI
rollback. The named SOL vault completed 19 finalized devnet transactions. The latest
governance/treasury/native run finalized 44 transactions, and four further
transactions verified direct and nested CPI return data.
[Inspect the patch and program validation](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-multisig-2026-09-26).
See [program measurements](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/BENCHMARKS.md) and
[release status](https://hopperzero.dev/docs/release-status) for scope and artifacts.

The registry framework version is **0.4.0**. Repository examples evolve
independently and are not published crates. The native/runtime **0.4.2** safety patch is devnet-tested; see the [release record](docs/RELEASE_0_4_VALIDATION.md) for publication status. Support packages `grillo-*` and
`hopper-topology` use their own 0.1.0 versions.

## Documentation

- [Program architecture](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/ARCHITECTURE.md)
- [Writing handlers and accounts](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/WRITING_HOPPER_PROGRAMS.md)
- [Token escrow](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-escrow/README.md)
- [Governance and treasury programs](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/GOVERNANCE_PROGRAMS.md)
- [Bounded fields](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/BOUNDED_FIELDS.md) and [dynamic tails](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/DYNAMIC_TAILS.md)
- [On-chain write policies](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/ONCHAIN_BYTE_POLICIES.md)
- [Capabilities and integration boundaries](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/PROGRAM_CAPABILITIES.md)
- [Safety and unsafe invariants](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/UNSAFE_INVARIANTS.md)

Framework code is licensed under MIT OR Apache-2.0 unless a component states
otherwise. See [security reporting](SECURITY.md) before disclosing a vulnerability.
