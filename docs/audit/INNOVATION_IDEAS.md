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
- **Blind spot closed 2026-07-01 (with I12):** whole-account
  `Context::load_mut` borrows — previously invisible because they never
  reach the segment registry — now land in the touch log as
  `(0, data_len)` write records via `record_account_touch` (footprint
  only; liveness stays with the account borrow byte). Remaining gap:
  whole-account *reads* (`Context::load`) and direct `AccountView`-level
  borrows are still unrecorded — documented, and the latter is the same
  enforcement boundary as I12.
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

### I12 — Field-level write policies: the ledger as enforcer (THE moat)

- **shipped (core) 2026-07-01.** Landed as
  `#[hopper::context(strict_writes)]`: the context's *existing* `mut` /
  `mut(seg, ...)` / lifecycle declarations compile into a `static
  WritePolicy` installed during `bind()`, and every Context-mediated
  write acquire (`segment_mut*`, `split_segments_mut`, `load_mut`,
  `raw_mut`, `as_mut_ptr`) is gated at acquisition time with error page
  `Custom(0xD000 | account_index)`. Whole-account borrows claim
  `[0, data_len)` for both the policy and the touch map
  (`SegmentBorrowRegistry::record_account_touch`), which also closed
  the I7 blind spot. The declaration surface turned out to be already
  shipped (`mut(seg, ...)` accessor syntax) — the innovation was making
  it *enforced* rather than advisory. Runtime:
  `hopper-runtime/src/write_policy.rs`; tests in `write_policy.rs`,
  `context.rs::write_policy_tests` (gate ordering, refusal indexing,
  read-only-contract, whole-account claims), both feature lanes green.
  Follow-ups: publish write-sets through the schema/manifest so clients
  and indexers see per-instruction effects; SVM-level declared-vs-actual
  test (policy vs I7 touch map); `hopper doctor` lint flagging raw
  `AccountView` borrows inside `strict_writes` handlers (the documented
  enforcement boundary).
- Original idea: Impact: **very high** (a security primitive no chain
  framework has, and none can copy without Hopper's ledger). Effort:
  medium-high.
- Close-read synthesis of `policy.rs` + the touch-map work: all the
  pieces already exist — per-handler compile-time policy consts
  (`HopperInstructionPolicy`), a registry that intercepts every segment
  write at `register()` time, and `#[hopper::state]` field constants
  mapping names → `(offset, size)`. Solana's own safety model stops at
  account-level `writable`.
- **The breakthrough:** `#[instruction(writes = [vault.balance,
  vault.nonce], reads = [config])]` emits a const write/read-set; the
  registry **rejects any write borrow outside the declared set at
  acquisition time**. Byte-range `writable` — statically declared,
  runtime-enforced, published through the schema so clients, indexers,
  and auditors get "this instruction cannot touch anything else" as a
  machine-enforced contract rather than a comment. Pairs with the I7
  touch map for declared-vs-actual verification in tests and `hopper
  explain`.
- **Prerequisite (also fixes I7's blind spot):** whole-account borrows
  (`try_borrow_mut` / `load_mut`) bypass the segment registry today, so
  the shipped touch map records only segment-level access and a naive
  enforcement would too. Route whole-account write borrows through the
  same ledger (a `(0, data_len)` record / policy check at the
  account-level acquire points) — one change serving both the complete
  touch map and sound enforcement.
- Owner files: `crates/hopper-runtime/src/{segment_borrow.rs,account.rs,
  context.rs,policy.rs}`, `crates/hopper-macros-proc/src/program.rs`
  (writes= parsing), schema plumbing for published write-sets.

### I4 — `hopper doctor` as the safety linter no competitor has

- **idea.** Impact: high (DX). Effort: medium.
- The `.txt` audit called for a doctor that detects native u64 overlays, missing
  owner/writable checks, compact loaders without owner verification, realloc
  under live borrows, non-stable discriminators, unsafe Token-2022 extensions.
- **Surpass:** wire the audit's own finding categories (this log + FULL_AUDIT)
  into `hopper doctor` lints so the framework ships the auditor. Each P-class
  finding we fix becomes a lint that stops users reintroducing it.

### I13 — Hostile-metadata property harness (shipped)

- **shipped 2026-07-02** (`hopper-core/src/collections/mod.rs`,
  `hostile_metadata_proptests`). Impact: high (safety moat + a standing
  regression gate). Effort: low.
- Every zero-copy collection is an overlay on account bytes, and account
  bytes are **attacker-writable between instructions**: the stored
  lengths, heads, counts, and free lists are untrusted input on every
  touch. The Batch-3 sweep found five point instances of one root cause
  (a collection trusting its own stored metadata → OOB or panic). The
  harness generalizes the fix into a property: for arbitrary buffer
  contents and any API sequence, every collection returns a clean `Err`,
  never panics, never leaves bounds. It earned its keep immediately by
  catching a ring-buffer `get` underflow the manual point-fixes had
  missed.
- **Surpass:** no competitor ships on-chain zero-copy collections at all
  (Anchor/Quasar/Pinocchio have none), so this is Hopper-only ground —
  now corruption-fuzzed. COMPARISON.md row queued.

### I14 — Adversarial Miri suite for the unsafe overlay paths

- **idea.** Impact: high (soundness assurance). Effort: medium.
- Quasar's one testing discipline worth importing: `lang/tests/miri.rs`
  runs the unsafe pointer paths under Miri with Tree-Borrows +
  symbolic-alignment flags and publishes a findings table
  (`& -> &mut` casts, `MaybeUninit` init, event memcpy, etc.). Hopper
  has Kani proofs on the segment registry but nothing pointing Miri at
  the collections' `read_unaligned` / `write_unaligned` /
  `copy_nonoverlapping` sites or the frame/segment projection casts.
- **Surpass:** a Hopper Miri suite covering the collections (post-I13
  hardening), the frame provenance fixes (Batch 3 Wave 1), and the
  segment-lease projection — with the same published findings table, but
  over a *larger* unsafe surface than Quasar has. Pairs with I13:
  proptest fuzzes logic/panics, Miri proves no UB.

### I15 — Self-proving `FixedLayout::SIZE` (safe trait → proven trait)

- **shipped (core) 2026-07-02** (`hopper-core/src/account/pod.rs`).
  Impact: high (closes a soundness-hole *class* + DX). Effort: low.
- The disease behind the whole Batch-3 collection cluster: `FixedLayout`
  was a **safe trait** whose free `const SIZE: usize` fed unsafe pointer
  arithmetic across 34 files. `SIZE` and `size_of::<Self>()` were equal
  only by convention — every overlay author had to write the size
  *twice* (`const SIZE = N` plus a separate `const _: () =
  assert!(size_of == N)`; 10 such hand-written asserts existed) and a
  forgotten or wrong one flowed straight into out-of-bounds pointer math
  that no byte-fuzzer could reach (it is a property of the type
  parameter, not the bytes).
- **The fix — the framework owns the invariant, not the author:** `SIZE`
  now *defaults* to `size_of::<Self>()` (correct for every align-1
  no-padding `Pod` overlay — most impls become an empty body, the DX
  win), and a `#[doc(hidden)] const _SIZE_IS_HONEST: () = assert!(SIZE ==
  size_of::<Self>())` makes any dishonest override a **compile error the
  moment the type is used** in unsafe math. Consumers touch the proof
  (the collections via `assert_zero_copy_element`); rolling the touch
  into `pod_from_bytes` / the frame / segment projection paths extends
  the guarantee to every overlay consumer.
- **Surpass:** this is the sovereign-framework move — a trait that makes
  an unsafe invariant *unrepresentable-wrong*, rather than a lint or a
  convention. No competitor has an equivalent because none has this
  trait; and it simultaneously *reduces* boilerplate (Quasar-parity DX)
  while *raising* the safety floor. Follow-up: delete the 10 now-redundant
  hand-written `assert!(size_of == N)` guards and the explicit `const
  SIZE = N` on conforming types.

### I16 — Field behaviors: packageable per-field lifecycle plugins (the Quasar DX gap)

- **spiked (runtime core shipped) 2026-07-02.** Impact: high (the one
  place Quasar's DX is genuinely ahead). Effort remaining: medium (macro
  attachment). `hopper-runtime/src/behavior.rs` ships the plugin
  contract (`HopperBehavior<T>` with const-gated check/update/exit
  phases), the `BehaviorChecked<B, _>` proof token, the `BehaviorWrite`
  write-set contribution, and typed-path runners where `run_update`
  *takes* the check token — ordering made structural. A worked `FeeCap`
  plugin is test-pinned. Full macro design in
  `docs/design/BEHAVIORS_RFC.md` (attachment syntax, strict_writes
  folding, epilogue emission, trybuild matrix).
- Quasar's `AccountBehavior<A>` (`lang/src/account_behavior.rs`) lets a
  *protocol* author a reusable, parameterized behavior module and attach
  it per-field: `#[account(fee_vault(bps = 30))]`. Phase hooks
  (`set_init_param` / `after_init` / `check` / `update` / `exit`) are
  gated by associated consts so the derive emits code only for the
  phases a behavior actually uses; `VALIDATES_ACCOUNT_DATA` even lets a
  behavior take over data validation so parsing can use a cheaper
  pre-load path. Hopper's equivalents are weaker for this use case:
  `constraint = expr` is ad-hoc (copy-paste, not packageable),
  `#[validate]` is one per-context method (not per-field, not
  composable), and the lifecycle helpers are framework-owned (not
  protocol-extensible).
- **Match:** a `HopperBehavior<A>` trait + `#[account(my_behavior(...))]`
  attachment with const-gated phases, same shape.
- **Surpass (what Quasar cannot do):**
  1. behaviors return **proof tokens** (the `AccountProof` /
     `ExternalProof` capability layer already exists) so downstream APIs
     can *require* evidence a behavior ran — Quasar behaviors are
     side-effects only;
  2. behaviors declare their **write-set contribution** so they compose
     with I12 `strict_writes` instead of punching holes in it;
  3. behavior executions are visible in the I7 **touch map** and the
     receipt system, so `hopper explain` can show which behavior touched
     which field — auditable plugins, not opaque ones.
- Owner files: `crates/hopper-runtime/src/proof.rs` (token surface),
  `crates/hopper-macros-proc/src/context.rs` (attachment parsing +
  phase emission), new `hopper-runtime/src/behavior.rs`.

### I17 — One fingerprint algorithm to rule both authoring paths (wire:v3)

- **idea (seeded by Batch 4).** Impact: high (cross-path interop +
  ABI-identity soundness). Effort: medium.
- Batch 4 found the two authoring paths compute **incompatible**
  `LAYOUT_ID`s: `#[hopper::state]` uses `hopper:wire:v2` canonical
  stems (now hardened against size-bearing generics), while
  `hopper_layout!`/`hopper_interface!` are stuck on the
  source-spelling-dependent `hopper:v1` stringify hash because
  `macro_rules!` cannot normalize type tokens. A declarative
  `hopper_interface!` view can never verify a proc-macro-authored
  account. The moat feature (dependency-free cross-program reads
  pinned by fingerprint) silently doesn't span the framework's own two
  front doors.
- **Surpass:** `hopper-core::__sha256_const` already proves const-eval
  SHA-256 works. A `wire:v3` fingerprint computed **at const-eval time
  from runtime facts** — field count, per-field `size_of`, offsets from
  the segment map, plus the macro-normalized name stems — would be (a)
  identical across both authoring paths by construction, (b) impossible
  to fool with spelling or phantom-generic drift, and (c) sensitive to
  every real wire-shape change because the sizes come from the compiler,
  not from token strings. No competitor has *any* layout fingerprint;
  Hopper would have one that is provably spelling-independent.
- Migration: v2/v1 tags stay decodable; layouts opt into v3 with a
  version bump (`hopper_assert_fingerprint!` pins catch accidental
  flips). `hopper doctor` lint (I4) flags mixed-path programs until
  they unify.
- Owner files: `crates/hopper-macros-proc/src/state.rs`,
  `crates/hopper-macros/src/lib.rs`, `crates/hopper-core/src/lib.rs`
  (`__sha256_const`), docs.

### Batch 4 competitor cross-check (2026-07-04)

Read Quasar's derive layer sources directly against Hopper's macro
fixes this batch (`E:\Frameworks\quasar\derive\src\{error_code,event}.rs`):

- **Error codes.** Quasar assigns sequential codes from 0 and lowers
  `From<T> for ProgramError` as `e as u32` — enum and wire agree by
  construction, but codes are *reorder-fragile* (insert a variant,
  every later code shifts). Hopper's SHA-derived codes are
  reorder-stable, and after this batch's fix the discriminants are
  written back so `as u32` agrees too — Hopper now holds **both**
  properties; no action needed, worth a COMPARISON row.
- **Events.** Quasar's `#[event]` has the size==sum padding assert and
  a *closed* field-type vocabulary (primitives + Address only — an
  allowlist instead of a proof). Hopper's fixed `#[hopper::event]`
  now asserts align-1 + no-padding + per-field Pod proofs over an
  *open* vocabulary (any Pod type, including user pods) — strictly
  more expressive at equal safety. Their `MaybeUninit` log-buffer
  emission path is CU-tuned but relies on the same fence.
- **IDL emission.** Quasar collects IDL fragments via
  `inventory::submit!` behind an `idl-build` feature (linker-section
  magic, std-only, invisible to the type system). Hopper's
  `SCHEMA_METADATA` consts are no_std, reflection-free, and reachable
  at compile time — the better architecture; document it rather than
  copy theirs.

