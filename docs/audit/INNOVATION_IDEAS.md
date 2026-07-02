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

### I5 — Heterogeneous field splits (`split_mut!` over mixed types)

- **idea.** Impact: high (field-level DX no framework has). Effort: medium.
- `split_segments_mut::<T, N>` is homogeneous over one `T`; the original
  audit explicitly asked for
  `ctx.split_mut!(vault.balance: WireU64, vault.nonce: WireU32, ...)`.
  Audit finding: the machinery is already in place — the registry accepts
  arbitrary `(offset, size)` pairs, `SegmentsMut` holds one exclusive byte
  borrow + N leases, and `TypedSegment<T, OFFSET>` already bakes per-field
  types/offsets as const generics. Only the per-`T` size check and the
  return shape (tuple of `&mut T1, &mut T2, …` instead of `[&mut T; N]`)
  block heterogeneity.
- **Surpass:** a `split_mut!` macro (arities 2–4) lowering to a
  tuple-returning `split_segments_mut_hetero`, fed by the
  `#[hopper::state]` field constants so offsets/types can't drift. No
  competitor has *any* field-level simultaneous-borrow story.
- Owner files: `crates/hopper-runtime/src/{account.rs,segment_lease.rs}`,
  macro sugar in `hopper-macros`.

### I6 — Const-driven typed segments + built-in duplicate audits (COMPARISON rows)

- **shipped — document it.** Impact: medium (positioning). Effort: low.
- Audit of `context.rs` confirmed two already-shipped capabilities that
  COMPARISON.md undersells: (a) the three-tier segment access where
  `TypedSegment<T, OFFSET>` monomorphizes field access to
  ptr+const-offset with a leased borrow (Anchor: none; Quasar:
  whole-account; Pinocchio: manual); (b) one-line Sealevel-attack
  mitigations `require_unique_writable_accounts()` /
  `require_unique_signer_accounts()` on every context (competitors leave
  duplicate-account checks entirely to the author); (c) the
  remaining-accounts taxonomy — `strict` (duplicate-rejecting),
  `passthrough`, `typed` (sequential typed parser), and `lazy` (indexed)
  modes over the most footgun-prone Solana surface, vs Anchor's raw
  slice. Add all three as COMPARISON.md rows with file/symbol citations.

### I7 — Instruction touch maps from the borrow ledger (nearly free)

- **shipped (core) 2026-06-30.** The segment registry now keeps an
  append-only, deduplicated touch log behind the `touch-map` feature
  (zero cost when off): `Context::for_each_touch` / `touch_map_len` /
  `touch_map_overflowed` yield the instruction's cumulative
  `(account, offset, size, R/W)` footprint in first-touch order —
  surviving RAII lease releases, which the live ledger does not.
  Test-pinned (`touch_log_survives_release_and_dedups`,
  overflow-partiality). Remaining follow-ups: receipt/`sol_log_data`
  emission encoding, `hopper explain` field-name decoding via
  `#[hopper::state]` constants, and `hopper_test::Trace` surfacing.
- Original idea: Impact: high (unique explainability moat). Effort: low-medium.
- Second-pass audit of `context.rs` found the original audit's wishlist
  item #7 ("instruction touch maps") is almost already built: `Context`
  embeds the instruction-scoped `SegmentBorrowRegistry`, every segment
  access flows through it, and `SegmentBorrowRegistry::for_each` already
  exists "for the hopper explain introspection path". By end-of-handler
  the ledger has recorded exactly which `(account, offset, size, R/W)`
  ranges the instruction touched — no extra bookkeeping needed.
- **Surpass:** emit the ledger as a machine-readable touch map through
  the receipt system and `hopper_test::Trace`
  (`"mutates": ["vault[8..16] W", "user[0..8] R"]`), and let `hopper
  explain` decode it against `#[hopper::state]` field constants into
  field names. No competitor can produce this because none has an
  instruction-scoped aliasing ledger to read it from.
- Owner files: `crates/hopper-runtime/src/{context.rs,segment_borrow.rs}`,
  `crates/hopper-core/src/receipt.rs`, `crates/hopper-test/src/trace.rs`.

### I8 — Tuned SBF linker script (close the binary-size gap cheaply)

- **invalidated by measurement (2026-06-30).** Built `hopper-counter` to
  SBF with and without an equivalent discard script: byte-identical
  (4 736 bytes), and `llvm-readelf -S` shows the baseline artifact already
  carries only the 8 essential sections — `cargo-build-sbf` 4.0 strips
  `.symtab`/`.eh_frame`/hash tables by default. Quasar's `sbf.ld` matters
  for *their* plain-cargo + `sbpf-linker` route, not Hopper's toolchain.
  **Consequence:** the 6.27 vs 7.53 KiB vault gap is real `.text`/framework
  code, so the size-gap attack is I10's smaller validation codegen plus a
  dead-weight review of the fixed runtime surface — not linker games.
- Original (superseded) idea kept for the record: Impact: high. Effort: low.
- Competitor-source study: Quasar ships `link/sbf.ld`, a ~20-line linker
  script whose `/DISCARD/` drops `.eh_frame*`, `.gnu.hash*`, `.hash*`,
  `.comment*`, `.symtab`, `.strtab`, and `.debug_*` from the final `.so`.
  This is very likely most of the 6.27 KiB (Quasar) vs 7.53 KiB (Hopper)
  vault-binary gap recorded in BENCHMARKS.md — not codegen, just sections.
- **Surpass:** ship an equivalent (or tighter) script wired through
  `hopper build` / the workspace `.cargo/config`, measure the delta in the
  Phase 3 benchmark, and keep `hopper verify`'s `.rodata` layout-id anchor
  intact (verify the anchor section survives the discard list).
- Second-pass detail: Quasar builds through Anza platform-tools'
  **`sbpf-linker`** (checked on PATH by their CLI; `-C link-arg=--btf
  -C debuginfo=2` only on their debug/profile path). Evaluate whether
  `hopper build` should support the same linker route alongside
  `cargo-build-sbf`.
- Owner files: new `link/sbf.ld` + `tools/hopper-cli` build plumbing;
  validate with `examples/hopper-parity-vault`.

### I9 — Static DWARF/ELF profiler fused with measured CU

- **idea.** Impact: medium-high (debug/perf DX). Effort: medium.
- Quasar's `profile/` crate statically attributes binary size per function
  from DWARF (`(addr, size, name)` symbol table + frame resolution), diffs
  two builds, and serves a local report UI. Hopper's `hopper profile bench`
  measures *runtime CU* but has no static attribution and no build diff.
- **Surpass:** `hopper profile size` with DWARF per-function bytes + stack
  attribution and `--diff <old.so>`, then fuse with the existing measured
  CU data into one report (bytes *and* CU per function) — Quasar only has
  the static half, Anchor/Pinocchio have neither.
- Owner files: `tools/hopper-cli` (new subcommand; `gimli`/`object` crates
  host-side only), reuse `hopper profile bench` plumbing.

Minor note from the same study: Quasar's `#[account(dup)]` lets a context
*declare* an intentional duplicate account and validates it through checked
borrows (`AccountLoad::check_checked`). Hopper's `require_unique_*` +
strict remaining-modes cover the safety side; a declared-duplicate field
attribute would be small ergonomic parity if users ask.

### I10 — Fused expected-header validation (one u32 compare per account)

- **idea.** Impact: **high** (directly attacks the fixed-overhead CU gap).
  Effort: medium (macro lowering change; primitive already exists).
- Second link/loader pass over Quasar found the real breakthrough: their
  `#[derive(Accounts)]` plan (`derive/src/accounts/plan.rs`) computes a
  **compile-time expected-header mask** per account field —
  `0xFF | (S << 8) | (W << 16) | (E << 24)`, the packed
  `[dup_marker, is_signer, is_writable, executable]` u32 at the head of
  each account record — and `parse_accounts` validates all three flags
  with **a single u32 load + compare against a literal, fused into the
  SVM-buffer parse walk** (dup-aware via `parse_account_dup`). Their
  generated context is bound in the same pass; no generic account array,
  COUNT known at compile time. This is very likely the 41-vs-72 CU
  auth-fail delta in BENCHMARKS.md.
- **Hopper already has the primitive**: `AccountView::header_u32()` /
  `flags()` / `expect_flags()` read exactly this packed u32. The gap is
  purely in lowering — `#[derive(Accounts)]`/`#[hopper::context]` emit
  separate `check_signer()` / `check_writable()` calls per field instead
  of one fused compare.
- **Surpass:** (a) lower per-field flag constraints to one
  `expect_flags`-style masked compare against a macro-computed literal;
  (b) go further than Quasar by folding the **discriminator byte** check
  into the same pass (their compare covers flags only), and keep the
  borrow-ledger/touch-map machinery they lack; (c) benchmark the delta on
  the parity vault's auth-fail row.
- Owner files: `crates/hopper-macros-proc/src/context.rs` (lowering),
  `crates/hopper-native/src/account_view.rs` (primitive exists),
  `examples/hopper-parity-vault` (measure).

### I11 — Shipped-but-invisible cluster: foreign lenses, proof markers, migration chains, crank discovery

- **shipped — document + polish.** Impact: high (positioning). Effort: low.
- The Batch-2 tail sweep found four substantial shipped capabilities that
  neither COMPARISON.md nor the website leads with, and that none of
  Anchor/Quasar/Pinocchio has any equivalent of:
  - `foreign.rs` — **manifest-backed foreign-account lenses**: 4-point
    verified cross-program reads (owner + disc + wire fingerprint +
    schema epoch) with caller-supplied manifests. No crate coupling, no
    silent ABI drift; Anchor requires importing the foreign crate,
    Quasar/Pinocchio hand-maintained offsets.
  - `proof.rs` — **proof-carrying account markers**: type-state
    capabilities (`OwnerChecked`, `SignerChecked`, `LayoutChecked<T>`,
    tuple composition) so downstream helpers *require* proofs in their
    signatures. Type-level check composition no competitor has.
  - `migrate.rs` — **schema-epoch migration chains**: declared edges
    applied in sequence, atomically, with epoch bumps at load time,
    before any typed access. The governed-migration moat, implemented.
  - `crank.rs` — **on-chain crank markers**: crankable-instruction
    discovery stamped into the binary for indexers/`hopper manager`.
- Action: COMPARISON.md rows + website copy for all four; line-by-line
  audit of the four files rides along in the Batch-2 tail.

### I4 — `hopper doctor` as the safety linter no competitor has

- **idea.** Impact: high (DX). Effort: medium.
- The `.txt` audit called for a doctor that detects native u64 overlays, missing
  owner/writable checks, compact loaders without owner verification, realloc
  under live borrows, non-stable discriminators, unsafe Token-2022 extensions.
- **Surpass:** wire the audit's own finding categories (this log + FULL_AUDIT)
  into `hopper doctor` lints so the framework ships the auditor. Each P-class
  finding we fix becomes a lint that stops users reintroducing it.
