# Gap Closure & Innovation Plan — 2026-07-07 research pass

This is the synthesis document the innovation log (`INNOVATION_IDEAS.md`) names
as its destination. It merges three inputs:

1. The internal baseline: `COMPARISON.md`, `BENCHMARKS.md` (2026-07-02 four-
   framework run), `HOPPER_COMPETITIVE_AUDIT_2026_06_27.md`, the I1–I17
   innovation log, and a fresh mining pass over the local competitor checkouts
   at `E:\Frameworks\{quasar,pinocchio,anchor}`.
2. External research (2026-07-07): latest upstream state of Quasar
   (blueshift-gg), Pinocchio (anza-xyz), Anchor, Typhoon, Steel, star-frame,
   and the current Agave cost model.
3. The strategic target set by the project owner: **Hopper must be able to
   write an Autobahn-class program — head-to-head Pinocchio vs Quasar vs
   Hopper — and do the same work in fewer CU, smaller binaries, with more
   safety.**

Status legend matches the innovation log: `idea | spiked | planned | shipped`.

---

## 1. Where Hopper measurably stands (internal baseline, 2026-07-02 run)

Four-framework vault parity benchmark (Mollusk, 8 seeds, same toolchain,
provenance in `BENCHMARKS.md`):

| Scenario | Hopper | Anza Pinocchio | Quasar | Anchor 0.31.1 |
|----------|-------:|---------------:|-------:|--------------:|
| Authorize | **466** | 2512 | n/a | 5017 |
| Auth-fail | 107 | **41** | n/a | 2284 |
| Counter | **564** | 2539 | n/a | 5156 |
| Deposit | **1713** | 3856 | 1756 | 7150 |
| Withdraw | **488** | 2548 | 592 | 5108 |
| Binary size | 7.46 KiB | 7.73 KiB | **5.47 KiB** | 190.11 KiB |

Read honestly:

- Hopper beats Quasar on both of Quasar's own upstream workloads while
  carrying the full state-contract surface.
- The Pinocchio deltas are mostly the stored-bump-vs-`find_program_address`
  strategy difference; a stored-bump Pinocchio row is tracked.
- Two rows Hopper does not win: **auth-fail** (107 vs Pinocchio 41 — the
  scanning-entrypoint pass; returns with SIMD-0321) and **binary size**
  (7.46 vs Quasar 5.47 KiB — real `.text`, not linker sections; I8 was
  invalidated by measurement).

## 2. External landscape — July 2026

### Quasar (blueshift-gg) — deep dive, verified 2026-07-07

Identity: `github.com/blueshift-gg/quasar`, created **2026-02-16**, last push
**2026-07-04** (local checkout `37e8a6b` @ 2026-06-28 is ~1 week behind, no
structural drift). Authors: Leonardo Donatacci (L0STE) + Dean Little; febo
(the Pinocchio author) has contributed. **v0.0.0 — no tags, no releases, not
on crates.io**; 159 stars / 54 forks; explicitly "Beta … not audited."
Ecosystem: `zeropod`, `sbpf-linker`, `quasar-svm`, `beethoven` (client-side
CPI router), vendored `solana-compiler-builtins`. Builds on **nightly Rust**
with a bespoke toolchain (platform-tools v1.52 or `sbpf-linker` +
`build-std`), `target-cpu=v2`.

Their CU playbook, cross-checked against Hopper's shipped state:

| # | Quasar technique (cited to their source) | Hopper status |
|---|---|---|
| 1 | Batched u32 account-header validation fused into the parse walk (`parse_account`) | **Parity** — I10 shipped (fused expected-header compare), plus disc-byte folding Quasar lacks |
| 2 | Batched CPI flag extraction: header u32 `>> 8` + transmute into `CpiAccount` | **Gap (micro)** — Hopper builds CPI metas field-by-field; see G6 |
| 3 | Direct `sol_invoke_signed_c`, const-generic stack CPI buffers | **Parity** — `hopper-native/src/cpi.rs` |
| 4 | PDA find loop: `keys_eq` instead of `sol_curve_validate_point` (~90 CU/iter saved) | **Parity** — verify-only PDA path (BENCHMARKS.md) |
| 5 | Stored-bump fast path **auto-detected** from a `bump: u8` field (`BUMP_OFFSET`) | **Gap (DX)** — Hopper has the primitive; auto-detection + golden path is I1 |
| 6 | Direct `sol_sha256` full find (~544 CU vs ~1,500), zero-copy seed passing | **Parity** — same class of primitive |
| 7 | `keys_eq` as 4×u64 word compare; `is_system_program` OR-fold | **Gap** — G1: Hopper's fast compare only on explicit call sites; `Address` uses derived eq (~40 CU measured vs ~8–12) |
| 8 | Per-instruction codegen shape selection (COUNT ≥ 8 → direct-bind path, else buffered) | **Gap (micro)** — Hopper emits one binding shape; see G5 |
| 9 | Fn-pointer-table dispatch for contiguous 1-byte discriminators (`callx`, ~5 CU) | **Gap (micro)** — Hopper dispatch is a match chain; see G5 |
| 10 | Const-folding dead work: event-branch elision, epilogue elision (`HAS_EPILOGUE`, ~2–7 CU), compile-time event-authority PDA | **Partial** — Hopper has `__sha256_const`/const fingerprints; epilogue/branch elision worth an audit pass |
| 11 | `#[cold]`/unlikely on all failure paths | **Parity** — `check/fast.rs`, `error.rs`, `raw_input.rs` |
| 12 | `no_alloc!` default + poisoned heap cursor per endpoint | **Parity-ish** — Hopper has `no_allocator!`/`default_allocator!`; cursor-poisoning is a nice detail |
| 13 | Instruction args as align-1 `#[repr(C)]` overlay, single length check | **Parity** — args overlay (Batch 4 fences) |

Their verification culture is real and close to ours: Kani 0.67 proof
harnesses on nearly every unsafe fn, Miri Tree-Borrows + symbolic alignment,
a `check-runtime-panics` CI gate (no panic paths in runtime/derive code).
Two Hopper-relevant differences: they run Miri over the whole lang/spl
surface (Hopper's I14 is still open), and generated Quasar *programs* set
`overflow-checks=off` (Hopper keeps overflow checks on outside the audited
substrate — a safety-positioning point).

**Open Quasar unsoundness/correctness issues (their tracker, 2026-07):**
`#238`/`#234` CPI return data `assume_init` over uninitialized bytes (UB —
Hopper's `get_return_data` verified sound against this class, 2026-07-07);
`#240` account self-close unbalanced-instruction error; `#239` migration
leaves stale state; `#242` `Remaining<T,N>` capacity overstated (real cap
`min(N, 64/COUNT)`); raw-handler duplicate-account aliasing footgun
(`ptr::read` aliased `AccountView`s + `borrow_unchecked_mut`) — exactly the
class Hopper's borrow registry eliminates.

**Quasar publishes no comparative CU benchmark** — no benchmarks page, no
committed numbers, only loose in-test bounds (`emit!` < 500 CU) and doc
claims. Their `make bench-cu` prints numbers at test time; the regression
script compares HEAD vs master but publishes nothing. Hopper's pinned,
provenance-checked `hopper-bench` matrix is ahead of every competitor here.

### The structural shift: everyone now builds on Pinocchio

As of July 2026 the low-CU field has **collapsed onto Pinocchio as the
substrate**: Anchor v2 (alpha), Typhoon, and star-frame all build on it;
Quasar shares its lineage (febo contributes to both). Only Steel (native
`solana-program`) and shipped Anchor 1.x don't. The old
`solana-nostd-entrypoint` is archived, superseded by Pinocchio. **Hopper is
the only serious framework with its own sovereign substrate**
(`hopper-native`). That is both the moat (no upstream coupling, our own
audited invariants, the segment-borrow ledger no Pinocchio-based framework
can retrofit) and a standing obligation: every Pinocchio soundness fix must
be cross-checked against our equivalents (their 0.11 line added `&mut
[AccountView]` mutable-view semantics, `Clock::from_bytes` alignment checks,
pointer privacy — the same class of hardening our audit batches did).

### Pinocchio (anza-xyz) — v0.11.2, 2026-06-09

Very active (last commit 2026-07-06). 0.11 was breaking: mutable account
views (`&mut [AccountView]`), `Resize`/`UnsafeResize` traits (safe variant
+2 CU/account), alignment/pointer hardening. Companion crates all bumped
2026-04 (token 0.6.0, token-2022 0.3.0, system 0.6.0). **Audited** (Neodyme
2025-06, Zellic 2025-06) with zero RUSTSEC advisories; p-token — SPL Token
rewritten in Pinocchio — is rolling out as the canonical token program via
**SIMD-0266** with Runtime Verification equivalence proofs and Certora
formal verification behind it. Production users beyond p-token: HumidiFi,
Doppler. Its pains are unchanged and structural: total manual safety burden,
no IDL, no client story (their issue #405 asks for derive macros).
**Bench-harness action: our in-tree Pinocchio comparator pins 0.10 — bump
to 0.11.2.**

### Anchor — shipped 1.1.2; the real story is the unreleased v2

- **Shipped:** `anchor-lang` 1.1.2 (2026-06-26), repo now
  `solana-foundation/anchor`, OtterSec maintains. 1.0 (2026-04-02)
  de-monolithized `solana-program` (−17.2% binary), decoupled the CLI,
  added `Migration<'info, From, To>`, moved IDL to a framework-agnostic
  spec (`solana-foundation/idl-spec`) + on-chain Program Metadata Program,
  made Surfpool the default test validator and LiteSVM the default unit
  template. **Our benchmark's Anchor column (0.31.1, 190 KiB) is a version
  behind: re-run against 1.1.2 before the next published table, and expect
  its binary-size row to shrink meaningfully.**
- **Anchor v2 (`anchor-lang-v2`, `anchor-next` branch, unreleased, active
  through 2026-06-23):** built on **pinocchio 0.11** with "Wincode"
  serialization. Their in-repo `bench/results.json` (2026-06-09, litesvm
  0.11, Agave 3.1.12): hello-init **v2 ~1,381 CU** vs Pinocchio 1,680 vs
  Quasar 1,412; vault deposit 1,905 / 1,229 / 1,889; withdraw 393 / 57 /
  396; vault binary **5,376 B** / 5,072 / 6,080 (Anchor v1: 107,368 B,
  Steel: 65,160 B). **Read this plainly: when v2 ships, "10× cheaper than
  Anchor" stops being a durable headline for anyone.** The durable ground
  is (a) winning within the zero-copy cluster on real workloads (§5), and
  (b) the state/safety/tooling moats no Pinocchio-derived framework has.
- Anchor's own version-trend data shows **runtime sensitivity**: identical
  code moved 571→685 CU from Solana 2.1→2.3. Our published numbers were
  measured on a 2.1-era stack — §5 step 0 (agave-4.0 Mollusk) is also what
  keeps our own claims honest.

### Typhoon (exotic-markets-labs) — the quiet third contender

v0.3.0 (2026-04-07), pushed 2026-07-06, pinocchio-based thin layer,
Anchor-like macros, Codama IDL, Wincode, no_std. Its in-repo bench
(2026-07-06, litesvm 0.9.1) across vanilla-Pinocchio / Anchor 1.0 / Typhoon
/ star-frame / Quasar: Quasar tops most micro-ops (ping 9 CU, binary
13,144 B on their fixture), Typhoon at hand-written parity (accounts row
292 CU vs vanilla 323). Unaudited, CLI incomplete, no client codegen, one
maintainer + febo/stegaBOB contributing. Watch, don't chase.

### Steel (regolith-labs) — fading

4.0.9 (2026-06-16), native `solana-program` base, macro_rules-only, bus
factor 1 (Hardhat Chad), unaudited, no IDL. In Anchor's bench it's the
laggard of the moderns (65 KiB binary, 4,867 CU init). Only Ore confirmed.
Not the bar to beat.

### star-frame (Star Atlas) — one important idea, low adoption

v0.30.0 (2026-02-25), pinocchio-0.9.2 hybrid (std on), borsh args +
bytemuck state. Its differentiator is the **unsized-types system**:
growable `UnsizedList`/`UnsizedMap` that insert/delete by shifting bytes
in place inside a fixed account buffer — first-class growable zero-copy
state. Self-reported 60–94% CU under Anchor. Essentially Star-Atlas-only,
unaudited, Token-2022 unsupported, commit cadence slowed. The unsized-types
idea is the part worth answering (see I22).

### Libraries and adjacents

sokoban (Ellipsis; critbit/rbtree/avl/hashtable over bytes; powers Phoenix)
is the highest-adoption zero-copy library (264 K downloads) — a library,
not a framework; Hopper's collections already cover this ground with
corruption-hardening sokoban lacks. light-zero-copy 0.7.0 serves ZK
compression. Codama is the codegen hub both Anchor and star-frame emit
into (Hopper already emits Codama JSON — underused advantage). Bolt (ECS
gaming), poseidon (TS transpiler, dormant), nautilus (dead) are
non-factors, as are "sails"/"saturn"/"mainstay" (see below).

### Cross-framework CU picture (with the necessary caveat)

No published benchmark states its sBPF version and each pins a different
runtime, so cross-source numbers are not comparable; within-table ordering
is. The consensus ordering everywhere: **raw Pinocchio ≈ Quasar ≈ Typhoon ≈
star-frame ≈ Anchor v2 (alpha) ≪ Steel < Anchor 1.x < legacy Anchor.**
Within the zero-copy cluster the per-op winner flips within noise — which
is exactly why the router-class workload benchmark (§5) matters: micro-op
tables can't differentiate the cluster; realistic workloads with dynamic
accounts, CPI fan-out, and token I/O can. Reference points: Dean Little's
Accelerate memo-program ladder (Anchor 649 → Pinocchio ~108 → asm 104 CU);
p-token vs SPL Token (79 vs 4,645 CU isolated Transfer — but ~59% cheaper
on a full-transaction basis; always state measurement scope).

### Whitespace check (what no one owns, mapped to Hopper)

The external scan's ranked whitespace, corrected against what Hopper
already ships:

1. **IDL + client codegen for non-Anchor zero-copy frameworks** — called
   "the biggest, clearest gap" (Steel #68, Typhoon #207/#52, Pinocchio
   none; Codama's Rust-macro IDL gen still immature). **Hopper already
   occupies this**: manifest → TS/Kotlin/Python/Go/C/Rust + Codama JSON +
   Anchor-compatible IDL. The gap is *visibility and polish* (PDA
   auto-resolution, fail-closed decode — G8), not existence. Lead with it.
2. **Turnkey CU + binary-size regression CI** — confirmed unowned (Mollusk
   bencher has deltas but no thresholds/Action/PR gating). This is I19.
3. **Fuzz-harness generation without an Anchor IDL** — Trident is
   Anchor-locked; Kani is Quasar-internal only. Hopper's manifests contain
   exactly the metadata needed. This is I21 (new, below).
4. **Growable safe zero-copy state** — only star-frame attacks it. Hopper's
   bounded tails + 8 collections cover most of it with hardening no one
   has; the in-place growable list/map shape is the remaining piece (I22).
5. **Token-2022 ergonomics** — contested (Quasar's spl crate is the direct
   rival); Hopper's TLV constraints + hook resolver are already ahead —
   publicize per CU_COSTS.md numbers.
6. **sBPF v3 readiness as a marketed feature** — no framework claims it;
   Hopper already has the feature-gate tooling precedent (SIMD-0321). Small,
   credible (I23).
7. **Verifiable-by-construction builds** — solana-verify exists but is
   fragile; minimal-dep no_std posture helps; fold the attestation hash
   into `hopper build`/`deploy` and the §5 publishing pipeline.
8. Anchor's most-complained DX wound is **cryptic macro errors** (#4015
   open since 2024) — Hopper's trybuild-tested diagnostics are a stated
   differentiator; keep them release-gated.

### The 2026 cost model (what CU claims must be priced against)

From agave `program-runtime/src/execution_budget.rs` (read 2026-07-07) and
active/scheduled SIMDs:

- **CPI base cost 1000 → 946** with **SIMD-0339**, which *also* adds CU
  scaling with the number of account-infos + instruction-account-metas
  passed, and raises the CPI account-info limit **64 → 255** (testnet
  activation cited at epoch 883, Agave ≥ 3.1). **This directly reprices
  router-class programs:** frameworks that marshal fewer bytes/accounts per
  CPI win more than before, and `Remaining`-style capacity assumptions
  change. Hopper actions: re-cost `DynCpi`/`invoke_with_bounds` docs, revisit
  remaining-accounts caps, and pin the bench feature set to include it.
- **SIMD-0268**: CPI stack depth 5 → 9 (deeper router nesting becomes legal).
- **SIMD-0170**: builtins reserved at 3,000 CU.
- **SIMD-0186**: loaded-account data pricing (`data_len + 64` per account,
  flat 8,248 B per ALT) — client codegen can optimize ALT usage against it.
- **sBPF v3** (SIMD-0178/0179/0189): stricter ELF headers, smaller binaries;
  toolchain-transparent. Track alongside the existing SIMD-0321 gate work.
- Reference constants for budget math: `create_program_address` 1,500 CU;
  `cpi_bytes_per_unit` 250; `mem_op_base` 10; sha256 85 + 1/byte; heap
  8 CU/32 KiB page; max ix CU 1.4 M; write-lock 300 CU (block cost, not
  program meter).
- **Mollusk is now `anza-xyz/mollusk` v0.13.4** on agave 4.0, and ships
  `MolluskComputeUnitMatrixBencher` — a purpose-built column-per-program
  matrix for head-to-heads. `hopper-bench` pins 0.10.3; upgrading is step 0
  of §5. CU parity with mainnet is governed entirely by `SVMFeatureSet` —
  a stale default silently shifts numbers.

### Deploy-cost economics (the "cheapest" axis, verified formula)

Rent-exempt deploy cost = `(elf_bytes + 128) × 6,960 lamports` (verified
against live mainnet autobahn: 176,205 B → 1.2277 SOL exactly). Applied to
the 2026-07-02 vault matrix: Hopper 7.46 KiB ≈ **0.054 SOL**, Quasar
5.47 KiB ≈ 0.040 SOL, Pinocchio 7.73 KiB ≈ 0.056 SOL, Anchor 190.11 KiB ≈
**1.356 SOL** (~25× Hopper). Hopper's 4,688-byte counter ≈ **0.034 SOL**.
Deploy/upgrade cost is a marketing-grade "cheapest" number worth publishing
per example, and it makes the G2 size gap worth ~0.014 SOL on the vault —
real but small; Anchor is the outlier to hammer.

### Competitive urgency: the Anchor camp is building its own benchmark

`otter-sec/anchor` issue **#4355 "Benchmark Anchor V2"** (opened 2026-03-25,
targeted before Accelerate, still unpublished 2026-07-07) plans a
standardized program compared across **Anchor V1, Anchor V2 (RC), Quasar,
Pinocchio, raw native** — confirming (a) an Anchor V2 release candidate
exists, and (b) the first published cross-framework benchmark table from a
neutral-ish party is coming. Hopper is not in their matrix. Publishing our
own reproducible router-class matrix **first** — with Hopper in it and
provenance no one can dispute — sets the reference frame before theirs
lands.

### Entrypoint-level reference numbers (eisodos, fetched 2026-07-07)

febo's `eisodos` (Mollusk, platform-tools 1.51 + LTO) now includes Sanctum's
jiminy: Ping 14/16 CU (Pinocchio/jiminy), Account(64) 512/982 CU, CPI
transfer 1,287/1,301 CU, binary **5,824 B Pinocchio vs 3,680 B jiminy** vs
64,784 B solana-program. Takeaway: jiminy wins size, Pinocchio wins account
parsing; both define the substrate floor Hopper's eager path is measured
against.

### Long-tail frameworks (verified 2026-07-07, primary sources)

- **jiminy (igneous-labs / Sanctum)** — github.com/igneous-labs/jiminy. Real
  and active (last commit 2025-12-27; org highly active), `no_std`
  zero-dependency pinocchio-class library. Its one genuinely novel idea:
  **compile-time account-borrow checking** (`AccountHandle` + `Abr` singleton)
  instead of Pinocchio's runtime `RefCell` checks; `ProgramError` as
  `NonZeroU64` for tighter bytecode; poolable CPI buffers. Not on crates.io;
  low adoption (18 stars) — internal Sanctum tooling, but the borrow-model
  idea is directly relevant to Hopper's segment-borrow registry (a
  const-provable subset would answer ROADMAP R-4).
  Naming note: the `jiminy` crates *on crates.io* (v0.17.0, `jiminy-core`,
  "built on Hopper") are this project's own sibling — unrelated to Sanctum's.
  Verified priority (commit-level, 2026-07-07): Sanctum's first use of the
  name is commit `824d892` "initial commit" (billythedummy / Han Yang),
  authored 2025-02-17 16:57 UTC — 78 s before repo creation, so no earlier
  imported history exists. Never published to any registry; their org has
  no other jiminy-matching repo. Our first crates.io publish is 2026-02-23,
  ~12 months later, and every `jiminy*` crate on crates.io is ours.
  Coincidental collision, different designs; a one-line README
  disambiguation is the only action worth taking.
- **mainstay** — dead Anchor 0.30 fork (`mainstay-lang` 0.30.x mirrors
  `anchor-lang`); no release since 2024-11-08, repo and author account
  deleted. Not a competitor.
- **"sails" / "saturn"** — no such Solana frameworks exist (sails is
  Gear/Vara; saturn hits are wallet/token products). Filtered out.

## 3. Gaps to close (ranked)

### G1 — Key-compare lowering is inconsistent; no program-wide intrinsic override (new, found 2026-07-07)

- **Evidence.** Quasar ships `solana-compiler-builtins`: `#[no_mangle] extern
  "C" memcmp` (word-wise inline for `n ≤ 32`, `sol_memcmp` syscall above) —
  and **only** memcmp; there are no memcpy/memset/memmove overrides. Source
  correction (2026-07-07 re-read): the override is gated
  `target_arch = "bpf"`, which is **false** on their default
  `cargo build-sbf` route (build-sbf sets `target_arch = "sbf"`,
  `target_os = "solana"`), so on that route the crate links nothing and
  their published numbers never included it. It also carries an ordering
  bug (returns `1` on any word mismatch, never `-1`). Separately, the
  intrinsic-override story does not reprice derived `Address` equality at
  all: LLVM inline-expands fixed-size 32-byte compares before a `memcmp`
  symbol is emitted — built Hopper `.so` files contain **no memcmp symbol**
  for those compares. The ~40 CU `check_keys_eq` vs ~8 CU token-constraint
  split (`BENCHMARKS.md`, `docs/CU_COSTS.md`) is a codegen-shape cost that
  only the manual `PartialEq` fixes; the override's value is
  *runtime-length* memcmp/memcpy/memset call sites (core slice cmp over
  long `&[u8]`, third-party code) that otherwise pay the ~10 CU `mem_op`
  syscall base + shim overhead even for tiny `n`.
- **Fix.** (a) Implement `PartialEq` for `Address` manually as four
  `read_unaligned::<u64>` compares (align-1 sound, no unsafe leak); route
  `require_keys_eq!`, `has_one`, `owner =`, `address =`, and dedup checks
  through it — this is what carries the derived-eq CU win. (b) Ship an
  opt-in `hopper-builtins` rlib with ordering-correct tuned
  `memcmp`/`bcmp`/`memcpy`/`memset` intrinsic overrides (no `memmove` —
  the toolchain shim is already the right shape), gated
  `target_os = "solana"` so it actually fires on the build-sbf route —
  ground Quasar does not hold. Duplicate-symbol risk vs the GLOBAL
  toolchain shims must be smoke-tested in a later phase before default-on.
  (c) Re-measure the vault matrix; expect
  20–30 CU off every account-validation-heavy row.
- **Owner files.** `crates/hopper-runtime/src/address.rs` (manual `PartialEq`),
  new `crates/hopper-builtins`, `crates/hopper-native/src/mem.rs` (threshold
  unification), `hopper-bench` re-run.

### G2 — Binary-size gap vs Quasar (7.46 vs 5.47 KiB on the vault)

- Post-I8 knowledge: the gap is real `.text` (framework validation codegen +
  fixed runtime surface), not ELF sections. Attack via (a) I9 DWARF
  per-function size profiler to name the bytes, (b) dead-weight review of the
  fixed runtime surface, (c) check what `solana-compiler-builtins`-style
  intrinsics do to size (word-loops are smaller than unrolled byte loops).
  The 2026-07-07 source pass adds two size-relevant Quasar habits worth
  copying where they fit: `#[inline]` rather than `#[inline(always)]` on
  large PDA functions (avoids `.text` duplication), and word-loop
  intrinsics, which are smaller than unrolled byte loops (I18 helps size
  as well as CU). Correction from the source re-read: Quasar's
  `solana-compiler-builtins` covers only `memcmp` and is gated
  `target_arch = "bpf"` — false under `cargo build-sbf` — so their 5.47 KiB
  vault does not include it; the size lever is ours to measure, not theirs
  to copy. sBPF v3 static syscalls are the other
  structural size lever (I23).

### G3 — Auth-fail row (fail-fast validation before full account scan)

- Quasar binds its generated context fused into the SVM-buffer parse walk and
  can reject at the first bad account (41 CU measured on Pinocchio's row; the
  same shape). Hopper's sound scanning entrypoint pays the walk first
  (107 CU). I10 (fused expected-header compare) shipped; the remaining gap is
  *scan-fused context binding* on the eager path, or making the lazy
  entrypoint (`hopper_lazy_entrypoint!` + `LazyContext`) the golden path for
  fail-fast-sensitive programs. SIMD-0321 (`r2` instruction data) recovers
  ~30–40 CU when it activates; gate already exists (`hopper feature-gate`).

### G4 — CU-regression CI (per-commit compare, Quasar-style)

- Quasar's `scripts/bench-tracked-programs.sh` captures CU + size for vault /
  escrow / multisig and diffs HEAD against master in a worktree. Hopper has
  `cu_baselines.toml` + release-gates in `hopper-bench` but no one-command
  per-commit compare lane in the framework repo's CI.
- **Fix.** `hopper bench compare [--base master]` in the CLI, wired to the
  hopper-bench harness; GitHub Action that comments the CU/size delta table on
  PRs; failure threshold from `cu_baselines.toml`.

### G5 — Dispatch and binding codegen: two Quasar micro-techniques (new, 2026-07-07)

- **Fn-pointer-table dispatch.** Contiguous 1-byte discriminators → `[fn; N]`
  table indexed by the disc byte → one `callx` (~5 CU flat) instead of a
  match chain that grows with instruction count. Hopper's dispatcher emits a
  match (single-byte fast path exists, `program.rs::dispatch_body`).
  Candidate: table dispatch under `profile = "tiny"` when discriminators are
  contiguous; measure on an 8+ instruction program (the lazy-dispatch-vault
  is the right fixture).
- **Adaptive context-binding shape.** Quasar's derive emits a direct-bind
  path for COUNT ≥ 8 accounts (skips one `Context → Ctx::new()` layer) and a
  buffered path for small counts — whichever is cheaper per instruction.
  Hopper emits one shape. Worth measuring whether a direct-bind variant pays
  on wide-account instructions (router class — directly relevant to §5).

### G6 — CPI account-meta construction not fused (new, 2026-07-07, micro)

- Quasar reads the 4-byte account header once as u32, shifts out the borrow
  byte, and transmutes a prebuilt `RawCpiBuilder` into the SDK `CpiAccount`
  (layout compile-time asserted). Hopper builds CPI metas field-by-field.
  A few CU per CPI account; matters at router fan-out (× accounts × hops).
  Owner: `crates/hopper-native/src/cpi.rs`, `hopper-runtime/src/cpi.rs`.

### G7 — `get_return_data` zero-fill (new, 2026-07-07, perf nit — soundness verified)

- Hopper's `get_return_data` is **sound** where Quasar's is UB (their #238:
  `assume_init` over uninitialized bytes) — but ours pays a 1 KiB zero-fill
  per call (`data: [0u8; MAX_RETURN_DATA]`). The sound-and-fast shape:
  `MaybeUninit` buffer + expose only the syscall-written prefix. Keep the
  soundness, drop the fill. Owner: `hopper-runtime/src/return_data.rs`,
  `hopper-native/src/return_data.rs`.

### G8 — June-27 audit roadmap items still open

Carried forward (see `HOPPER_COMPETITIVE_AUDIT_2026_06_27.md` for detail):
client PDA auto-resolution from manifest resolvers; buffer-hygiene command
group; stored-bump golden-path example with CU assertion; trace-grade SVM
JSON output; interface-account macro polish (Token/Token-2022 polymorphism);
manifest-backed fail-closed clients; devnet audit one-command runner; docs
consolidation; deploy-size policy; release-blocking operational smoke lane.

Further externally-sourced gaps are folded into §2's whitespace check and
the roadmap (§6): Mollusk/Pinocchio/Anchor comparator staleness (P0 #3),
and the runtime-version sensitivity of all published CU claims.

## 4. Innovations they missed — build these

Open items from the log that this pass re-confirms as differentiators:
I1 (PDA canonicalization by default), I3 (compile-time CU budgets),
I4 (`hopper doctor` shipping the auditor's lints), I5 (heterogeneous
`split_mut!`), I9 (DWARF size profiler fused with measured CU), I14
(adversarial Miri suite), I16 (field behaviors — macro attachment remaining),
I17 (`wire:v3` unified fingerprint).

New this pass:

### I18 — Program-wide tuned intrinsics (`hopper-builtins`)

See G1. The bar is lower than first read: Quasar's
`solana-compiler-builtins` overrides only `memcmp`, gets the sign wrong on
mismatch, and its `target_arch = "bpf"` gate never fires on the default
`cargo build-sbf` route — their published numbers never included it.
Hopper's `hopper-builtins` (opt-in `builtins` feature) ships
ordering-correct `memcmp`/`bcmp`/`memcpy`/`memset` overrides gated
`target_os = "solana"`, live on the build-sbf route — ground Quasar does
not hold. `memmove` is deliberately not overridden (the toolchain shim is
already the right shape). Still to do: publish the CU model per size
class, smoke-test the duplicate-symbol risk against the GLOBAL toolchain
shims, and verify with the Miri suite
(I14) plus a differential fuzz against the naive implementations.

### I19 — CU-regression CI as a product feature

See G4. No framework ships this for *user programs*: `hopper init` templates
get a ready-made GitHub Action that fails a PR when a tracked instruction's
CU or binary size regresses past thresholds. Quasar has the shell script for
their own repo; nobody hands it to program authors as a first-class feature.

### I20 — Adversarial suite over competitor bug classes (safety marketing that's also real)

Turn every open competitor unsoundness class into (a) a Hopper regression
test proving the class can't occur here, and (b) a `hopper doctor` lint (I4)
so user code can't reintroduce it. Seed list from Quasar's tracker
(2026-07): CPI-return `assume_init` UB (#238/#234 — Hopper verified sound
2026-07-07), self-close imbalance (#240), migration stale-state (#239),
remaining-accounts capacity lies (#242), duplicate-account aliasing in raw
handlers. Publish the matrix ("bug class → how Hopper makes it
unrepresentable") as a docs page. No competitor can copy this without first
admitting the bug class.

### I9 (update 2026-07-07) — the profiler bar moved

Quasar's `quasar-profile` now does per-function CU breakdown with delta
tracking and interactive flamegraphs from DWARF/ELF. The leapfrog for
`hopper profile` is fusing **measured Mollusk CU per instruction** with
DWARF size attribution and `--diff <old.so>` in one report — bytes *and*
CU per function per instruction, which their static view cannot produce.

### I21 — Fuzz-harness generation from Hopper manifests (the Trident gap)

Trident (Ackee) — the only Solana fuzzing framework — **requires an Anchor
IDL**, locking out every zero-copy framework. Hopper manifests already
carry account layouts, instruction account lists, constraints, and layout
fingerprints: exactly the metadata a fuzz harness needs. `hopper fuzz init`
generates a Mollusk-driven harness per instruction (arbitrary account
bytes + arbitrary ix data → must return clean `Err`, never panic/OOB —
the I13 hostile-metadata property generalized to *user programs*), plus
declared-vs-actual write-set checks powered by I7 touch maps and I12
policies. No competitor can generate this without first building both a
manifest format and a borrow ledger.

### I22 — In-place growable zero-copy collections (answer star-frame's one good idea)

star-frame's unsized types (`UnsizedList`/`UnsizedMap` that shift bytes in
place inside a fixed buffer) are the only competitor take on growable
zero-copy state. Hopper's bounded tails + 8 fixed-capacity collections
cover most needs with corruption-hardening star-frame lacks; the remaining
piece is a shift-in-place growable list/map over the segment system —
implemented against the borrow ledger (leases invalidated on shift), with
the same parse-don't-validate constructors and hostile-metadata proptests
as the existing collections. Evaluate demand first: it's a real
differentiator for orderbook/registry-shaped state, but the safety
machinery must come along or it's a footgun factory (star-frame ships it
unaudited).

### I23 — Market sBPF v3 readiness first

SIMD-0178/0179 (static syscalls, no relocations) + SIMD-0189 (strict ELF)
land via toolchain; SIMD-0500 proposes disabling v0–v2 deploys. No
framework markets v3 readiness today. Hopper already has the
cluster-feature-gate precedent (`hopper feature-gate`, SIMD-0321): extend
it to report sBPF-version deployability per cluster, add a v3 lane to CI
and the bench matrix (static syscalls should shrink binaries — measure
against G2), and publish the story before anyone else claims it.

## 5. The Autobahn-class head-to-head

Goal: a router/executor-grade parity benchmark — the program class where
framework overhead actually shows (dynamic remaining accounts, multi-hop CPI
fan-out, token balance deltas, minimal state).

**Verified 2026-07-07: no public router-class framework head-to-head
exists.** Autobahn is `blockworks-foundation/autobahn` (a DEX aggregator,
unrelated to Quasar); Quasar/Blueshift publish no comparative CU table at
all; the nearest third-party benchmark is a memo program (Anchor 649→281 CU
hand-optimized / Pinocchio ~108 / sBPF asm 104 — no Quasar column). The
head-to-head the project owner described is therefore an **empty category
Hopper can create and own**: first published, reproducible, provenance-
pinned router benchmark across Hopper / Pinocchio / Quasar / Anchor.
(Watch item: Blueshift's `beethoven` repo is a client-side CPI router — the
natural seed if they ever publish their own.)

### What autobahn actually is (verified against source + live mainnet, 2026-07-07)

`blockworks-foundation/autobahn` — Mango/Blockworks' DEX aggregator.
**AGPL-3.0**: our benchmark implementation must be **clean-room behavioral
parity, never a source copy**. Maintenance-frozen since ~Q1 2025 (last push
2025-03-17), which makes it a stable, inspectable reference. Live program
`AutobNFLMzX1rFCDgwWpwr3ztG5c1oDbSrGq7Jj2LgE`, ELF ≈ 172 KiB, deploy rent
1.2277 SOL. Written in **raw `solana-program` 1.17** (not Anchor, not
Pinocchio).

The executor's 7 instructions, with `data[0]` packing discriminator (low
nibble) + router version (high nibble): `ExecuteSwapV3` (primary multi-hop
path), `ExecuteSwapV2`, `OpenbookV2Swap`, `ChargeFees`/`ChargeFeesV2`,
`CreateReferral`/`WithdrawReferral`. The `ExecuteSwapV3` hot loop per hop:
slice `accounts[i..i+count]` (heap Vec), patch the previous hop's output
amount into this hop's ix data at `in_amount_offset`, build an `Instruction`
(more heap), read token balance **before and after** the venue CPI to derive
`out_amount`, `invoke`, emit a `SwapEvent` via `sol_log_data` (3,000-byte
stack buffer), and finally gate `out < min_out_amount`. Its dominant
framework-independent CU sinks: **full 165-byte `spl_token` `Pack::unpack`
2–3× per hop** for balance reads (a zero-copy `amount @ offset 64` read
collapses this), per-hop heap allocation, and CPI marshalling — exactly the
axes where Hopper/Pinocchio/Quasar-class runtimes win.

CU per swap is **published nowhere**, and no Pinocchio/Quasar rewrite with
numbers exists — reconfirmed by a second independent search. The nearest
prior art: febo's `eisodos` (entrypoint micro-bench), OtterSec's unpublished
Anchor-V2 benchmark plan (#4355), and our own vault matrix.

### The benchmark: `hopper-bench` "router parity lab"

Step 0 — harness upgrade: bump `mollusk-svm` 0.10.3 → **0.13.4**
(`anza-xyz/mollusk`, agave 4.0) for `MolluskComputeUnitMatrixBencher`
(column-per-program matrix), and pin `SVMFeatureSet` to mainnet-active
**including SIMD-0339/0268** so CPI pricing matches production.

1. **Mock AMM fixture.** One tiny `mock_amm` program (single `swap` ix,
   deterministic constant-rate vault-to-vault token movement), built once,
   `.so` committed as a fixture. Every framework's executor CPIs into the
   *same* mock — isolates framework overhead from venue logic and keeps runs
   offline-reproducible (real venues can't be fixtured deterministically).
2. **Shared behavioral contract** (same hard rule as the vault bench):
   parse hops from the same varint wire format, forward hop N's out-amount
   into hop N+1's in-amount, CPI the mock AMM per hop, same balance-delta
   derivation, same `min_out` slippage gate, same account ordering/seeds/
   mints. All emit the same event or none (prefer none for clean deltas).
   A framework that can't express a row gets `n/a`, never a synthesized
   substitute.
3. **Rows:** `swap_1hop` / `swap_2hop` / `swap_3hop` (the slope across hops
   is the per-CPI + per-account marshalling overhead — the 946-CU base × N
   cancels in deltas); `charge_fees` (SPL transfer + a Token-2022
   `transfer_checked` variant); `zero_copy_balance_read` (isolates the token
   read path where autobahn burns `Pack::unpack`); and a **safety gate row**:
   the `min_out` violation MUST be rejected by every framework (excluded
   from CU totals on failure, mirroring `unsigned_withdraw_rejected`).
4. **Columns:** Hopper (eager path, `DynCpi` for hop fan-out), in-tree Anza
   Pinocchio, Quasar (upstream checkout, pinned SHA), Anchor (explicit
   opt-in, for the headline table). Hopper is baseline; report
   `cu_delta = other − hopper`, binary bytes/KiB, `.text` stack frame,
   deploy rent in SOL, and the `solana-verify` build hash.
5. **Pinning:** one shared `[profile.release]` (`opt-level=3, lto=fat,
   codegen-units=1, strip=true`), same platform-tools + sBPF target for all
   columns, competitor SHAs in `competitors.lock`, seeds fixed, noise floor
   stated (<50 CU ≈ Mollusk run noise).
6. **CI:** docker build of all columns + matrix run + markdown/JSON/CSV
   artifacts; regression tolerance from `cu_baselines.toml`; publish the
   table in `hopper-bench` and summarize (with provenance) in
   `BENCHMARKS.md`.

Hopper-side work the rows will exercise (and where we expect to win):
zero-copy SPL token views (no unpack), heap-free `DynCpi` hop building vs
autobahn's per-hop `Vec`s, fused header validation at parse, stored-bump
PDA fee accounts, and — after G1/G5/G6 land — word-compare keys, adaptive
binding for wide account lists, and fused CPI metas.

Design constraints already known from the internal baseline:

- Same shared-contract discipline as the vault bench (`METHODOLOGY.md`):
  behavioral equivalence, same seeds/fixtures/toolchain, `n/a` over synthesis.
- Hopper primitives that map to the workload: `DynCpi<'a, MAX_ACCTS,
  MAX_DATA>` (heap-free dynamic CPI), remaining-accounts modes
  (strict/passthrough/typed/lazy), SPL token views + CPI builders,
  `hopper_lazy_entrypoint!` for wide-account-list instructions, stored-bump
  PDA verification, segment borrows for in-place fee/stats state.
- Measure: CU per swap leg (1/2/3-hop), CU for the quote/validation failure
  path, binary size, stack frames, and the safety-correctness gate (a route
  that violates min-out MUST be rejected by all frameworks).

## 6. Priority roadmap (post-research)

The strategic read that orders everything: **Anchor v2 (Pinocchio-based,
Quasar-level CU in their own benches) will erase every framework's "10×
cheaper than Anchor" headline when it ships.** What it cannot erase:
Hopper's sovereign substrate, the borrow-ledger family (segments, touch
maps, write policies, receipts), first-class migrations/schema/clients,
and whoever owns the reference benchmark for real workloads. Hence:

### P0 — this cycle

1. **Router parity lab** (§5). The head-to-head the owner asked for, in the
   category nobody has published, before OtterSec's Anchor-v2 table lands.
   Deliverables: `mock_amm` fixture, four clean-room executors, Mollusk
   0.13.4 matrix, published table + provenance + solana-verify hashes.
2. **G1 + I18 — fast key compares & `hopper-builtins`.** Whole-program CU
   win that improves every row of the router lab; smallest
   effort-to-impact ratio on the list.
3. **Bench-stack refresh** (part of step 0 of §5 but stands alone): Mollusk
   0.10.3 → 0.13.4, Pinocchio comparator 0.10 → 0.11.2, Anchor comparator
   0.31.1 → 1.1.2, `SVMFeatureSet` = mainnet + SIMD-0339. Note:
   BENCHMARKS.md currently calls 0.31.1 "the latest stable line" — already
   stale at publication (Anchor 1.0 shipped 2026-04-02); the fix is a
   re-run, not a text edit.

### P1 — next

1. **I1 — PDA canonicalization by default** + stored-bump golden path (the
   one Quasar DX advantage on the hot path that's also a safety win).
2. **I19 — CU/size regression CI as a user-facing feature** (`hopper init`
   template ships the Action; thresholds from `cu_baselines.toml`).
3. **G5/G6/G7 micro-CU batch** (fn-pointer dispatch under `tiny`, adaptive
   binding shape, fused CPI metas, return-data fill) — measured against the
   router lab, adopted only where the delta is real.
4. **G8 client polish: PDA auto-resolution + fail-closed decode** — the
   visibility work for whitespace #1, which Hopper already owns technically.

### P2 — strategic

1. **I21 — manifest-driven fuzz-harness generation** (unowned whitespace,
   deep moat synergy with I7/I12/I13).
2. **I16 behaviors macro completion; I17 `wire:v3`; I14 Miri suite; I20
   competitor-bug-class matrix** — the safety-story compounding items.
3. **I22 growable collections (evaluate), I23 sBPF v3 story, I9 profiler
   CU+size fusion** — differentiators to sequence as bandwidth allows.

**Positioning shifts to make in docs/website now** (no code): lead with
"the only sovereign, audited zero-copy framework with shipped clients/IDL";
stop leading with the Anchor-multiple (it has a shelf life); add the
deploy-rent economics table; state the naming distinction from Sanctum's
jiminy where the stdlib is mentioned.


---

## 7. Build-pass outcomes — 2026-07-07 (measured)

The P0 items were implemented, adversarially reviewed (6 confirmed
findings fixed, 3 refuted), and measured the same day. Workspace suite:
1,295 passed / 0 failed.

- **G1 shipped.** All 32-byte key compares now funnel through 4x-u64 word
  compares (manual `PartialEq` on both Address types; macros, `has_one`,
  token/TLV checks rerouted; native `address_eq` rewritten). **Honest
  measurement:** the vault matrix is unchanged (466/107/564/1714/488 —
  deposit +1 CU is noise) because LLVM was already inline-expanding the
  vault's hot compares. The win is structural (codegen-independent fast
  path, eliminated copies, owner-check fn no longer optimizer-dependent);
  the April "~40 CU `check_keys_eq`" primitive figure must be re-measured
  through a Mollusk micro-row before any CU claim is published.
- **G7 shipped.** Both `get_return_data` paths are MaybeUninit
  prefix-only — sound where Quasar is UB (#238), no 1 KiB zero-fill.
- **I18 shipped (experimental, opt-in).** `hopper-builtins` with
  ordering-correct memcmp/bcmp/memcpy/memset. Two hard-won facts:
  (a) `rust-lld` REFUSES the strong-vs-strong collision — the feature
  requires `RUSTFLAGS="-C link-arg=--allow-multiple-definition"`, which
  also proves Quasar's reference crate cannot link on the build-sbf route
  at all; (b) the overrides need `#![no_builtins]` or LLVM loop-idiom
  recognition rewrites them into self-recursive libcalls (P1, caught by
  adversarial review with an empirical reproduction, fixed). Vault CU is
  identical with the feature on; size +0.41 KiB; value is runtime-length
  mem ops (LLVM lowers those to `bcmp` — which platform-tools doesn't
  even provide).
- **I20 shipped.** 13 pinned tests across two suites, and the authoring
  pass found + fixed a real bug: `safe_close` accepted an aliased
  destination and silently burned the drained lamports (the exact Quasar
  #240 shape). Guard + pin test landed.
- **Router parity lab: first numbers.** Contract v1 (lamport mock-AMM,
  measured amount forwarding, min-out gate), Mollusk 0.10.3, 8 seeds:

  | Row | Hopper | Pinocchio (hand-written) |
  |---|---:|---:|
  | swap_1hop | 1,616 CU | 1,523 CU |
  | swap_2hop | 3,120 CU | 2,975 CU |
  | swap_3hop | 4,625 CU | 4,431 CU |
  | Binary | **10.09 KiB** | 10.98 KiB |
  | min-out gate | rejected | rejected |

  Hopper is ~50 CU/hop (~3%) over raw Pinocchio while smaller in binary
  and carrying the framework surface. The per-hop delta is the G5/G6
  target list (adaptive binding, fused CPI metas). The Quasar column is
  open: write `quasar-router` against `ROUTER_CONTRACT.md` and the
  harness picks it up. Results:
  `hopper-bench/results/router-parity-2026-07-07-first/`.
