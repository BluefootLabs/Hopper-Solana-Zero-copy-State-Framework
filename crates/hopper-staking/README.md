# hopper-staking

Reward-per-token staking math for Hopper programs. Accumulators, emission
rates, pending rewards, and reward debt are all expressed as pure `no_std`,
`no_alloc`, BPF-safe helpers.

Part of the **[Hopper](https://hopperzero.dev)** framework.

This is the standard accumulator pattern with `u128` precision and checked
updates. Pool state and per-user debt update in O(1), so stake, unstake, and
claim handlers never iterate over depositors.

```rust
use hopper_staking::{pending_rewards, update_reward_debt, update_reward_per_token};

pool.reward_per_token = update_reward_per_token(
	pool.reward_per_token,
	rewards_since_last,
	pool.total_staked,
)?;
let pending = pending_rewards(user.staked, pool.reward_per_token, user.reward_debt)?;
user.reward_debt = update_reward_debt(user.staked, pool.reward_per_token);
```

Docs: <https://docs.rs/crate/hopper-staking/0.1.0>

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
