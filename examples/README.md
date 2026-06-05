# Hopper Examples

These examples teach Hopper in layers. Start with the counter, then move into a
small vault or escrow flow, Token-2022, migration, cross-program interfaces, and
the full showcase once the basics are familiar.

## Learning Order

### Tier 1: Start Here

1. **[hopper-counter](hopper-counter/src/lib.rs)** - The smallest macro-first
   Hopper program: one account, one context, one instruction. Start here when
   you want the framework model before any systems concepts.

2. **[hopper-vault](hopper-vault/src/lib.rs)** - A compact SOL vault. Three
   instructions (init, deposit, withdraw). Good for seeing Hopper's account,
   dispatch, and state basics in one place. Companion guide:
   [hopper-vault/README.md](hopper-vault/README.md)

3. **[hopper-escrow](hopper-escrow/src/lib.rs)** - Token escrow with authority
   checks and SPL Token integration. Companion guide: [hopper-escrow/README.md](hopper-escrow/README.md)

4. **[hopper-token-2022-vault](hopper-token-2022-vault/src/lib.rs)** - Hopper-owned
   Token-2022 vault flow with a local `hopper.manifest.json`, so the CLI can
   infer the package manifest and emit lowered accessors directly from the
   example directory.

5. **[quasar-port-20-min](quasar-port-20-min/src/lib.rs)** - Bounded
   dynamic-tail example for a fixed vault and multisig metadata using
   Quasar-pretty `#[hopper::account]` fields, initialization helpers, and threshold checks.
   Companion guide:
   [../docs/PORT_QUASAR_IN_20_MINUTES.md](../docs/PORT_QUASAR_IN_20_MINUTES.md)

### Tier 2: Advanced Patterns

6. **[hopper-proc-vault](hopper-proc-vault/src/lib.rs)** - Proc-macro vault that
   demonstrates the `#[hopper::account]` / `#[hopper::program]` authoring path.

7. **[hopper-treasury](hopper-treasury/src/lib.rs)** - Multi-segment treasury
   with permissions and budget controls.

8. **[hopper-registry](hopper-registry/src/lib.rs)** - Segmented account with
   journal, virtual state, and named segment lookup.

9. **[hopper-migration](hopper-migration/src/lib.rs)** - V1 to V2 layout
   evolution. Shows how append-only versioning and the migration planner work
   together. Companion guide: [hopper-migration/README.md](hopper-migration/README.md)

10. **[hopper-virtual-state](hopper-virtual-state/src/lib.rs)** - Multi-account
   entities with VirtualState and ShardedAccess. For when your state is too big
   for one account.

11. **[cross-program-read](cross-program-read/)** - Two separate programs reading
   each other's accounts via `hopper_interface!`. No shared on-chain crate
   dependency, plus a host-only devnet runner that proves Program B can validate
   Program A's live deployed owner and layout contract. Companion guide:
   [cross-program-read/README.md](cross-program-read/README.md)

12. **[hopper-policy-vault](hopper-policy-vault/src/lib.rs)** - Three sibling
    programs (`strict`, `sealed`, `raw`) demonstrating the policy-driven runtime
    plus the `#[instruction(N, unsafe_memory, skip_token_checks)]` per-handler
    overrides. Companion guide:
    [hopper-policy-vault/README.md](hopper-policy-vault/README.md)

13. **[hopper-showcase](hopper-showcase/src/lib.rs)** - Full systems-mode tour.
    Uses layout, dispatch, phased frame, policy, receipts, invariants, segment
    roles, and state diffs. Read this after the smaller examples. Companion
    guide: [hopper-showcase/README.md](hopper-showcase/README.md)

14. **[hopper-devnet-audit](hopper-devnet-audit/src/lib.rs)** - Deployable
   devnet audit program for dynamic tails, typed contexts, segment mutation,
   remaining-account parsing, and substrate CU probes. Companion guide:
   [hopper-devnet-audit/README.md](hopper-devnet-audit/README.md)

15. **[hopper-external-oracle](hopper-external-oracle/src/lib.rs)** - Known
   non-Hopper oracle bytes through `ExternalAccount<T>`, typed zero-copy views,
   checked lenses, snapshot hashes, and normal Hopper-owned state updates.

### Tier 3: Escape Hatch

Every example uses the standard Hopper path. When you need to go lower, the
framework provides `load_unchecked`, raw `overlay_mut`, manual `write_header`,
and `segment_data_mut_unchecked`. These are documented in the API but not
demonstrated in a dedicated example because the whole point is that you
should not need them often.
