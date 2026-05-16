# hopper-vesting

Vesting schedule math for Hopper programs: linear schedules with cliffs,
stepped unlocks, elapsed-step calculation, and safe claimable amounts. Pure
functions, `no_std`, `no_alloc`, and BPF-safe.

Part of the **[Hopper](https://hopperzero.dev)** framework.

The crate does not own schedule storage. Your account layout stores the terms;
these helpers compute how much is vested and how much is still claimable at a
given timestamp.

```rust
use hopper_vesting::{claimable, vested_amount};

let vested = vested_amount(total, start, cliff, end, now);
let to_send = claimable(vested, already_claimed);
```

Docs: <https://docs.rs/crate/hopper-vesting/0.2.0>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
