# Hopper Roadmap

This file tracks capabilities from the architecture checklist that are **not yet
implemented** in the tree, with the rationale for deferral and the intended
shape of the work. Everything in `COMPARISON.md` marked **No (tracked)** has a
row here. Items that are implemented live in the code and are cited in
`COMPARISON.md`; they are not repeated here.

## Deferred capabilities

### R-1: litesvm-based integration test harness

**Status:** deferred.
**What:** a `hopper-test` crate wrapping `litesvm` (or `mollusk`) so program
authors can spin up an SVM, deploy a Hopper program, build instructions with the
generated client surface, and assert on account bytes / receipts.

**Why deferred:** pulling `litesvm` into the workspace adds a `solana-program` /
`solana-sdk` dependency subtree. That is acceptable for a *test-only* crate, but
it must be isolated so it can never leak onto the runtime hot path or into
`cargo build-sbf` output. Wiring that isolation (separate workspace member,
dev-dependency-only, feature-gated) is a self-contained chunk of work that was
scoped out of the soundness/parity pass to keep this PR focused. Current testing
is unit + integration against the runtime, codec, schema, and macro layers
(1000+ tests), which covers correctness of the framework itself; the harness is
about *program-author ergonomics*, not framework soundness.

**Shape:** new `crates/hopper-test` member, `litesvm` as a normal (not dev-only)
dependency of that crate so downstream test code can use it, with the rest of
the workspace unaffected. Provide `deploy_program`, `with_account`, and
receipt-assertion helpers.

### R-2: `proptest` / `kani` verification layer for layout checks

**Status:** partial / deferred.
**What:** property-based and model-checked proofs that the verified-cast layout
checks (size, alignment, discriminator, version) are sound for all inputs.

**Why deferred:** `proptest`-style coverage already exists in
`crates/hopper-core/tests/property_tests.rs`. A full `kani` harness for the
unsafe casting boundary is higher-effort (toolchain install, harness authoring,
CI integration) and was scoped out. The unsafe surface is currently guarded by
explicit `// SAFETY:` invariants, the `docs/UNSAFE_INVARIANTS.md` inventory, and
trybuild compile-fail fixtures.

**Shape:** add `kani` proof harnesses over `AccountView::load` and the segment
borrow registry's disjointness check; gate behind a `verification` feature and a
dedicated CI lane (kani is not part of the standard `cargo test` run).

### R-3: `cargo build-sbf` CI proof for the runtime crate

**Status:** deferred (toolchain unavailable in this environment).
**What:** CI that compiles the runtime crate and at least one example with
`cargo build-sbf --no-default-features` to prove the no_std / no-heap claim on
the real BPF target.

**Why deferred:** the Solana SBF toolchain (`cargo build-sbf`, `platform-tools`)
is **not installed** in the build environment used for this pass, so the BPF
build could not be exercised here. The host-target proxy for the claim —
`cargo check -p hopper-runtime --no-default-features` — passes clean, and the
runtime's default feature set is empty (`[]`), with no `solana-program` on the
hot path. The remaining work is purely CI plumbing once a `build-sbf`-capable
runner is available.

**Shape:** add a CI job that installs `solana` + `platform-tools` and runs
`cargo build-sbf --manifest-path examples/hopper-counter/Cargo.toml` plus a
size-report step.

### R-4: compile-time disjointness proof for segment borrows

**Status:** partial.
**What:** prove segment borrow non-overlap at *compile time* via const generics
over byte ranges, eliminating the runtime registry check entirely for the
const-offset path.

**Why deferred:** Hopper already provides `segment_ref_typed::<T, const OFFSET>`
(compile-time offset) and a runtime registry that rejects overlaps with a tight
aliasing invariant. A fully compile-time disjointness proof across *multiple*
simultaneous typed segments requires const-generic range arithmetic that stable
Rust does not yet express ergonomically. The runtime registry is sound and
cheap; the compile-time proof is an optimization, not a correctness fix.

**Shape:** revisit when `generic_const_exprs` stabilizes; emit overlap rejection
as a `const { assert!(...) }` over the declared segment table.

## Non-goals (intentionally out of scope)

- **Client SDK breadth.** Hopper is a *program* framework. Client codegen exists
  to make programs usable, but Hopper will not grow into a general-purpose
  TypeScript/web client library. (Brief hard rule: "This is a `program`
  framework, not a client SDK.")
- **Multiple simultaneous backend families.** The runtime rejects more than one
  backend family at once by design; `--all-features` is not a backend matrix.
  See `AUDIT.md` RSK-5.
- **Release-grade benchmark claims in framework docs.** Performance numbers live
  with the benchmark repository's reproducibility envelope, not inline in the
  framework README. See `AUDIT.md` R2/RSK-4 and `BENCHMARKS.md`.
