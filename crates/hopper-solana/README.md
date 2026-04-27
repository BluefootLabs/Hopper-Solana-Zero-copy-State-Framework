# hopper-solana

Solana integration layer for the Hopper zero-copy state framework.

Part of the **[Hopper](https://hopperzero.dev)** framework.

Everything that touches Solana-specific primitives lives here: SPL Token reads,
CPI guards, authority rotation, oracle helpers, and more. Keeps the core
framework chain-agnostic while giving you production-ready Solana tooling.

`no_std`, `no_alloc`.

## What's in here

- **Token/Mint readers** - Zero-copy SPL Token and Mint account parsing. No deserialization, just overlay the bytes
- **Token-2022 screening** - Extension detection and risk screening for Token-2022 mints (freeze authority, transfer fee, permanent delegate, etc.)
- **CPI guards** - Detect CPI invocation, flash loan brackets, and subsequent calls. Protect your program from being composed in ways you didn't intend
- **Typed CPI** - CPI helpers with typed account wrappers
- **Authority rotation** - Two-step authority transfer primitives (propose + accept)
- **Balance guards** - Lamport conservation checks across instruction execution
- **Compute monitoring** - Track remaining compute budget
- **Oracle/TWAP** - Pyth oracle price feed readers and TWAP helpers
- **Crypto** - Ed25519 signature verification and Merkle proof validation
- **ATA utilities** - Associated Token Account address derivation
- **Transaction introspection** - Signer detection, remaining account iteration

## Quick example

```rust
use hopper_solana::{token_account_amount, token_account_mint, assert_no_cpi};

// Zero-copy token account read
let amount = token_account_amount(account_data)?;
let mint = token_account_mint(account_data)?;

// CPI guard (pass the Instructions sysvar account)
assert_no_cpi(sysvar_account, &program_id)?;
```

## License

Apache-2.0
