# Hopper Roadmap

This file tracks capabilities from the architecture checklist that are **not yet
implemented** in the tree, with the rationale for deferral and the intended
shape of the work. Everything in `COMPARISON.md` marked **No (tracked)** has a
row here. Items that are implemented live in the code and are cited in
`COMPARISON.md`; they are not repeated here.

## Completed in the devnet pass

These were tracked here as deferred and are now shipped:

- **In-process SVM integration test harness (was R-1).** `crates/hopper-test`
  exposes `LiteSvmHarness`, backed by `mollusk-svm` (already in-tree; `litesvm`
  is not vendored, so the established mollusk harness is the workspace choice).
  It is its own workspace member with `solana-*` as ordinary deps of that
  test-only crate, so nothing leaks onto the runtime hot path or into
  `cargo build-sbf` output. The gated devnet integration tests for escrow,
  versioned-state, and orderbook exercise the deployed programs directly.
- **Segment-disjointness `proptest` (was R-2, proptest half).** A randomized
  oracle in `crates/hopper-runtime/src/segment_borrow.rs` carves arbitrary
  segment maps and asserts the registry accepts exactly the disjoint borrows and
  rejects every overlap, with read-sharing and per-account-isolation properties.
- **`cargo build-sbf` CI (was R-3).** `.github/workflows/solana-sbf.yml` installs
  the Anza toolchain and builds counter, vault, the Quasar port, escrow,
  versioned-state, and orderbook to SBF on every PR, enforcing the counter size
  budget. Every example was also built locally and deployed to devnet this pass.

## Deferred capabilities

### R-2: `kani` model-checking for the unsafe cast boundary

**Status:** partial.
**What:** model-checked proofs that the verified-cast layout checks (size,
alignment, discriminator, version) are sound for all inputs.

**Why still partial:** the `proptest` half is now shipped (see above) and a kani
proof layer already exists for the segment-borrow overlap predicate. Extending
kani coverage to the full `AccountView::load` casting boundary is higher-effort
(harness authoring over the header parse + length check) and stays gated behind
a dedicated lane so it never enters the standard `cargo test` run. The unsafe
surface is otherwise guarded by explicit `// SAFETY:` invariants, the
`docs/UNSAFE_INVARIANTS.md` inventory, and trybuild compile-fail fixtures.

**Shape:** add `#[kani::proof]` harnesses over `AccountView::load`; gate behind a
`kani` feature and a dedicated CI lane.

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
