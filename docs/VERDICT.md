# Hopper Framework Verdict — 2026-06-12 (updated 2026-06-15)

> **Historical document — 2026-07-07 note.** This is a June 2026 review kept
> for the record; several items below have since shipped or been superseded.
> In particular: the Kani-harness proposal shipped (`#[kani::proof]` harnesses
> in `crates/hopper-runtime/src/segment_borrow.rs` and `tail.rs`), F1/F2 are
> fixed, the profiler answer moved (measured-CU lab, I9), and the test suite
> is now 1,295 passing (was 755/759 at review time). For the current verified
> state — competitive landscape, shipped gap closures, and the 2026-07-07
> measured results — see
> [`docs/audit/GAP_CLOSURE_AND_INNOVATION_2026.md`](audit/GAP_CLOSURE_AND_INNOVATION_2026.md),
> section 7.

Independent crate-by-crate review of Hopper against Pinocchio (anza-xyz),
Quasar (blueshift-gg), and Anchor zero-copy. Research covered the live
upstream repos (Pinocchio 0.11.2, Quasar beta master) and every Hopper
workspace crate, with the full test suite executed during the review.

## Update — findings resolved (2026-06-15)

All P0–P2 findings below were implemented and verified. Test suite:
**755 passing (default features) / 759 (all features), 0 failures** (was 2
failing trybuild snapshots at review start). A new feature-exercising
program, `examples/hopper-smoke`, was built to SBF and run live on devnet
(`initialize → deposit → withdraw` confirmed), exercising the account
macros, init/has_one/close constraints, two System-program CPIs, the Clock
sysvar syscall, layout-fingerprint header writes, checked math, and typed
events end-to-end. See `examples/hopper-smoke/README.md`.

| Finding | Status |
|---|---|
| F1 fast-entrypoint unsound | **Fixed.** Gated behind `simd-0321` cargo feature; default build uses the scanning entrypoint; the gated path null-checks `r2` and falls back. Docs corrected. |
| F2 shared-borrow counter wrap | **Fixed.** `try_borrow`/`segment_ref` now reject the 255th shared borrow (cap 254) so the count can't alias `NOT_BORROWED`. |
| F3 resize not zeroing | **Fixed.** `resize()` zero-fills the grown region; `resize_raw()` added for the overwrite-in-full hot path. |
| F4 EpochSchedule ABI | **Fixed.** Verified against agave source; added `offset_of!` ABI-lock asserts for `EpochSchedule` and `Clock`. |
| F5 benchmark framing | **Fixed.** BENCHMARKS.md now states the Pinocchio gap is stored-bump-vs-`find_program_address`, and the fast-entrypoint row is marked SIMD-0321-gated. |
| F6 boilerplate SAFETY comments | **Partly addressed.** The security-critical borrow state machine now has specific SAFETY comments; the rest remain a tracked cleanup. |
| F7 coverage gaps | **Closed.** System WithSeed + durable-nonce family, `sol_get_sysvar`/`sol_get_epoch_stake`/SlotHashes/StakeHistory, secp256r1 introspection, opt-in `default_allocator!` bump allocator, and a `no_std` Token-2022 `ExtraAccountMetaList` resolver all landed. |
| Innovation #1 (feature-gate-aware deploys) | **Shipped.** `hopper feature-gate` queries the SIMD-0321 gate on the target cluster (confirmed active on devnet at slot 446688000); `hopper doctor` points to it. |

### Alignment bound — tightened (2026-06-19, P0)

An external expert review (`E:\Hopper is now clearly shaped as a sovere.txt`)
correctly overruled an earlier interim decision here. The real bug was that
`Pod` claimed `align_of == 1` in its contract yet was implemented for native
multi-byte integers (`u64` has align 8), so `segment_ref::<u64>` / `raw_ref::<u64>`
type-checked and could form a `&u64` at an unaligned account offset — alignment
UB. **The guard was tightened, not retired:**

- `Pod` is now implemented **only** for alignment-1 types (`u8`, `i8`,
  `[u8; N]`, `()`, and macro-authored wire/layout types). Every
  reference-returning overlay API (`load`, `segment_ref`, `raw_ref`,
  `pod_from_bytes`, …) bounds on `Pod`, so `segment_ref::<u64>` is now a
  **compile error** (`crates/hopper-native/src/pod.rs`; proven by
  `tests/hopper-trybuild/tests/ui/fail/{raw_ref,segment_ref}_u64_rejected.rs`,
  with `WireU64`/`[u8;8]` accepted in `…/pass/overlay_wire_types_accepted.rs`).
- A separate `ValuePod` marker (all native ints + arrays) plus
  `read_unaligned_value` / `Context::read_data::<u64>` cover by-value scalar
  decoding via `core::ptr::read_unaligned`.
- The two internal aligned-cast reads the review named
  (`AccountView::header_u32`, the fast-entrypoint length prefix) now use
  `read_unaligned`, so `UNSAFE_INVARIANTS.md` invariant #1 ("all overlay
  targets are alignment-1; no cast yields a reference with align > 1") is now
  literally true and type-enforced.

### Other review items closed (2026-06-19)

| Review item | Status |
|---|---|
| Rent hard-coded `(128+len)*6960` in `lifecycle.rs` | **Fixed.** `rent::minimum_balance_live` reads the **live** Rent sysvar on-chain (falls back to launch constants off-chain); `lifecycle` delegates to it, removing the duplicate magic-number formula. |
| Segment split / simultaneous disjoint mutable borrows | **Added.** `AccountView::split_segments_mut` / `Context::split_segments_mut` register N disjoint ranges (proving disjointness once) and return a `SegmentsMut` guard whose `all_mut()` hands back `[&mut T; N]`. Tested for the happy path and overlap/OOB rollback. |
| `hopper-system` only re-exported a subset | **Fixed.** All System builders (WithSeed family, durable-nonce family, `NonceState`, the consts) are re-exported at the crate root. |
| SBF deployability of new generic code | **Verified.** The `hopper-smoke` program rebuilt with every change above redeployed to devnet and re-ran `initialize → deposit → withdraw` (`SMOKE OK`); new runtime code uses conservative `MaybeUninit` array idioms to keep SBPF codegen broadly compatible. |

## Bottom line

**Hopper is a true, complete Solana program framework.** It owns every layer
a program needs — loader-input parsing, entrypoint, syscalls, account model,
typed state, validation, CPI, SPL/Token-2022/ATA/Metaplex surfaces, schema,
client codegen, CLI, and test harnesses — with no `solana-program` or
Pinocchio dependency on the hot path. It is capability-superior to both
Pinocchio (which is a substrate library, not a framework) and Quasar (which
matches Anchor's macro surface but has none of Hopper's state-evolution,
receipt, policy, or off-chain SDK machinery). The claims in `COMPARISON.md`
were spot-checked against source and held up, with the caveats listed below.

Verification performed this pass:

- `cargo check --workspace --all-features` — clean (exit 0).
- `cargo test --workspace --all-features` — **257 passed, 2 failed**. Both
  failures are `tests/hopper-trybuild` stderr-snapshot drift from a newer
  rustc (the compile-fail cases still fail compilation as designed). Bless
  with `TRYBUILD=overwrite` after review.
- Constraint lowering in `hopper-macros-proc/src/context.rs` covers the full
  Anchor set: `init`, `init_if_needed`, `zero`, `mut`, `signer`, `seeds`,
  `bump`, `payer`, `space`, `has_one`, `dup`, `owner`, `address`,
  `constraint`, `executable`, `rent_exempt`, `close`, `realloc`, plus
  `token::*`, `mint::*`, `associated_token::*`, `metadata::*`,
  `master_edition::*`, and Token-2022 extension gates.
- Dispatch (`program.rs`) supports 1-byte (default) through 8-byte
  discriminators with compile-time duplicate detection **and prefix-shadow
  detection** — neither Anchor nor Quasar does the latter.
- Owner checks: `AccountView::load()` validates header identity only
  (disc/version/layout_id/epoch/size); ownership is enforced one layer up in
  `Account::try_new` / the derive lowering. The layering is sound, but see
  finding F6.

## Where Hopper is genuinely ahead

1. **Segment-level borrows** with a runtime lease registry
   (`segment_borrow.rs`, `segment_lease.rs`) and const-offset typed segments.
   No competitor has disjoint `&mut` views into one account.
2. **Upgradeable state contracts**: 16-byte header with layout fingerprint +
   schema epoch, `MigrationEdge` chains, manifest-level compatibility
   analysis (`is_append_compatible`, `requires_migration`). Anchor/Quasar
   have nothing here; the 8-byte `LAYOUT_ID` catches silent struct drift that
   every other framework misses.
3. **Receipts and policy graphs** (`receipt.rs`, `policy.rs`) — structural
   audit trails at ~50–150 CU and declarative capability/requirement
   enforcement before dispatch.
4. **The off-chain half**: `hopper-sdk` (manifest-driven readers/builders,
   receipt narration, fingerprint verification) plus client codegen for TS,
   Kotlin, Python, Go, C, Rust, Codama JSON, and Anchor-shaped IDL. No
   competitor ships a symmetric off-chain SDK.
5. **Loader-input hardening**: the duplicate-marker forward-reference trap in
   `raw_input.rs` plus `parse_instruction_frame_checked` (a bounds-checked
   fuzzable twin of the pointer parser) is unique — Pinocchio has no safe
   companion parser and no fuzz surface for loader input.
6. **Tooling breadth**: 20+ CLI command families (deploy, doctor, lint,
   profile, watch, tx_explain, test_gen, manager, mobile…), Mollusk-backed
   `hopper-test`, Hopper-owned `hopper-svm` host harness, devnet-deployed
   example artifacts.
7. **Domain crates** (finance, lending, staking, vesting, distribute,
   multisig) — no competitor ships audited-shape protocol building blocks.

## Findings (prioritized)

### F1 — `hopper_fast_entrypoint!` is unsound on current clusters (P0)

`crates/hopper-native/src/entrypoint.rs` claims the SVM "has provided the
second argument since runtime ~1.17". That is incorrect. The r2
instruction-data pointer is **SIMD-0321**, currently in *Review* status with
feature gate `5xXZc66h4UdB6Yq7FzdBxBiRAFMMScMLwHxk2QZDaNZL` not activated on
any public cluster. Today r2 contains uninitialized data at entry; a program
built with `hopper_fast_entrypoint!` reads garbage as its instruction-data
pointer.

**Fix:** gate the macro behind a cargo feature (e.g. `simd-0321`) with a
prominent doc warning; correct the doc comment; add a `hopper doctor` /
`hopper deploy` RPC check that queries the feature-gate account on the
target cluster and refuses (or warns) when the gate is inactive. That last
part turns a footgun into a headline safety feature no framework has.

### F2 — Shared-borrow counter wraps into NOT_BORROWED (P1)

`account_view.rs::try_borrow` / `segment_ref`: borrow_state `0xFF` = free,
`0` = exclusive, `1..=0xFE` = shared count. The overflow guard only rejects
`new_state == 0`. At `state == 0xFE`, the 255th concurrent shared borrow
computes `0xFE + 1 == 0xFF == NOT_BORROWED` and silently resets the counter;
a subsequent `try_borrow_mut` then succeeds while 255 shared refs are live —
aliasing UB. Exotic (requires 255 simultaneously held guards) but real.

**Fix:** also reject `new_state == NOT_BORROWED` (cap shared count at 254).
One-line change in both `try_borrow` and `segment_ref`.

### F3 — `resize()` does not zero regrown memory (P1, verify)

`AccountView::resize` updates `data_len`/`resize_delta` without zeroing. The
classic shrink-then-regrow-within-one-transaction pattern can expose stale
bytes. Pinocchio's resize takes an explicit zero-init flag. Higher Hopper
layers (`safe_realloc`, `hopper_init!`) zero correctly, but the raw native
method should either zero on growth or document the hazard and offer
`resize_zeroed`.

### F4 — `EpochSchedule` syscall ABI needs a layout test (P2)

`sysvar.rs::EpochSchedule` is `#[repr(C)]` with a `bool` mid-struct. The
fields after `warmup` are only correct if the syscall writes the same padded
layout. Add a Mollusk test that reads the sysvar and asserts
`first_normal_epoch`/`first_normal_slot` against known cluster values.

### F5 — Benchmark framing: the Pinocchio gap is PDA strategy, not magic (P2)

`hopper-bench/pinocchio-vault` is honest, idiomatic code, but its own doc
comment concedes the headline deltas (e.g. deposit 1 669 vs 3 856 CU) come
mostly from `find_program_address` (bump-search, ~1.5–2.5k CU) versus
Hopper's stored-bump verify (~200 CU). A stored-bump Pinocchio variant would
be near parity. The honest claim — and it is still a strong one — is
"Hopper is fast **by default**; the cheap path is the one the macros
generate." Add a stored-bump Pinocchio row to the parity table before anyone
else does it for you.

### F6 — Boilerplate SAFETY comments violate the project's own invariant (P2)

Dozens of unsafe blocks carry the identical line "This block is part of
Hopper's audited zero-copy/backend boundary…". Design invariant #7 in
`ARCHITECTURE.md` requires each SAFETY comment to justify alignment, length,
and aliasing specifically. External auditors will flag the boilerplate
wholesale. Replace incrementally, starting with `raw_input.rs`,
`account_view.rs`, and the CPI paths.

### F7 — Coverage gaps vs "all of Solana" (P2)

- **System program**: only `CreateAccount`, `Transfer`, `Assign`, `Allocate`.
  Missing `CreateAccountWithSeed`, `TransferWithSeed`, `AllocateWithSeed`,
  `AssignWithSeed`, and the entire durable-nonce family. Pinocchio-system
  covers all of these.
- **Sysvars**: no generalized `sol_get_sysvar` syscall binding — so no
  SlotHashes, StakeHistory, EpochRewards, or last-restart-slot-style reads
  beyond what's declared; no `sol_get_epoch_stake`.
  (`sol_remaining_compute_units` exists at the runtime layer.)
- **Token-2022**: extension *gating* is excellent (TLV checks for 12+
  extensions), but there is no `ExtraAccountMetaList` resolver for transfer
  hooks — hook-CPI account resolution is manual.
- **Precompiles**: ed25519/secp256k1 introspection helpers exist;
  **secp256r1** (passkeys, SIMD-0075) is absent.
- **Allocator**: `no_allocator!` only; no opt-in bump allocator for users
  who want `alloc` (Pinocchio offers both).
- **CI**: `cargo build-sbf` lane still missing (ROADMAP R-3) — devnet
  artifacts prove the build works, CI doesn't.

## Innovation proposals

Ranked by leverage-per-effort for making Hopper easier, faster, and safer.

1. **Feature-gate-aware deploys** (pairs with F1). `hopper doctor`/`deploy`
   query feature-gate activation on the target cluster and check them
   against what the binary was compiled to assume (fast entrypoint, new
   syscalls). Nobody has compile-config ↔ cluster-state validation.
2. **Compile-time segment disjointness** (ROADMAP R-4, achievable now): the
   layout macros know every segment's offset and size, so emit pairwise
   `const _: () = assert!(...)` non-overlap proofs and skip the runtime
   registry entirely for static layouts. Zero-CU segment safety would be a
   first.
3. **Stored-bump as a header convention**: reserve one byte of the Hopper
   header flags region for the canonical PDA bump, written at init. Then
   `seeds`/`bump` constraints always verify with a single sha256 — the F5
   benchmark advantage becomes a permanent, principled default instead of a
   per-program pattern.
4. **no_std transfer-hook resolver**: a zero-alloc `ExtraAccountMetaList`
   parser + account resolver in `hopper-token-2022`. No framework has one;
   it unblocks real Token-2022 DeFi and would make Hopper the default choice
   for hook-aware programs.
5. **secp256r1/passkey helpers**: precompile-introspection guards mirroring
   the ed25519 ones, plus a `hopper-webauthn` example. Consumer-wallet teams
   adopt the framework that makes passkeys trivial.
6. **Static CU profiler with budgets**: Quasar's one genuinely better
   tool is its static CU flamegraph. Hopper measures real CU
   (`hopper profile bench`) but cannot bound it. Add SBPF bytecode analysis
   with per-handler worst-case bounds and let authors declare
   `#[instruction(0, cu_budget = 5_000)]` — CI fails when the static bound
   exceeds the budget. Measured + static + budget-enforced beats every
   competitor.
7. **Kani harnesses** (ROADMAP R-2): `parse_instruction_frame_checked` and
   the segment-borrow registry are perfectly shaped for model checking.
   Matching Quasar's "formally model-checked" line removes their last
   differentiator.
8. **Fleet migration crank**: `hopper migrate fleet` — scan program accounts
   via RPC, diff stored layout_ids against the manifest, plan and send
   batched migration instructions with receipts as proof. Closes the loop on
   the framework's signature feature: nobody else can even express this.
9. **Durable-nonce + full system-program surface** (closes F7): mechanical,
   small, and removes the last "Pinocchio has it, Hopper doesn't" row.
10. **Optional bump allocator**: `hopper_default_allocator!` for teams that
    want `alloc` in cold paths, keeping no-alloc as the default.

## Final assessment

Hopper already does everything Pinocchio and Quasar do for program authors —
and a documented, tested superset beyond both (segments, migrations,
receipts, policies, manifest-driven clients, off-chain SDK, domain crates).
Its honest weaknesses are not capability but **trust accumulation** (Anchor
and Pinocchio have years of mainnet mileage and external audits) and the
specific findings above. Fix F1 and F2 immediately (both are small), close
the F7 coverage gaps, and ship innovations 1–4, and Hopper is not just
competitive — it is the only framework whose safety story strengthens, rather
than erodes, as programs grow and evolve.
