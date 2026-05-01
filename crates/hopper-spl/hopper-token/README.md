# hopper-token

Hopper-owned SPL Token builders. The default public API is safety-first:
`TransferChecked`, `MintToChecked`, `BurnChecked`, `ApproveChecked`, plus
`CloseAccount`, `Revoke`, and `InitializeAccount`. Stack-allocated instruction
data, no heap.

[![Crates.io](https://img.shields.io/crates/v/hopper-token.svg)](https://crates.io/crates/hopper-token)
[![Docs.rs](https://img.shields.io/docsrs/hopper-token)](https://docs.rs/hopper-token)

Part of the **[Hopper](https://hopperzero.dev)** framework.

```rust
use hopper::prelude::*;

hopper_token::instructions::TransferChecked {
    from,
    mint,
    to,
    authority,
    amount,
    decimals,
}
.invoke()?;
```

Deprecated plain builders (`Transfer`, `MintTo`, `Burn`, `Approve`) are hidden
unless the non-default `legacy-token-instructions` feature is enabled. Use that
feature only for migration tests against legacy SPL Token programs that cannot
use the checked instructions.

For Token-2022 mints with extension awareness, see
[`hopper-token-2022`](../hopper-token-2022). License: Apache-2.0.
