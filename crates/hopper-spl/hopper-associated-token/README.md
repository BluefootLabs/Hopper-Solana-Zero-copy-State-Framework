# hopper-associated-token

Hopper-owned Associated Token Account (ATA) builders. `Create`,
`CreateIdempotent`, `RecoverNested`. ATA derivation helpers re-exported from
the runtime.

[![Crates.io](https://img.shields.io/crates/v/hopper-associated-token.svg)](https://crates.io/crates/hopper-associated-token)
[![Docs.rs](https://img.shields.io/docsrs/hopper-associated-token)](https://docs.rs/hopper-associated-token)

Part of the **[Hopper](https://hopperzero.dev)** framework.

```rust
use hopper::prelude::*;

hopper_associated_token::instructions::CreateIdempotent {
    payer,
    associated_token,
    owner,
    mint,
    system_program,
    token_program,
}
.invoke()?;
```

Works against both legacy SPL Token and Token-2022 mints; pass the
`token_program` account that matches the mint. License: Apache-2.0.
