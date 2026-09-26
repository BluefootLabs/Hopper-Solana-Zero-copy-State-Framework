# Program capabilities

Use Hopper to build full Solana applications with typed Rust handlers and direct
account-state access.

| Capability | Shipped surface | Application responsibility |
|---|---|---|
| Accounts and authorization | Typed wrappers, signer/writable/owner checks, constraints, PDA helpers | Choose authorities, roles, accepted programs, and initialization rules |
| SOL transfers | System CPI builders and checked program-owned lamport transfers | Validate sender, recipient, amount, rent reserve, and permission |
| Token movement | Classic Token and Token-2022 builders, checked transfers, PDA signing | Bind mint, token authority, destination, and supported extensions |
| Token account lifecycle | Account/mint initialization, associated-token helpers, mint/burn/close instructions | Allocate exact space and choose the intended asset policy |
| Escrow | Funded classic-token example with exchange, cancellation, and surplus refunds | Pricing/product rules and supported token types |
| Multisig custody | Bounded member example with actual transaction approvals and SOL transfers | Proposal persistence, timelocks, weighted voting, and recovery if required |
| Delegated spending | SOL treasury with operator, freeze, budget, cooldown, and rent checks | Choose limits and administrator-controlled budget-reset policy |
| Dynamic state | Bounded fields, final tails, sequences, slabs, and other collections | Capacity, migration, lifecycle, and account placement |
| External integrations | Checked generic CPI, generated CPI builders, external account adapters | Implement and validate the target program's ABI and authority policy |
| Client DX | Manifests, IDL exports, generated clients, build/test/deploy/inspect commands | Wallet integration, frontend behavior, and transaction composition |

The framework does not automatically supply a matching engine, a complete
airdrop contract, every asset-protocol adapter, or an audited DAO product.
Those programs can use Hopper's execution primitives, with their own business
rules and tests. The examples show concrete implemented paths and their scope.

Start with [application examples](APPLICATION_USE_CASES.md) and
[the execution architecture](ARCHITECTURE.md).
