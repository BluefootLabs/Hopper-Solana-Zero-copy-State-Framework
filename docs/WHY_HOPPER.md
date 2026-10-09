# Why Hopper

Hopper is a zero-copy framework for Solana programs that hold funds, move
tokens, and enforce application rules on chain.

## Start with a real action

Build an escrow that settles a trade, a vault that makes authorized payments,
or a treasury that limits delegated spending. The examples include account
creation, token or SOL movement, cancellation, and failure checks. Claims,
order matching, and NFT integrations still need your application's rules.

## Keep control as the program grows

- **Read state in place.** Supported fixed layouts and bounded tails borrow
  account bytes directly. Application code and other formats may still copy
  data or allocate; zero-copy is an account-access model.
- **Declare account checks.** Typed contexts generate the signer, owner,
  layout, and PDA checks you request. Checked CPIs reject conflicting borrows.
  These checks support your authorization logic; they do not invent it.
- **Use one execution stack.** Framework handlers and lower-level account,
  CPI, and syscall APIs share Hopper's runtime. Adopting the raw layer requires
  explicit validation and workload testing.
- **Enforce limits on chain.** Optional write policies and payout checks let
  programs constrain mutations and verify the tokens a recipient received.
  Their documented coverage and exclusions are part of the contract.

## Check the result

The next release connects borrowed argument layouts to ordinary Rust handlers:
declare `#[hopper::args]` once and accept `&MyArgs`. The same declaration supports
manual parsing, generated dispatch, nested value checks, and fixed wire-size
metadata. Large fixed payloads stay borrowed instead of becoming owned argument
structs. The [argument guide](BORROWED_ARGUMENTS.md) describes the syntax and
boundaries. This feature is unreleased; measure the complete instruction before
claiming a compute advantage.

The classic-token escrow at commit `847a0b0` completed 33 finalized devnet
transactions on September 30, 2026, including expected refusals. Taking an
offer, refunding surplus, and closing accounts cost 7,836 CU in that fixture.
These are application measurements, not a cost guarantee for another program.
The [transactions and state checks](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/devnet-evidence-2026-09-30/escrow-round12)
identify the source and deployed binary.

Hopper 0.5.0 is published. Use the [migration guide](MIGRATION_0_5.md) when
upgrading and [release readiness](RELEASE_READINESS.md) for the explicitly
unreleased additions and outstanding review requirements.

## Build

- [Your first program](FIRST_FIVE_MINUTES.md)
- [Token escrow](../examples/hopper-escrow/README.md)
- [SOL vault](../examples/hopper-vault/README.md)
- [Governance and treasury programs](GOVERNANCE_PROGRAMS.md)
- [Lower-level programs](FROM_PINOCCHIO.md)
