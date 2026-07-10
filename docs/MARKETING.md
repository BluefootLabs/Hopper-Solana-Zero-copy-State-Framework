# Hopper Marketing Playbook — 2026-07

Every claim in this playbook is measured or source-verified; provenance lives
in `BENCHMARKS.md`, `docs/THE_MOAT.md`, and the `hopper-bench` results dirs.
The house rule carries over from engineering: **we market what we measure.**
That discipline *is* the brand — every competitor claims speed; only Hopper
publishes reproducible, provenance-pinned tables including the rows it loses.

## The one-liner

> **Write like Anchor. Run like Pinocchio. Prove what no one else can.**

Long form: Hopper is the only Solana framework with its own substrate and a
byte-range borrow ledger — which is why it's the only one that can hand you
receipts, touch maps, field-level write policies, and write-containment tests,
while beating the other zero-copy frameworks on the benchmarks they compete on.

## The five proof points (lead with these, in this order)

1. **Safety is free, measured.** Hopper's safe validated overlay costs
   **exactly what a raw unsafe pointer cast costs — 2 CU net, the same
   number** (Mollusk, 2026-07-09, net of logging). The equality is the
   claim; quote both sides together. The "safety tax" argument against
   frameworks dies here. Bonus from the same run: the receipt engine got
   ~31% cheaper this week (full receipt+emit = 1.1% of a 200k budget).
2. **We win our class on a real workload.** First published router-class
   three-way (multi-hop swaps, dynamic accounts, min-out gate): Hopper beats
   Quasar on **every** CU row (2026-07-09: 1,559/3,035/4,512 vs
   1,582/3,064/4,546), within **1.8–2.4%** of hand-written Pinocchio, with
   the **smallest binary** (10.74 KiB). And the meta-story is even better:
   a same-day re-run caught a regression that flipped these rows, we
   SUSPENDED the claim in public, bisected every CU, fixed it, and
   re-earned the win a few hours later measurably better than before.
   Nobody else's benchmark culture can even express that sentence.
3. **Cheapest to ship.** A complete deployable program in 3,736 bytes ≈
   **0.027 SOL** of rent (2026-07-09 build; the 2026-07-07 devnet artifact
   was 4,688 bytes ≈ 0.034 SOL); an Anchor-class artifact ≈ 1.36 SOL. ~50×
   cheaper to deploy on the counter, and the vault artifact is 26× smaller
   (7.46 vs 190.11 KiB).
4. **The moat is structural, verified from their source.** Every 2026
   framework (Anchor v2 alpha, Typhoon, star-frame, Quasar-adjacent) builds on
   Pinocchio's single per-account borrow byte — account-granular forever.
   Anchor v2's finest write tracking is a `MUT_MASK [u64;4]` over account
   *indices*. Hopper's ledger names **byte ranges**, which is the granularity
   Solana 2026 prices (SIMD-0339 per-info CPI costs, local fee markets on
   write locks). They cannot retrofit this without forking their substrate.
5. **The framework that audits itself — and its competitors.** 1,570 tests
   (2026-07-09), line-by-line audit trail, Kani harnesses, and a published
   bug-class suite (18 pinned tests) pinning Hopper's immunity to
   competitors' open soundness issues (Quasar has
   five, unfixed since June). Our own suite found and fixed a real Hopper bug
   the same day — say that out loud; honesty converts better than perfection.

## Positioning vs each competitor (factual, never sneering)

- **vs Anchor (1.x):** 4–12× CU (2026-07-09 vault: authorize 11.9×, deposit
  4.3×, withdraw 10.5×), 26× smaller, ~50× cheaper deploys — but say
  the shelf-life caveat before anyone else does: Anchor v2 alpha reaches
  Quasar-level CU. Then pivot: v2 is account-granular by construction; the
  moat features don't transfer.
- **vs Quasar:** we beat them on their own axis (CU) on their vault
  (2026-07-09: 1,653 vs 1,756 deposit, 486 vs 592 withdraw) AND the router
  lab (1,559/3,035/4,512 vs 1,582/3,064/4,546, re-earned same day). We're
  published on crates.io with an audit trail (they're v0.0.0, unaudited,
  nightly-only, five open soundness bugs), and our benchmark harness is
  public and reproducible (theirs publishes nothing).
- **vs Pinocchio:** never claim to beat hand-written Pinocchio; claim ~2%
  away *with* the full contract layer, and that Anza's own audited substrate
  ideas are cross-checked into ours. Pinocchio is the floor we respect, not a
  rival to disparage. Programs that outgrow raw Pinocchio graduate to Hopper.

## The 2026 story (where the puck is going)

Solana is repricing exactly what Hopper already does: SIMD-0339 charges per
account-info (we dedup), local fee markets price write locks (we publish
byte-range write-sets — "scheduler-legible programs"), SIMD-553 bills unused
CU (measured-CU manifests next), sBPF v3 static syscalls (we're ready,
default-off). Tagline for this thread: **"Built for the Solana that's
coming, measured on the Solana that's here."**

## Channels & sequencing

1. **The router benchmark post** (the wedge): "We published the first
   three-way router benchmark. Here's the harness, the contract, and the day
   we went from losing to winning." X thread + long-form. Invite Quasar and
   Anchor to submit rows — openness is the flex.
2. **`THE_MOAT.md` as a standalone essay**: why byte-range vs account-granular
   is the 2026 dividing line, with the `MUT_MASK` receipt.
3. **"Safe = raw, same number" micro-content**: the safe-overlay-equals-raw-cast chart (2 = 2 CU net, 2026-07-09).
4. **Solana Foundation surface area**: verified builds, formal-verification
   flywheel (Kani/Certora), and the write-set SIMD draft with Hopper as
   reference implementation — the standards play that makes Hopper the
   incumbent before the standard exists.
5. **Migration funnels**: `PORT_QUASAR_IN_20_MINUTES.md` and
   `MIGRATION_FROM_ANCHOR.md` timed to Anchor v2's disruption window (v1→v2
   is a rewrite moment — devs re-evaluating anyway are the cheapest converts).
6. **"Zero Node dependencies" (developer-ergonomics wedge):** `hopper
   publish-idl` publishes your Anchor-compatible IDL to the SPL Program
   Metadata PDA in pure Rust — no `npx`, no `node_modules`, no
   `@solana-program/program-metadata` shell-out (which is exactly what
   Anchor's own `anchor idl` does). Explorer interop for free, one binary,
   no JS toolchain. Small, concrete, and it needles the pain every Rust
   Solana dev already feels.

## Claims register (what we may and may not say)

| Claim | Status |
|---|---|
| "Beats Quasar on every router row" | ✅ **re-earned 2026-07-09** — 1,559/3,035/4,512 vs Quasar's 1,582/3,064/4,546 (margins 23/29/34), BETTER than the 07-07 win (1,564/3,044/4,525). The claim was suspended for ~6 hours the same day when a re-run caught a Batch-6 regression (+52 CU/hop: gate-machinery calls reachable from the per-meta CPI closure forced spill-heavy codegen); a full bisect attributed every CU, and the fix (once-per-CPI delegation sweep behind a liveness branch) landed measured. Tell that story — the suspension IS the brand. Smallest binary holds (10.74 vs P 10.98 / Q 11.05 KiB) |
| "Safe overlay = raw cast" | ✅ measured EQUAL, 2 CU net each (2026-07-09 primitive lab; the 07-07 run measured both at 1 — the equality is the durable claim, never quote the absolute alone) |
| "~2% from hand-written Pinocchio" | ✅ 1.8–2.4% (2026-07-09 router lab; was 2.1–2.7%) |
| "Faster than Pinocchio" | ❌ never — one auth-fail row only, say "fast by default" |
| "~50× cheaper deploys than Anchor" | ✅ vs 0.31.1 artifact (counter 3,736 B ≈ 0.027 SOL, 2026-07-09 build, vs 1.356 SOL); the retired "40×" figure was the 4,688-B devnet artifact; add v2 caveat |
| "Only byte-range framework" | ✅ source-verified vs Anchor v2 / Quasar / Pinocchio |
| "Audited" | ✅ internal line-by-line trail; ❌ do not imply third-party audit |
| "check_keys_eq ~40 CU" and April primitive figures | ❌ retired — superseded by Mollusk numbers |
| "Smallest binary" | ⚠️ **router-scoped only.** Quasar still wins the vault (5.47 vs 7.46 KiB, re-measured 2026-07-09, `BENCHMARKS.md`). New claimable fact from that run: the Hopper vault `.so` is **smaller than Pinocchio's** on the identical contract (7.46 vs 7.73 KiB, zero writable sections). Never claim a general size lead; the remaining Quasar delta is paid-for structure (receipts, ledger, lamport gate) |
| "Kani-proves the SHIPPED SPL/System CPI encoders" | ✅ the harnesses now call the shipped `hopper_runtime::{token,system}::encoders::*` directly (22 wire-format + 22 differential-oracle vs an independent reference as of 2026-07-09, up from 17+17), byte-identical to what the CPI emits. ⚠️ scope: the **fixed-size** instruction set. The System `*WithSeed` family splices a runtime seed slice and is NOT proven — never imply it is |
| "Smallest binary" (again) | ✅ resolved 2026-07-09: the Batch-5–7 `.text` regression (14.03 KiB) was clawed back — the re-run measures the vault at `.so` 7,640 B (7.46 KiB), `.text` 6,016 B, zero writable sections after the SBF-loader P0 fix. The size row is quotable again, with the row scoping above |
| "Native `publish-idl`, no Node" | ✅ real signed on-chain send (fresh inline / Allocate + chunked Write + Initialize / `--overwrite` SetData). No JS toolchain. ⚠️ not yet devnet-battle-tested |
| "The CLI publishes the program's byte-range write-set" | ✅ the manifest loader now carries `writeRanges` / `strictWrites` / `mutationComplete` / `lamportAccounts` / `cuEstimate` end-to-end |
| "Anchor constraint-DSL parity" | ✅ incl. `@ CustomError` binding, the `realloc::payer` / `realloc::zero` spelling, and — as of 2026-07-09 — optional accounts (`Option<Account<T>>` / `Option<Signer>` / every role wrapper + `AccountView`, Anchor's program-id-in-slot absence convention, absent = one address compare + zero checks, present = the full required-form checks; proven by expansion tests + a hopper-svm both-arms integration suite). ⚠️ scope: an optional field cannot be an `init`/`init_if_needed`/`zero`/`close`/`realloc`/`sweep` lifecycle target or carry `mut(seg)` lists (compile errors, like Anchor's own combo restrictions). ❌ still missing composite/nested contexts — call that catch-up, never "innovation" |
| "Rent-exempt math is reprice-safe" | ✅ `Rent::minimum_balance` reads the live sysvar and byte-matches Solana. ⚠️ the `const` fast path is explicitly NOT reaping-safe — say so |
| "`mutation_complete` means complete" | ⚠️ **scoped**: substrate escape hatches (`substrate::batch::transfer_lamports`, the `unsafe` unchecked CPI tier) sit outside the gate. Sealevel's `writable` bit still enforces underneath, so the failure mode is a failed tx, never fund loss |
| "Every Hopper tx explains its own state effects" | ✅ demonstrable end-to-end as of 2026-07-09: `examples/hopper-smoke`'s `Withdraw` context opts in via `#[accounts(strict_writes, emit_touch_map)]`, and its e2e test (`examples/hopper-smoke/tests/touch_map_e2e.rs`) executes the compiled `.so` in an in-process SVM, captures the log stream, and decodes exactly one touch-map record (`W` vault `[48..56)` = the `balance` write) through the same wire-format check `hopper tx explain` applies — with zero records when the same handler errs after touching state (Ok-only emission, host-verified). ⚠️ not yet claimable as "**every** Hopper tx" — the emission is per-context opt-in by design (each record costs a `sol_log_data`, CU honesty), whole-account wrapper borrows (`Account::get_mut`) are NOT captured (only Context-mediated segment/`load_mut` paths are), and no live devnet transaction has been decoded yet. Say "Hopper transactions can explain their own state effects" |
| "Single-pass checked CPI" | ✅ fused validate+build, adversarially reviewed clean. ⚠️ the CU delta is **estimated** (one array traversal's control-flow removed); never fabricate a number |
