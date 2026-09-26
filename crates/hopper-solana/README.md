# hopper-solana

Solana integration layer for the Hopper zero-copy program framework.

Part of the **[Hopper](https://hopperzero.dev)** framework.

This crate groups higher-level Solana integrations: SPL Token reads, CPI
guards, authority rotation, oracle helpers, ATA utilities, and transaction
introspection. Lower-level loader, syscall, CPI, System Program, and SPL
builder surfaces live in Hopper's runtime and dedicated helper crates.

`no_std`, `no_alloc`.

## What's in here

- **Token and mint readers** - Zero-copy SPL Token and Mint parsing.
- **Token-2022 screening** - Extension detection and risk screening for freeze authority, transfer fees, permanent delegates, and related surfaces.
- **CPI guards** - Reject CPI invocation and check token-program ownership.
- **Typed CPI** - System Program and SPL Token CPI helper functions.
- **Authority rotation** - Two-step authority transfer primitives.
- **Transfer receipts** - Bind token accounts to an exact debit and minimum receipt policy, then verify after CPI.
- **Balance guards** - Explicit token and lamport delta checks.
- **Compute monitoring** - Remaining-compute helpers are opt-in on chain and require a supported target cluster.
- **Oracle and TWAP helpers** - Pyth price feed readers and TWAP math.
- **Crypto helpers** - Ed25519 and secp256k1 precompile checks plus Merkle proof validation.
- **ATA utilities** - Associated Token Account address derivation.
- **Transaction introspection** - Instructions-sysvar parsing for program IDs, instruction data, account keys, caller, top-level, and subsequent-invocation checks, and flash-loan bracket detection.

## Quick example

For escrow releases, claims, and treasury payouts, use a snapshot around your
chosen token CPI:

```rust,ignore
use hopper_solana::transfer::TokenTransferSnapshot;

let snapshot = TokenTransferSnapshot::capture(
    source, destination, &configured_mint, amount, minimum_received,
)?;
// Invoke the validated classic SPL Token or Token-2022 transfer here.
let outcome = snapshot.verify()?;
// Account for outcome.credited in raw mint units.
```

An exact receipt uses `minimum_received == amount`. A lower minimum explicitly
allows a fee. The snapshot rechecks the same accounts, mint, program owner,
initialized base state, and token authorities without holding a data borrow
across CPI. It does not authorize a payout or screen extensions. Propagate
verification errors so the transaction rolls back; catching an error does not
undo the CPI. [Transfer guide](https://hopperzero.dev/docs/token-receipts).

Other integration helpers:

```rust
use hopper_solana::cpi_guard::assert_no_cpi;
use hopper_solana::crypto::ed25519::check_ed25519_signature_at;
use hopper_solana::crypto::secp256k1::check_secp256k1_instruction_at;
use hopper_solana::token::{token_account_amount, token_account_mint};

// Zero-copy token account read
let amount = token_account_amount(account_data)?;
let mint = token_account_mint(account_data)?;

// CPI guard (pass the Instructions sysvar account)
assert_no_cpi(sysvar_account, &program_id)?;

// Native precompile payload checks (pass raw Instructions sysvar data)
check_ed25519_signature_at(instructions, ed_ix, 0, signer, message)?;
check_secp256k1_instruction_at(instructions, secp_ix, 0, eth_address, message)?;
```

The full shipped/planned crypto matrix lives in
[`docs/CRYPTO_CAPABILITIES.md`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/CRYPTO_CAPABILITIES.md).

Docs: <https://docs.rs/crate/hopper-solana>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

Apache-2.0
