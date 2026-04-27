# hopper-distribute

Dust-safe proportional distribution and fee extraction for Hopper.
Largest-remainder splitting, basis-point and flat fees, no leftover lamports.

Part of the **[Hopper](https://hopperzero.dev)** framework.

The math is integer-only and conservation-preserving: the sum of the parts
always equals the input, so no dust accrues to (or is stolen from) any
account across repeated splits.

```rust
use hopper_distribute::{split_proportional, fee_bps};

let shares = [10_u64, 30, 60];
let mut out = [0_u64; 3];
split_proportional(1_000, &shares, &mut out)?;
// out == [100, 300, 600], sum == 1_000

let (fee, net) = fee_bps(1_000_000, 30)?; // 30 bps == 0.30%
```

License: Apache-2.0.
