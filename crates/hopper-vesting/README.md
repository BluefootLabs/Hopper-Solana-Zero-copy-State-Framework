# hopper-vesting

Token vesting schedule calculations for Hopper. Linear with cliff,
stepped/periodic unlocks, safe claimable amounts.

[![Crates.io](https://img.shields.io/crates/v/hopper-vesting.svg)](https://crates.io/crates/hopper-vesting)
[![Docs.rs](https://img.shields.io/docsrs/hopper-vesting)](https://docs.rs/hopper-vesting)

Part of the **[Hopper](https://hopperzero.dev)** framework.

Pure functions that take a vesting schedule and a wall-clock timestamp
and return the currently-claimable amount. Conservation-preserving: the
total ever returned over the life of a schedule equals the grant, even
across rounding boundaries.

```rust
use hopper_vesting::{Schedule, claimable};

let s = Schedule::linear_with_cliff(total, start, cliff, duration);
let unlocked = claimable(&s, now)?;
let to_send = unlocked.saturating_sub(already_claimed);
```

License: Apache-2.0.
