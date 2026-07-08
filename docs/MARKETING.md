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
   **1 CU — identical to a raw unsafe pointer cast** (Mollusk, net of
   logging). The "safety tax" argument against frameworks dies here.
2. **We win our class on a real workload.** First published router-class
   three-way (multi-hop swaps, dynamic accounts, min-out gate): Hopper beats
   Quasar on **every** CU row (1,564/3,044/4,525 vs 1,582/3,064/4,546), holds
   within **2.1–2.7%** of hand-written Pinocchio, with the **smallest binary**.
   Both same-day snapshots published — including the morning we were losing.
3. **Cheapest to ship.** A complete deployable program in 4,688 bytes ≈
   **0.034 SOL** of rent; an Anchor-class artifact ≈ 1.36 SOL. 40× cheaper to
   deploy, 25× smaller.
4. **The moat is structural, verified from their source.** Every 2026
   framework (Anchor v2 alpha, Typhoon, star-frame, Quasar-adjacent) builds on
   Pinocchio's single per-account borrow byte — account-granular forever.
   Anchor v2's finest write tracking is a `MUT_MASK [u64;4]` over account
   *indices*. Hopper's ledger names **byte ranges**, which is the granularity
   Solana 2026 prices (SIMD-0339 per-info CPI costs, local fee markets on
   write locks). They cannot retrofit this without forking their substrate.
5. **The framework that audits itself — and its competitors.** 1,368 tests,
   line-by-line audit trail, Kani harnesses, and a published bug-class suite
   pinning Hopper's immunity to competitors' open soundness issues (Quasar has
   five, unfixed since June). Our own suite found and fixed a real Hopper bug
   the same day — say that out loud; honesty converts better than perfection.

## Positioning vs each competitor (factual, never sneering)

- **vs Anchor (1.x):** 4–12× CU, 26× smaller, 40× cheaper deploys — but say
  the shelf-life caveat before anyone else does: Anchor v2 alpha reaches
  Quasar-level CU. Then pivot: v2 is account-granular by construction; the
  moat features don't transfer.
- **vs Quasar:** we beat them on their own axis (CU) on both their vault and
  the router lab, we're published on crates.io with an audit trail (they're
  v0.0.0, unaudited, nightly-only, five open soundness bugs), and our
  benchmark harness is public and reproducible (theirs publishes nothing).
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
3. **"1 CU = 1 CU" micro-content**: the safe-overlay-equals-raw-cast chart.
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
| "Beats Quasar on every router row" | ✅ measured 2026-07-07, post-review run |
| "Safe overlay = raw cast (1 CU)" | ✅ measured, primitive-bench |
| "~2% from hand-written Pinocchio" | ✅ 2.1–2.7%, router lab |
| "Faster than Pinocchio" | ❌ never — one auth-fail row only, say "fast by default" |
| "40× cheaper deploys than Anchor" | ✅ vs 0.31.1 artifact; add v2 caveat |
| "Only byte-range framework" | ✅ source-verified vs Anchor v2 / Quasar / Pinocchio |
| "Audited" | ✅ internal line-by-line trail; ❌ do not imply third-party audit |
| "check_keys_eq ~40 CU" and April primitive figures | ❌ retired — superseded by Mollusk numbers |
