# hopper-lending

Lending protocol primitives for Hopper: collateralization ratios, health
checks, liquidation math, interest calculations. Zero-copy, no_std,
no_alloc, BPF-safe.

[![Crates.io](https://img.shields.io/crates/v/hopper-lending.svg)](https://crates.io/crates/hopper-lending)
[![Docs.rs](https://img.shields.io/docsrs/hopper-lending)](https://docs.rs/hopper-lending)

Part of the **[Hopper](https://hopperzero.dev)** framework.

Designed to drop in beside a Hopper account layout: hand it the
collateral value, debt value, and liquidation threshold and it returns
the health factor, max-borrow capacity, and the seize amount for a
liquidation event. All math is checked.

```rust
use hopper_lending::{health_factor, max_seize};

let hf = health_factor(collateral_value, debt_value, ltv_bps)?;
if hf < HEALTHY {
    let seize = max_seize(debt_repay, price_collateral, bonus_bps)?;
}
```

License: Apache-2.0.
