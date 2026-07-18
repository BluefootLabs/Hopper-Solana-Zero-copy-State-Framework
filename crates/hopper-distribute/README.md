# hopper-distribute

Dust-safe proportional splits and fee extraction for Hopper programs.
Largest-remainder distribution, basis-point fees, flat fees, and exact
conservation. Pure functions, `no_std`, `no_alloc`, and BPF-safe.

Part of the **[Hopper](https://hopperzero.dev)** framework.

The math is integer-only: split outputs always sum to the input, and fee
extraction always returns `(net, fee)` where `net + fee == amount`.

```rust
use hopper_distribute::{extract_fee, proportional_split};

let shares = [10_u64, 30, 60];
let mut out = [0_u64; 3];
proportional_split(1_000, &shares, &mut out)?;
// out == [100, 300, 600], sum == 1_000

let (net, fee) = extract_fee(1_000_000, 30, 0)?; // 30 bps == 0.30%
```

Docs: <https://docs.rs/crate/hopper-distribute/0.3.0>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
