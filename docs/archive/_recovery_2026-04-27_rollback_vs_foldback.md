# Hopper recovery note - rollback vs fold-back

Date: 2026-04-27

## Current verified state

- Main repo: `d:\tmp\Hopper-Solana-Zero-copy-State-Framework`
- Current branch: `main`
- Current HEAD: `610ec5c Integrate hopper-memo crate and enhance IDL emission with constant support`
- Working tree: clean
- Pre-split commit: `a2b1965 Hopper Native enhancements: wire operator overloads, static-syscalls flag, dual license, baseline scripts`
- Split commit: `9b0efc5 Extract hopper-runtime, hopper-core, hopper-macros, hopper-derive, hopper-spl, hopper-cli, hopper-bench to BluefootLabs sister repos`
- Split commit deleted the framework-internal crates from main:
  - `crates/hopper-runtime`
  - `crates/hopper-core`
  - `crates/hopper-macros`
  - `crates/hopper-macros-proc`
  - `crates/hopper-token`
  - `crates/hopper-token-2022`
  - `crates/hopper-associated-token`
  - `crates/hopper-metaplex`
  - `tools/hopper-cli`
  - `bench/*`

## Fresh safety backups created

Created fresh Git bundles for current main + all sister repos before any destructive action:

- `d:\tmp\hopper-recovery-20260427-144300\Hopper-Solana-Zero-copy-State-Framework.bundle`
- `d:\tmp\hopper-recovery-20260427-144300\hopper-runtime.bundle`
- `d:\tmp\hopper-recovery-20260427-144300\hopper-core.bundle`
- `d:\tmp\hopper-recovery-20260427-144300\hopper-macros.bundle`
- `d:\tmp\hopper-recovery-20260427-144300\hopper-derive.bundle`
- `d:\tmp\hopper-recovery-20260427-144300\hopper-spl.bundle`
- `d:\tmp\hopper-recovery-20260427-144300\hopper-cli.bundle`
- `d:\tmp\hopper-recovery-20260427-144300\hopper-bench.bundle`

Created main-repo safety refs:

- `safety/current-main-before-recovery-20260427-144300` -> current `HEAD`
- `recovery/pre-split-a2b1965-20260427-144300` -> pre-split commit `a2b1965`

## Answer: can we roll back and get everything back?

Yes, the old in-main framework tree is still in Git at `a2b1965`.

But a plain `reset --hard a2b1965` is not the right final move because it would discard newer work made after the split:

- `hopper-memo` integration in main
- Anchor IDL constant support in main/schema tests
- `#[hopper::constant]` proc macro work in `hopper-derive`
- Metaplex context/macro work in `hopper-derive` and `hopper-spl`
- `hopper-cli lint` Metaplex + zero-allocation checks
- standalone sister-repo workspace fixes

So the safe recovery is **not pure rollback**. It is:

1. Start from a recovery branch, not by rewriting `main` immediately.
2. Restore the pre-split in-main tree from `a2b1965`.
3. Overlay or subtree-fold newer sister repo work back into the main repo.
4. Preserve the new main-only commits (`hopper-memo`, constants IDL/tests).
5. Validate before touching remote `main`.

## Recommended path

### Option 1 - safest history-preserving fold-back (recommended)

Use current `main` as the base and fold sister repos back into it with `git subtree add` under temporary `_foldback/*` paths, then reconcile.

Pros:

- Preserves current main history.
- Preserves sister repo commit history and authorship.
- Does not force-push until fully validated.
- Does not lose post-split fixes.

Cons:

- Requires reconciliation work because old paths were deleted from main.

High-level flow:

1. Ensure clean tree.
2. Create branch: `recovery/fold-back-framework-internals`.
3. Add sister repos as temporary subtrees:
   - `_foldback/runtime`
   - `_foldback/core`
   - `_foldback/macros`
   - `_foldback/derive`
   - `_foldback/spl`
   - `_foldback/cli`
4. Compare `_foldback/*` to old paths from `a2b1965`.
5. Move canonical winners into final framework layout.
6. Rewrite path deps/workspace deps away from Git URLs.
7. Run targeted tests.

### Option 2 - rollback branch + cherry-pick/overlay

Start a branch at `a2b1965`, then apply newer main and sister work.

Pros:

- Instantly gets old tree back.
- Easier to reason about if the desired final tree is close to pre-split.

Cons:

- Sister repo commit history is not naturally included unless subtrees are added afterward.
- Cherry-picks may conflict because post-split main expects git dependencies rather than local crates.

High-level flow:

1. Checkout branch from `a2b1965`.
2. Cherry-pick only wanted main commits (`hopper-memo`, constants IDL).
3. Overlay newer sister repo directories over restored old directories.
4. Fix workspace membership and path dependencies.
5. Validate.

### Option 3 - direct hard reset (not recommended)

`git reset --hard a2b1965` would restore the old in-main tree immediately, but it loses post-split main commits and does nothing to bring in newer sister repo fixes. Only use this if the current main branch can be thrown away and rebuilt manually.

## Architecture recommendation after live Quasar verification

Blueshift's pattern is one repo per coherent product, not one repo total.

Quasar framework repo contains internal crates in one framework product repo:

- `lang`
- `derive`
- `spl`
- `metadata`
- `idl`
- `schema`
- `profile`
- `cli`

Standalone Blueshift product repos include:

- `quasar-svm` - independent SVM/test engine, separately packaged
- `beethoven` - CPI router product
- `doppler` - oracle product
- `zeropod` / `wincode` - independent reusable libraries
- toolchain/docs repos

Mapping to Hopper:

- Fold framework-internal crates back into the main Hopper framework repo:
  - `hopper-runtime`
  - `hopper-core`
  - `hopper-macros`
  - `hopper-derive`
  - `hopper-spl`
  - `hopper-cli`
- Keep `hopper-bench` separate for now as a cross-framework benchmark product.
- `hopper-svm` should eventually be a standalone product only when it is independently packaged/released like `quasar-svm`; until then it can remain in the framework workspace.
- `hopper-manager` should become standalone only when it is a real operator application with its own release surface.
- Domain crates (`finance`, `lending`, `staking`, `vesting`, etc.) can remain in main until they prove independent demand/release cadence.

## Recommended immediate next action

Do not reset `main` directly.

Create a recovery branch and implement Option 1. Only after validation should `main` be moved or a PR merged.
