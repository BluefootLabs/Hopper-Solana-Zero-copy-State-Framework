# Hopper

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
![no_std](https://img.shields.io/badge/no__std-yes-green.svg)

**A Rust framework for Solana with zero-copy account state.** Declare account
layouts and validation, write handlers, and call other programs through CPI.
Supported layouts borrow the account bytes supplied to your program, so a field
update does not require deserializing and serializing the whole account.

Use the framework for application code and Hopper's own native runtime for
instructions that need direct control over accounts and syscalls. Your
program executes on chain; Hopper needs no separate execution service.

**Hopper 0.5.0 is published.** This checkout also contains explicitly marked
unreleased additions. Upgrade the framework,
runtime, and native crates together; see [the migration guide](docs/MIGRATION_0_5.md)
for API changes and [0.5 validation](docs/RELEASE_0_5_VALIDATION.md)
for token tests, registry availability, and validation scope.

The next release adds [composable checked borrowed arguments](docs/BORROWED_ARGUMENTS.md):
declare a wire layout once, then accept `&MyArgs` in a program handler or parse it
at a lower-level entrypoint. Both paths validate nested option and enum values
without allocating or copying the struct. Borrowed layouts can be followed by
ordinary arguments or an explicit byte tail.
Its [Solana fixture](bench/borrowed-args/README.md) checks exact and tailed inputs,
return bytes, and unchanged accounts after refusals. These APIs are unreleased.

The next release also adds [checked borrowed batches](docs/BORROWED_SLICES.md).
Accept `BoundedSlice<'_, Order, 32>` in a handler to validate a length-prefixed
batch directly in the instruction buffer. Capacity limits and nested value
checks run before handler admission, without copying into a 32-element array.
Manual parsing and generated handlers share the same wire contract.

The workspace now targets **0.6.0**. Its [migration guide](docs/MIGRATION_0_6.md)
explains checked client encoders, alias-preserving metadata, and the client
languages that support bounded arguments. Registry availability remains 0.5.0
until the next publication completes.

The [function lab](bench/function-lab/README.md) exercises Hopper's runtime
functions as a real Solana program, with independent expected values and
explicit cluster-feature requirements. It complements the application and
token suites; a passing fixture is not a claim that every feature is deployed.
The [network baseline](docs/SOLANA_NETWORK_BASELINE.md) records observed syscall
availability and sysvar inconsistencies that applications must account for.

## Build the program your users need

| Application | Working starting point |
|---|---|
| Token trading and settlement | [Funded token escrow](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-escrow/README.md): deposit, atomic exchange, cancellation, surplus refunds, and rent recovery |
| SOL custody and payments | [SOL vault](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-vault/README.md): create, deposit through the System Program, and authorized withdrawal |
| Multisig administration | [Bounded multisig](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-bounded-multisig/README.md): member-approved payments, expiring single-use payouts, permissionless execution, and revocation |
| Delegated treasury spending | [Treasury](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-treasury/README.md): real SOL transfers, operator permissions, period budgets, freeze controls, and live-clock cooldowns |

The escrow example supports classic SPL Token with explicit mint/account
restrictions. The [Token-2022 guide](https://hopperzero.dev/docs/token-2022)
covers the extension-aware APIs, and the
[token lab](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-token-lab/README.md)
exercises shared token operations on both programs, plus Token-2022 extensions, on devnet. The orderbook example stores orders;
a matching engine and exchange settlement are application logic.

For claims, rewards, and asset integrations, start with the
[capability guide](docs/PROGRAM_CAPABILITIES.md). Token CPI, vesting and
distribution math, and Token Metadata helpers are building blocks. Eligibility,
replay protection, marketplace settlement, and Bubblegum integration require
application code.

## Start with ordinary Rust

The next release adds recursively checked borrowed arguments and execution
receipts that invalidate stale self-audit results when source inputs change.
See [release readiness](docs/RELEASE_READINESS.md) for verified gates and open
requirements, and [framework boundaries](docs/FRAMEWORK_BOUNDARIES.md) for
source-pinned comparisons with Pinocchio, Pina, Quasar, and Anchor v2.

```toml
[dependencies]
hopper = { package = "hopper-lang", version = "0.5.0", features = ["proc-macros"] }
```

```rust
use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[derive(Accounts)]
pub struct Increment<'info> {
    #[account(mut, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

#[program(profile = "tiny")]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        let mut counter = ctx.accounts.counter.get_mut()?;
        counter.value.checked_add_assign(1)?;
        Ok(())
    }
}
```

This excerpt from [the counter](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-counter/src/lib.rs) expects
initialized state. Read [your first program](https://hopperzero.dev/docs/first-five), then follow a
funded example through its account creation, authorization, transfers, and tests.

## One execution stack, from handlers to syscalls

- **Framework:** typed accounts, constraints, value rules on layout fields, dispatch,
  initialization, account closure, migrations, bounded collections, and generated clients.
- **Runtime:** checked account borrows, PDA helpers that also run in plain unit
  tests, CPI validation, and optional policies restricting tracked data and
  lamport writes.
- **Native:** loader-memory parsing, duplicate-account resolution, entrypoints,
  Solana syscalls, a heap allocator that uses a requested heap frame, SlotHashes
  lookup by slot, panics that report file and line, and explicit low-level APIs.

`hopper-native` has no crate dependencies. `hopper-runtime` uses that native
layer directly. Application programs run on Solana's SVM; no off-chain service
is required to authorize their ordinary instructions or execute transfers.
Host tools generate clients, inspect accounts, and collect evidence.

[The execution model](docs/THE_HOPPER_MODEL.md) separates generated admission
checks, handler logic, optional policies, and Solana's runtime enforcement.
Hopper uses Rust and Cargo; its macros do not introduce a separate language or VM.
Scalar handler arguments are decoded values; the unreleased `&MyArgs` integration
borrows a checked wire layout directly. Borrowed account state, borrowed
argument APIs, and dynamic-field codecs each have their own copying behavior.

Programs can [inspect prior calls on chain](docs/INSTRUCTION_INTROSPECTION.md),
including their data, account identities, and instruction privileges, using
caller-owned scratch buffers. Applications keep authorization and transfer
outcome checks explicit.

SOL wallet deposits invoke the System Program. SPL Token balances change through
the appropriate token program. A program may directly debit lamports only from
accounts it owns. Zero-copy account access does not bypass those runtime rules.

## Choose the state contract you need

Headered accounts validate owner, discriminator, version, and layout identity.
Compact accounts use an explicit discriminator and size contract. Bounded
strings and vectors keep variable data controlled; segment borrows let handlers
work with selected fields without copying an entire account.

The unreleased source also provides [composable borrowed argument validation](docs/BORROWED_ARGUMENTS.md):
checked parsers follow aliases, arrays, nested layouts, and present optional
values. Choose exact-length payloads or an explicit borrowed tail, while keeping
instruction arguments in their original buffer.

Optional write policies constrain Hopper-tracked accesses within the program.
They do not sandbox arbitrary downstream programs, create byte-level transaction
parallelism, or guarantee a fee discount. Solana locks writable accounts.

## Built to be audited

Hopper writes down what your program declares and compares it between
releases. `grillo authority-diff old.manifest.json new.manifest.json` fails
when an upgrade drops a signer, makes an account writable, widens a write
range, or loosens a value rule on a stored field. The rules themselves are
one attribute on the field:

```rust
#[check(value >= 1 && value <= 10)]
pub tier: u8,
```

The repository retains the verification archives separately from the library
download. The framework holds itself to the same standard. `audit/UNSAFE_MAP.md`
lists every `unsafe` site in Hopper with the justification written next to
it, the tests that reach it, and a hash of its code, and CI fails when a
site has no reasoning of its own or runs on the host without a test that
reaches it. A review ledger ties each sign-off to the code that was read
and flags anything edited since. `audit/api/` locks the signature of every
public item, and `scripts/api-lock.py --against-published` names the
version the next release needs, including the breaks cargo-semver-checks
does not see. Hopper has not had an independent security audit; the
[self-audit guide](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/SELF_AUDIT.md)
covers what these checks prove and what they do not.

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

Hopper 0.5 includes the native and runtime borrow fixes, token builders and
extension initialization, bounded token batches, and on-chain inspection APIs.
Upgrade all crates exposing native or runtime types together. The macro crates
also use the 0.5 line because their expansions target those APIs. `grillo-*` and
`hopper-topology` use 0.1.1; unchanged `hopper-builtins` remains 0.4.0.

Native instruction inspection passed 11 finalized devnet transactions, and
[token receipt policies](docs/TOKEN_RECEIPTS.md) passed a separate 34-transaction
run. Those dated results and the later token-lab runs document specific fixtures;
they are not a whole-framework security audit. Repository examples are not
published crates. See the [0.4 release record](docs/RELEASE_0_4_VALIDATION.md)
for earlier package lineage.

## Documentation

- [Program architecture](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/ARCHITECTURE.md)
- [Writing handlers and accounts](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/WRITING_HOPPER_PROGRAMS.md)
- [Token escrow](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-escrow/README.md)
- [Governance and treasury programs](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/GOVERNANCE_PROGRAMS.md)
- [Bounded fields](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/BOUNDED_FIELDS.md) and [dynamic tails](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/DYNAMIC_TAILS.md)
- [On-chain write policies](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/ONCHAIN_BYTE_POLICIES.md)
- [Capabilities and integration boundaries](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/PROGRAM_CAPABILITIES.md)
- [Safety and unsafe invariants](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/UNSAFE_INVARIANTS.md)
- [Self-audit](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/SELF_AUDIT.md), the [unsafe map](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/audit/UNSAFE_MAP.md), and the [public API lock](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/api)
- [Token-2022, including confidential transfers](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/TOKEN_2022_GUIDE.md)
- [Moving to 0.5](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/MIGRATION_0_5.md)

Framework code is licensed under MIT OR Apache-2.0 unless a component states
otherwise. See [security reporting](SECURITY.md) before disclosing a vulnerability.
