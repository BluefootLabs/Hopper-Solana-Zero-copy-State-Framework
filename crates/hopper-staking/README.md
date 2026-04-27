# hopper-staking

Staking reward accumulators for Hopper: MasterChef-style
reward-per-token, emission rates, reward debt tracking. Zero-copy,
no_std, no_alloc, BPF-safe.

Part of the **[Hopper](https://hopperzero.dev)** framework.

The classic reward-per-share accumulator pattern, expressed in checked
fixed-point math. Pool state and per-user reward debt update in O(1) on
each stake/unstake/claim, with no iteration over depositors.

```rust
use hopper_staking::{accrue, pending_reward};

accrue(&mut pool, now, total_staked)?;
let pending = pending_reward(&pool, &user)?;
```

License: Apache-2.0.
