# Hopper Innovation Log — 2026

Leap-ahead opportunities surfaced while auditing Hopper line-by-line and reading
the competitor sources at `E:\Frameworks\{anchor,quasar,pinocchio}`. This is the
raw feed for the Phase 5 gap-closure + innovation plan
(`GAP_CLOSURE_AND_INNOVATION_2026.md`). The bar is **match and surpass**: each
idea should make Hopper do something no competitor does, or do a shared thing
demonstrably better.

Ranking: **Impact** (dev-adoption / safety / performance) × **Effort**.
Status: `idea` | `spiked` | `planned` | `shipped`.

## Ideas

### I1 — PDA canonicalization by default (safety moat)
- **idea.** Impact: high. Effort: medium.
- The bump-canonicalization vuln (an attacker-supplied non-canonical bump
  verifying for the same seeds) is a recurring Solana exploit class. Anchor
  makes you write `bump = state.bump` by hand; Quasar/Pinocchio leave it fully
  manual. Hopper already has the cheap primitive (`verify_pda_from_stored_bump`,
  ~200 CU stored-bump verify vs ~1500 CU search).
- **Surpass:** make `#[account(seeds = [...], bump)]` *automatically* store the
  canonical bump at init and verify the stored bump on every subsequent load,
  with no `bump = ...` argument — canonicalization becomes impossible to get
  wrong. A `hopper doctor` lint flags any hand-rolled PDA check that verifies a
  caller-supplied bump.
- Owner files: `crates/hopper-macros-proc/src/context.rs` (seeds/bump lowering),
  `crates/hopper-native/src/pda.rs` (primitives exist).

### I2 — Borrow-sound-by-construction zero-copy (audit-trail moat)
- **idea.** Impact: high (marketing + real safety). Effort: low (mostly done).
- This audit is converting every reference-returning account API to return
  borrow-tracked `Ref`/`RefMut` guards (see FULL_AUDIT close/zero_data/projection
  fixes). Quasar is "beta/unaudited"; Pinocchio leaves aliasing to the author;
  Anchor pays RefCell cost at runtime.
- **Surpass:** finish the sweep so *the entire* Hopper zero-copy surface is
  aliasing-sound with zero `unsafe` reachable from safe user code, then publish
  the `FULL_AUDIT_2026.md` trail as a first-class differentiator ("the only
  audited zero-copy Solana framework"). Consider a `cargo hopper verify-unsafe`
  that asserts every `unsafe` block has a machine-checked SAFETY tag.

### I3 — Compute-unit type-state / const CU budgeting
- **idea.** Impact: medium-high. Effort: high.
- BENCHMARKS.md shows Hopper knows the CU cost of each primitive. No framework
  exposes a *compile-time* CU budget.
- **Surpass:** a `#[hopper::budget(200_000)]` attribute that sums the known
  per-primitive CU costs of an instruction's validated path at build time and
  fails the build (or warns) if the static lower bound blows the budget —
  turning CU regressions into compile errors. Ties into the existing
  `hopper profile bench` numbers and `hopper_test::Trace`.

### I4 — `hopper doctor` as the safety linter no competitor has
- **idea.** Impact: high (DX). Effort: medium.
- The `.txt` audit called for a doctor that detects native u64 overlays, missing
  owner/writable checks, compact loaders without owner verification, realloc
  under live borrows, non-stable discriminators, unsafe Token-2022 extensions.
- **Surpass:** wire the audit's own finding categories (this log + FULL_AUDIT)
  into `hopper doctor` lints so the framework ships the auditor. Each P-class
  finding we fix becomes a lint that stops users reintroducing it.
