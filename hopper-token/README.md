# hopper-token

Hopper-owned SPL Token builders. `Transfer`, `MintTo`, `Burn`, `CloseAccount`,
`Approve`, `Revoke`, `InitializeAccount`. Stack-allocated instruction data,
no heap.

[![Crates.io](https://img.shields.io/crates/v/hopper-token.svg)](https://crates.io/crates/hopper-token)
[![Docs.rs](https://img.shields.io/docsrs/hopper-token)](https://docs.rs/hopper-token)

Part of the **[Hopper](https://hopperzero.dev)** framework.

```rust
use hopper::prelude::*;

hopper_token::instructions::Transfer {
    source,
    destination,
    authority,
    amount,
}
.invoke()?;
```

For Token-2022 mints with extension awareness, see
[`hopper-token-2022`](../hopper-token-2022). License: Apache-2.0.
