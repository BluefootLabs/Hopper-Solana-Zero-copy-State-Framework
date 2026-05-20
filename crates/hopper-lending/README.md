# hopper-lending

Lending protocol math for Hopper programs: collateral ratios, health checks,
liquidation limits, seize amounts, utilization, and simple interest. Pure
functions, `no_std`, `no_alloc`, and BPF-safe.

Part of the **[Hopper](https://hopperzero.dev)** framework.

Pass the current collateral value, debt value, and protocol thresholds. The
crate returns basis-point ratios or rejects the position with a concrete
`ProgramError`. There is no hidden state and no heap allocation.

```rust
use hopper_lending::{
    check_healthy,
    liquidation_seize_amount,
    max_liquidation_amount,
};

check_healthy(collateral_value, debt_value, liquidation_threshold_bps)?;
let max_repay = max_liquidation_amount(debt_value, close_factor_bps)?;
let seized = liquidation_seize_amount(max_repay, bonus_bps)?;
```

Docs: <https://docs.rs/crate/hopper-lending/0.2.1>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
