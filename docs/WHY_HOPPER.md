# Why build with Hopper?

Build a Solana program that owns assets and performs useful on-chain actions,
with a clear account contract and ordinary Rust handlers.

- **Keep state access predictable.** Typed zero-copy views read and update account
  memory directly. Choose headered, compact, bounded, or explicitly segmented layouts.
- **Compose with Solana programs.** System and token builders perform real CPIs;
  generic checked CPI supports application integrations and PDA authorities.
- **Make permissions visible.** Declare account constraints and enforce business
  rules in handlers. Optional tracked-write policies narrow which state may change.
- **Keep clients aligned.** Export the account/instruction contract for generated
  clients, inspection, and upgrade review.
- **Use the level of control you need.** Start with typed accounts. Native
  entrypoints, syscalls, and explicit unsafe APIs remain available for reviewed
  low-level work.

Try [funded token escrow](https://hopperzero.dev/docs/token-escrow), a
[SOL vault](../examples/hopper-vault), or
[governance and treasury programs](GOVERNANCE_PROGRAMS.md). Their tests check
balances and refusal paths as well as state fields.

Performance depends on the complete instruction and its validation contract.
Use the [dated program measurements](../BENCHMARKS.md) to inspect the workload,
then measure the actual program you intend to deploy.
