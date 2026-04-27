# hopper-finance

DeFi math primitives for Hopper: AMM constant-product math, slippage
guards, economic bounds. Zero-copy, no_std, no_alloc, BPF-safe.

Part of the **[Hopper](https://hopperzero.dev)** framework.

Pure functions over `u64` / `u128` with checked arithmetic and explicit
overflow returns. Useful for AMMs, order routers, and any program that
needs to price an `x*y = k` swap without pulling in a heap allocator.

```rust
use hopper_finance::amm;

let amount_out = amm::constant_product_out(
    amount_in,
    reserve_in,
    reserve_out,
    fee_bps,
)?;
amm::enforce_min_out(amount_out, min_out_slippage)?;
```

License: Apache-2.0.
