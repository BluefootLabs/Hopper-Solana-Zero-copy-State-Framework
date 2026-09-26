# Hopper token escrow

An all-or-nothing exchange of two classic SPL tokens, implemented with Hopper
accounts, canonical PDA validation, and checked token CPIs.

- **Make:** create the 192-byte escrow state and a 165-byte token vault, then
  transfer the maker's offered tokens into custody.
- **Take:** verify the expected quote, pay the maker, release the offered tokens
  to the taker, refund any excess vault tokens to the maker, and close both
  vault and escrow. Any failure rolls back the entire instruction.
- **Cancel:** only the maker can refund all vault tokens and close the offer.

Both accounts' rent returns to the maker. Anyone may take an offer with the
correct payment; the maker cannot change its quote after creation. No client,
indexer, receipt service, or off-chain authorization is needed to enforce it.

## Token policy

This example deliberately accepts classic SPL Token only. Mints must be
initialized and have no freeze authority. Token accounts must have the expected
mint and owner, be initialized and unfrozen, and not represent wrapped SOL.
The custody vault must have neither a delegate nor a separate close authority.
Token-2022, transfer fees/hooks, native SOL, partial fills, expiration, and
multisig token owners are not implemented here. Other Hopper token APIs are
separate from this example's policy.

User token accounts must already exist. Make creates a fresh token vault from a
new keypair; its **token authority** is the canonical PDA derived from
`[b"escrow-vault", escrow_address]`. The vault keypair cannot authorize spending
its tokens after initialization. The vault is not an ATA. A prefunded vault
address is rejected by System CreateAccount; choose a fresh vault keypair.

Take requires the stored maker payment account. Refunds may go to any validated
maker-owned account for the offered mint, allowing a replacement refund account
if the original source was closed. Keep the maker payment account available
until settlement or cancel the offer. Unsolicited vault deposits return to the
maker; they do not change the taker's quote.

## Account and instruction ABI

Data starts with a one-byte discriminator. Make and Take then contain
`amount_offered: u64 LE` and `amount_wanted: u64 LE` (17 bytes total). Cancel is
exactly one byte. Take's amounts are an expected quote, not a partial-fill amount.

| Tag | Accounts in order (`w` writable, `s` signer) |
|---|---|
| 0 Make | maker(ws), escrow(ws), vault_authority, vault(ws), mint_a, mint_b, maker_source(w), maker_receive, token_program, system_program |
| 1 Take | taker(s), escrow(w), maker(w), vault_authority, vault(w), mint_a, mint_b, taker_receive(w), taker_source(w), maker_receive(w), maker_refund(w), token_program |
| 2 Cancel | maker(ws), escrow(w), vault_authority, vault(w), mint_a, maker_refund(w), token_program |

All application account roles must be distinct. The bounded generated
entrypoint can ignore surplus account metas; the payload lengths above are
exact. The source-generated `hopper.manifest.json` describes this ABI.

State discriminator remains 2, but **layout version is now 2**. The old
161-byte state-only example is incompatible. Deploy this version to a fresh
program ID; historical deployments and state-only test receipts do not validate
the funded implementation. The obsolete state-only devnet test was replaced by
the token custody runner below.

## Verify

Build the actual program and execute it against the canonical SPL Token ELF:

```bash
cargo build-sbf --manifest-path examples/hopper-escrow/Cargo.toml -- --locked
HOPPER_ESCROW_SBF=target/deploy/hopper_escrow.so \
  cargo test --manifest-path bench/framework-comparison/verifier/Cargo.toml \
  --test token_escrow_sbf --locked -- --ignored --nocapture
```

The compiled suites check exact complete account states for make, take, cancel,
and excess deposits. Refusals cover signer/writable privileges, account links,
mint/owner/state, custody delegation, wrong programs, stale quotes, malformed
payloads, insufficient funds, reinitialization, and repeated settlement.
A synthetic rent-return overflow proves full rollback after successful token
transfers and token-vault closure. This fault injection is a local VM test, not
a claim of a naturally occurring devnet condition.

After deploying the exact ELF to devnet, with a clean committed checkout:

```bash
python scripts/test-token-escrow-devnet.py \
  --program PROGRAM_ID --payer /path/to/devnet-payer.json \
  --hopper /path/to/hopper --elf target/deploy/hopper_escrow.so \
  --header-hex HEADER_PRINTED_BY_COMPILED_TEST \
  --out target/hopper/token-escrow-devnet
```

The runner verifies devnet genesis, compares deployed ELF bytes before and after,
creates real mints and token accounts, and checks finalized full account
snapshots after each transaction, including expected failures. Private keypairs
remain under its ignored output directory. This example has not received an
independent security audit.
