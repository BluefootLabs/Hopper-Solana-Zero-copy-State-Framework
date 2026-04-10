# Hopper Examples

These examples teach Hopper in order. Start with showcase, move to vault for
something simpler, then explore the advanced patterns as you need them.

## Learning Order

### Tier 1: Start Here

1. **[hopper-showcase](hopper-showcase/src/lib.rs)** - The canonical Hopper
   program. Uses every layer of the pipeline: layout, dispatch, phased frame,
   policy, receipts, invariants, segment roles, state diffs. Read this first.

2. **[hopper-vault](hopper-vault/src/lib.rs)** - A simple SOL vault. Three
   instructions (init, deposit, withdraw). Good for seeing the basics without
   the advanced stuff.

3. **[hopper-escrow](hopper-escrow/src/lib.rs)** - Token escrow with authority
   checks and SPL Token integration.

### Tier 2: Advanced Patterns

4. **[hopper-treasury](hopper-treasury/src/lib.rs)** - Multi-segment treasury
   with permissions and budget controls.

5. **[hopper-registry](hopper-registry/src/lib.rs)** - Segmented account with
   journal, virtual state, and named segment lookup.

6. **[hopper-migration](hopper-migration/src/lib.rs)** - V1 to V2 layout
   evolution. Shows how append-only versioning and the migration planner work
   together.

7. **[hopper-virtual-state](hopper-virtual-state/src/lib.rs)** - Multi-account
   entities with VirtualState and ShardedAccess. For when your state is too big
   for one account.

8. **[cross-program-read](cross-program-read/)** - Two separate programs reading
   each other's accounts via `hopper_interface!`. No shared crate dependency.

### Tier 3: Escape Hatch

Every example uses the standard Hopper path. When you need to go lower, the
framework provides `load_unchecked`, raw `overlay_mut`, manual `write_header`,
and `segment_data_mut_unchecked`. These are documented in the API but not
demonstrated in a dedicated example because the whole point is that you
should not need them often.
