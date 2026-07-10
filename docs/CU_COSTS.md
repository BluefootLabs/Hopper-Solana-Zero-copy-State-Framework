# Compute-unit cost reference

The per-primitive cost table Hopper developers budget against. Every number
in the tables below is a measured value from one reproducible Mollusk run —
same artifact, same harness, same day. Where an operation is not yet covered
by the measurement lab, this page says so instead of estimating.

If a CU number elsewhere in the docs or marketing drifts from this table,
the table wins. Cross-framework comparisons (Anchor, Pinocchio, Quasar) are
not made on this page; those are measured separately, with their own
provenance, in `BENCHMARKS.md`.

## Provenance

| Field | Value |
| --- | --- |
| Measured | **2026-07-09** |
| Framework | commit `5819a63`, clean tree |
| Benchmark program | `hopper-bench` on-chain program (sibling benchmark repo). It **path-depends on this framework**, so the artifact embeds this commit's codegen: `hopper_bench.so`, 18,088 bytes, sha256 `1652867bd3f47c7fb688ed29bc0fc935f2b057e91f58fc0007454da5f40b036b` |
| SVM harness | `mollusk-svm 0.10.3` (validator-free, default feature set), driven by the `primitive-bench` host runner |
| Toolchain | rustc 1.96.0; `cargo-build-sbf` 4.0.0 (platform-tools v1.53); release profile |
| Runner command | from the `hopper-bench` repo root: `cargo build-sbf --manifest-path hopper-bench/Cargo.toml`, then `cargo run --manifest-path primitive-bench/Cargo.toml --release -- --out-dir results/primitive-bench-2026-07-09` |
| Raw artifacts | `hopper-bench/results/primitive-bench-2026-07-09/primitive-cu.{md,csv}` — the CSV (`disc,primitive,whole_ix_cu,bracketed_cu,net_cu,...`) is the machine-readable source for any tooling that consumes this table |

**Method.** Each primitive is one instruction of the benchmark program
(discriminators 0–21), executed under Mollusk. Two columns per row:

- **Net CU** — the delta between the two `sol_log_compute_units()` calls
  bracketing the primitive, minus the empty-bracket overhead measured by a
  dedicated probe (disc 21: **101 CU** in this run). This is the closest
  estimate of the primitive alone, and the number to budget against.
- **Whole-ix CU** — `compute_units_consumed` for the entire instruction:
  entrypoint account parse, dispatch, fixture checks, and the measurement
  logging (four log-class syscalls, ≈ 400 CU) all included. Useful as an
  upper bound and when comparing against end-to-end instruction numbers.

The runner executes every disc twice and refuses to publish if the two runs
disagree, so each number is CU-exact for this artifact. Treat net values of
0–2 CU as *at measurement resolution*: at that scale the bracket cannot
distinguish the primitive from zero.

CU costs are toolchain- and runtime-relative (identical code has been
observed to move between Solana runtime versions), so compare numbers only
within one provenance block. The prior run of this same lab (2026-07-07,
same harness and toolchain, framework commit `fa2bfdb`) is retained at
`hopper-bench/results/primitive-bench/` so deltas stay checkable. A refresh
on the agave-4.0 Mollusk stack (0.13.x) is queued.

## Per-primitive costs (Mollusk, 2026-07-09)

| Disc | Operation | Net CU | Whole-ix CU | Category | Notes |
| --- | --- | ---: | ---: | --- | --- |
| 0 | `check_signer` | 3 | 461 | Validation | is-signer flag check |
| 1 | `check_writable` | 3 | 462 | Validation | is-writable flag check |
| 2 | `check_owner` | 14 | 472 | Validation | owner vs program id, 4×u64 word compare |
| 3 | `Vault::load()` (T1 full check) | 33 | 495 | Account loading | owner + disc + version + layout_id + size |
| 4 | `check_keys_eq` | 15 | 491 | Validation | two 32-byte keys, word compare |
| 5 | `Vault::overlay()` (57 B) | 2 | 475 | Memory access (Tier A) | header + layout_id + bounds check |
| 6 | `write_header` | 6 | 483 | Account init | write the 16-byte Hopper header |
| 7 | `zero_init` (57 B) | 21 | 498 | Account init | zero the account, then write header |
| 8 | `check_account_fast` | 5 | 462 | Validation (fast path) | fused fast-path account check |
| 9 | `emit_event` (32 B payload) | 240 | 688 | Events | one `sol_log_data` segment, syscall included |
| 10 | `TrustProfile::load` (Strict) | 29 | 493 | Trust loading | full cross-program trust validation |
| 11 | `pod_from_bytes` (57 B) | 2 | 475 | Memory access (Tier B) | bounds-checked `Pod` cast |
| 12 | `StateReceipt::begin + commit` | 1,915 | 2,395 | Receipts | snapshot + diff + encode cycle |
| 13 | `read_layout_id` + compare | 6 | 479 | Fingerprint check | 8-byte layout fingerprint verify |
| 14 | `StateSnapshot::capture + diff` | 0 † | 476 | State tracking | see footnote — optimizes out in this shape |
| 15 | `overlay_mut` + field write | 4 | 482 | Memory access (Tier A mut) | mutable overlay + one field set |
| 16 | `raw_cast_baseline` (unsafe ptr) | 2 | 475 | Competitor baseline | size check + pointer cast only |
| 17 | `StateReceipt` (enriched fields) | 1,917 | 2,396 | Receipts | + phase, compat_impact, validation, migration |
| 18 | `receipt + emit` (64 B log) | 2,231 | 2,711 | Receipts | begin + set + commit + `to_bytes` + emit |
| 19 | `proc_macro_typed_dispatch` | 183 | 651 | Macro dispatch | full `#[hopper::program]` path: dispatch + binding + u64 decode + handler |
| 20 | `write_proc_header` (probe) | — | 73 | Harness probe | unbracketed: entrypoint + dispatch + header write, **zero logging** |
| 21 | `measurement_overhead` (probe) | 0 | 444 | Harness probe | empty bracket = 101 CU, subtracted from every net figure |

† **Disc 14 footnote.** The measured region captures a snapshot of an
unchanged account, diffs it, and discards the result. At this commit the
compiler inlines the whole sequence and eliminates it as dead code, so the
bracket measures 0 CU. Read it as "a discarded snapshot+diff costs nothing",
not "state tracking is free": when the diff result feeds a receipt or a
branch, you pay the receipt-row costs below. The 2026-07-07 run measured
this same source at 26 CU net — the elimination is new codegen behavior at
this commit, not a measurement trick.

**Why whole-ix is ~450 CU when net is 3.** The whole-instruction column
carries the measurement scaffolding: two `msg!` marker logs plus the two
bracket syscalls (~400 CU of log-class syscalls at 100 CU each) and the
entrypoint/dispatch path. The two probes bound it: disc 21 (no accounts,
empty bracket, full logging) costs 444 CU whole; disc 20 (one account,
dispatch + header write, **no logging at all**) costs **73 CU whole** — the
leanest complete Hopper instruction in this lab. Production instructions
carry no brackets, so their whole-instruction cost is much closer to the
disc-20 shape; see the end-to-end parity numbers in `BENCHMARKS.md` (e.g.
vault withdraw 486 CU, auth-fail rejection 66 CU, whole instructions
including PDA verification).

## Memory-access tiers

Same run, net CU:

| Tier | Operation | Net CU | What you get |
| --- | --- | ---: | --- |
| Raw (unsafe) | `raw ptr cast` | 2 | size check + pointer cast only — the competitor baseline |
| B (pod) | `pod_from_bytes` | 2 | bounds-checked typed view |
| A (safe) | `Vault::overlay()` | 2 | header + layout_id + bounds check |
| A (mut) | `overlay_mut` + field set | 4 | mutable overlay + one write |
| Full load | `Vault::load()` | 33 | owner + disc + version + layout_id + size |
| Strict trust | `TrustProfile::load` | 29 | full cross-program trust validation |

The measured claim this table supports: **the safe, validated overlay costs
the same as a raw unsafe pointer cast** — 2 CU net each, in the same run,
at measurement resolution. Validation you opt into scales the cost: the
full protocol-grade load is 33 CU net.

## Validation costs

Same run, net CU:

| Check | Net CU | Purpose |
| --- | ---: | --- |
| `check_signer` | 3 | verify account is a signer |
| `check_writable` | 3 | verify account is writable |
| `check_account_fast` | 5 | fused fast-path account check |
| `check_owner` | 14 | owner vs program id (word compare) |
| `check_keys_eq` | 15 | compare two account keys (word compare) |
| Full T1 load | 33 | all checks: owner + disc + version + layout_id + size |
| Strict trust load | 29 | `TrustProfile` with all validations |

## Receipts and state tracking

Same run, net CU:

| Operation | Net CU | Notes |
| --- | ---: | --- |
| `read_layout_id` + compare | 6 | 8-byte fingerprint verification |
| `StateSnapshot::capture + diff` (discarded) | 0 † | see the disc-14 footnote above |
| `StateReceipt::begin + commit` | 1,915 | full snapshot + diff + encode cycle |
| `StateReceipt` (enriched) | 1,917 | + phase, compat_impact, validation, migration |
| `receipt + emit` | 2,231 | full cycle: begin + set + commit + emit |
| `emit_event` (32 B) | 240 | log-based event emission (`sol_log_data`) |

A complete audit trail of one state mutation — full enriched receipt plus
emission — measures **2,231 CU net, ≈ 1.1% of a 200,000 CU instruction
budget** (down from 3,141 CU in the 2026-07-07 run; the receipt encode path
got ~29% cheaper at this commit). Receipts remain a reasonable default for
audit-sensitive state changes; CU-critical one-shot programs can stay on
the load/overlay/fingerprint tier and skip receipts entirely.

### Self-CPI events (`event_cpi`)

Measured separately (2026-07-10, Mollusk 0.10.3, `examples/hopper-smoke`
instruction 4 on a **pinned** program id so the event-authority bump — and
therefore the sha256 verify-loop attempt count — is identical across runs;
the number is that instruction's total, not a net-of-harness primitive):

| Operation | CU | Notes |
| --- | ---: | --- |
| `emit_receipt` total (whole instruction) | 3,586 | bind (4 accounts incl. one PDA verify) + one counter write + self-CPI + generated sink |
| inner sink execution alone | 279 | entrypoint + dispatch + signer check + PDA address pin |
| top-level forgery refusal | 112 | marker instruction with an unsigned authority dies at the signer check |

The total moved 3,534 → 3,586 (+52) later on 2026-07-10 when the
instruction-ambient touch log landed: hopper-smoke enables the
`touch-map` feature crate-wide, so this instruction's one `get_mut`
records its write footprint (plus the per-context log reset) even
though the context never emits a map. That +52 is the measured price of
write-observability in a touch-map-enabled crate; programs that do not
enable the feature pay none of it.

Two structural notes, both disclosed wherever the feature is claimed:

- The dominant cost is the **CPI itself** (the ~1k-CU-class invoke plus the
  nested entrypoint), which every self-CPI event scheme pays — Anchor's
  `emit_cpi!` included. The log-based `emit_event` (240 CU) remains the
  cheap tier when log truncation is acceptable.
- Hopper has no compile-time program id, so the event-authority PDA is
  verified **at runtime** by the sha256 compare loop: ~148 CU per attempt
  (the 256-attempt exhaustion below ÷ 256), attempt count = 256 − bump.
  This smoke program's authority sits at the first attempt and its verify
  measures 171 CU. Anchor v0.31+ pins the authority against a compile-time
  constant for ~free — a real, stated disadvantage. `bind()` fuses
  validation and bump capture into exactly ONE derivation (measured: the
  fuse took this instruction from 3,705 to 3,534 CU). A failed bind with a
  wrong authority address exhausts the loop: ~37.9k CU on the failing
  (attacker-paid) transaction.

## Budgeting rule of thumb

From the rows above: full validated load (33) + overlay access (2) +
fingerprint re-check (6) ≈ **~40 CU of framework cost per account**. A
three-account instruction pays on the order of ~120 CU of Hopper overhead
before its business logic — well under 0.1% of a 200k budget. Opting into a
full emitted receipt adds ~2.2k CU (~1.1%).

For client-side budgeting (`SetComputeUnitLimit`), the schema field
`cu_estimate` is author-supplied and must come from a measured worst-case
run of the actual instruction, not from summing this table — per-primitive
nets do not include the entrypoint/dispatch cost of your real program, and
composition effects are not additive at single-CU resolution.

## What this lab does not measure

The following axes have **no current measured per-primitive figure**. Their
old April 2026 numbers came from a different method (validator-log deltas
on Solana 2.1) and are retired — see the appendix. Until the lab grows
discs for them, do not budget from this page:

- **Token / Mint constraint reads** (`token::mint`, `mint::decimals`, …)
- **Token-2022 extension TLV walks** (`extensions::*`)
- **PDA derivation** (`seeds`/`bump` paths). Structural guidance that
  remains true and is measured end-to-end in the vault parity lab
  (`BENCHMARKS.md`): verifying a **stored bump** with one
  `create_program_address` is roughly an order of magnitude cheaper than
  `find_program_address` bump search, and it is the path Hopper's macros
  steer you toward.
- **Logging macro variants** (`hopper_log!`, `msg!` with formatting,
  `hopper_emit_cpi!`). The measured anchor points from this run: one
  log-class syscall bills 100 CU base (the empty bracket measures 101 CU),
  and a 32-byte `sol_log_data` event measures 240 CU net (disc 9).
- **CPI tiers** (`invoke`, `invoke_signed`, `HopperDynCpi`). Measured
  end-to-end context: the router parity lab (`BENCHMARKS.md`) prices full
  router hops, each including one CPI to a mock AMM, at ~1.5k CU per hop.

Accessors that compile to a direct field read (`AccountView::address()`,
`lamports()`, `data_len()`, `raw_ref()`) carry no measured row for a
structural reason: there is nothing to bracket. They are pointer/field
reads emitted inline; any nonzero cost they had would surface in the
overlay and load rows that contain them.

## How to measure yourself

From the sibling `hopper-bench` repo (exact protocol of this page):

```bash
cargo build-sbf --manifest-path hopper-bench/Cargo.toml
cargo run --manifest-path primitive-bench/Cargo.toml --release -- \
  --out-dir results/primitive-bench-$(date +%F)
```

From this framework workspace, `hopper profile bench` runs the same
primitive lab, and `hopper profile elf` gives static size analysis, SBF
instruction-count estimates, ELF section summaries, and a flamegraph for
any compiled `.so`.

## Appendix: retired April 2026 validator-log figures

Everything below was measured (or estimated) with the **retired** method —
raw `sol_log_compute_units` deltas in `solana-test-validator 2.1`
transaction logs, without bracket-overhead subtraction — or from
first-principles syscall counting. These figures are **not comparable** to
the net column above and **must not be quoted or budgeted against**. They
are kept only so old quotes stay traceable. The per-primitive April figures
(e.g. `check_keys_eq` ~40, receipts ~50/~80/~150) are recorded row-by-row
next to their measured replacements in `BENCHMARKS.md`.

Retired account-access estimates: `try_borrow` ~2, `pod_from_bytes` ~3,
`Account::load` ~5, `load_mut` ~7, `field_segment_ref` ~4.

Retired token/mint constraint estimates: `token::mint`/`token::authority`
~8, `token::token_program` ~4, `mint::authority`/`mint::freeze_authority`
~12, `mint::decimals` ~3, `associated_token::mint` ~60.

Retired Token-2022 TLV estimates: single-extension scans ~35–50 depending
on extension and compare width.

Retired PDA estimates: inferred bump ~1,500–3,000; stored bump ~25.

Retired dispatch estimates: ~3 per single-byte match arm.

Retired logging estimates: `hopper_log!` literal ~100; label+u64 ~200;
`msg!` formatted ~300–600; `emit!` ~250; `hopper_emit_cpi!` ~1,500.

Retired CPI estimates: `invoke` ~600 + recipient; `invoke_signed` ~750 +
recipient; signer-seed threading ~150.

Retired baseline estimates: empty `sol_log_` ~100; `sol_log_64_` ~100;
`sol_invoke_signed_c` to a no-op recipient ~600.

The April-era comparative multiples that used to accompany these tables
("10x cheaper than Anchor" per constraint, etc.) were never re-measured
under the current method and are withdrawn with them; measured
cross-framework numbers live in `BENCHMARKS.md`.
