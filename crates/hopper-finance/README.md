# hopper-finance

DeFi math primitives for Hopper programs: constant-product swaps, LP math,
slippage checks, and price bounds. Pure functions, `no_std`, `no_alloc`, and
BPF-safe.

Part of the **[Hopper](https://hopperzero.dev)** framework.

Every public helper uses checked `u128` intermediates and returns an explicit
`ProgramError` on overflow or invalid bounds. Use it when the protocol logic
needs predictable math without pulling in an allocator.

```rust
use hopper_finance::{check_slippage, constant_product_out};

let amount_out = constant_product_out(
    amount_in,
    reserve_in,
    reserve_out,
    fee_bps,
)?;
check_slippage(amount_out, minimum_out)?;
```

Docs: <https://docs.rs/crate/hopper-finance/0.1.0>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
