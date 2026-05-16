# hopper-solana

Solana integration layer for the Hopper zero-copy state framework.

Part of the **[Hopper](https://hopperzero.dev)** framework.

Everything that touches Solana-specific primitives lives here: SPL Token reads,
CPI guards, authority rotation, oracle helpers, ATA utilities, and transaction
introspection. The core framework stays chain-agnostic; this crate carries the
Solana surface.

`no_std`, `no_alloc`.

## What's in here

- **Token and mint readers** - Zero-copy SPL Token and Mint parsing.
- **Token-2022 screening** - Extension detection and risk screening for freeze authority, transfer fees, permanent delegates, and related surfaces.
- **CPI guards** - Detect CPI invocation, flash-loan brackets, and subsequent calls.
- **Typed CPI** - CPI helpers with typed account wrappers.
- **Authority rotation** - Two-step authority transfer primitives.
- **Balance guards** - Lamport conservation checks across instruction execution.
- **Compute monitoring** - Remaining compute budget tracking.
- **Oracle and TWAP helpers** - Pyth price feed readers and TWAP math.
- **Crypto helpers** - Ed25519 signature checks and Merkle proof validation.
- **ATA utilities** - Associated Token Account address derivation.
- **Transaction introspection** - Signer detection and remaining account iteration.

## Quick example

```rust
use hopper_solana::{token_account_amount, token_account_mint, assert_no_cpi};

// Zero-copy token account read
let amount = token_account_amount(account_data)?;
let mint = token_account_mint(account_data)?;

// CPI guard (pass the Instructions sysvar account)
assert_no_cpi(sysvar_account, &program_id)?;
```

Docs: <https://docs.rs/crate/hopper-solana/0.2.0>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

Apache-2.0
