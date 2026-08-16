# Cicada manifest-fuzz semantic adapter

This non-publish host binary is the strict application adapter for Cicada's
committed manifest-derived fuzz plan. It independently checks every requested
case against `hopper_cicada::PROGRAM_MANIFEST`, then exercises Cicada's actual
private claim, execution, duplicate-meta, mint-delegation, route-bound, and
native/non-native lamport-floor guards through a doc-hidden module that is
absent from `target_os = "solana"` builds.

The adapter returns exactly one result per requested case, refuses unknown or
modified cases and invariants, and never skips. Its evidence is intentionally
labelled **host semantic**, not SBF transaction execution. The compiled SBF
lifecycle suite remains the source of transaction/runtime evidence.

From the workspace root:

```text
cargo build -p hopper-cicada-fuzz-adapter --locked
cargo run -p hopper-cli --locked -- fuzz run \
  --program target/hopper/fuzz/cicada.manifest.json \
  --plan fuzz/plans/hopper-cicada.plan.json \
  --adapter target/debug/hopper-cicada-fuzz-adapter \
  --require-invariant cicada-business-semantics
```
