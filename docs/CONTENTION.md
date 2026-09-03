# Contention and declared lock footprint

`hopper contention <manifest>` reports the **write-lock and signature
footprint a program declares** — the part of a leader's price that the
declaration fixes, computed from the manifest alone.

```bash
hopper contention examples/hopper-sentinel/hopper.manifest.json
```

```text
Instruction                 W  W-eff  Sigs Fixed max   Saved  Proven RO   Rem
--------------------------------------------------------------------------------
initialize_config           2      2     1     1320       -          -     -
honest_pause                1      1     1     1020       -          -     -
unpause                     1      1     1     1020       -          -     -
initialize_ledger           2      2     1     1320       -          -     -
begin_admin_transfer        1      1     1     1020       -          -     -
accept_admin_transfer       1      1     1     1020       -          -     -
collect_fees                1      1     1     1020       -          1     -
--------------------------------------------------------------------------------
```

Everything in that table is derived from the declaration — the account
list plus the same `writeRanges` / `lamportAccounts` the runtime enforces
and the manifest publishes. Role counts are exact, offline, and
reproducible. `Fixed max` is a deterministic upper bound: optional account
roles may be absent and `dup` roles may resolve to one Pubkey, while Agave
charges each unique present key once.

## Read `Fixed max` for what it is

**`Fixed max` is not a transaction's block cost.** Agave's
`calculate_transaction_cost` sums five terms:

```text
signature_cost + write_lock_cost + data_bytes_cost
  + programs_execution_cost + loaded_accounts_data_size_cost
```

Hopper computes the first two, because those are the ones a declaration
fixes. The other three are caller choices that no manifest analysis can
supply:

- `programs_execution_cost` is the transaction's **requested compute
  limit** — not its burn. It is usually the largest term by an order of
  magnitude: a transaction that sets no `ComputeBudget` instruction is
  charged 200,000 CU for a single instruction. A client lowers it with
  `SetComputeUnitLimit`.
- `loaded_accounts_data_size_cost` is the **requested** loaded-data limit,
  8 CU per 32 KiB page, defaulting to the 64 MiB ceiling — 16,384 CU.
  Lowered with `SetLoadedAccountsDataSizeLimit`.
- `data_bytes_cost` is `instruction_data_len / 4`.

Two more scope limits, both real:

- The **fee payer** is a writable signing account of every message. When
  it is not already among an instruction's declared writable accounts, a
  real transaction pays one more write lock than this reports.
- Locks and signatures are charged **once per transaction** over
  deduplicated keys. The per-instruction rows do not add up to a
  transaction's cost, and the tool says so in its own summary.

Use `Fixed max` to compare declarations and to gate declaration drift — not
to predict a fee or a block share.

## The constants

Each carries its upstream source, so a repricing SIMD is a one-line audit
here rather than archaeology. Verified against `anza-xyz/agave` master:

| Quantity | Value | Agave source |
| --- | --- | --- |
| Per writable account lock | **300 CU** | `cost-model/src/block_cost_limits.rs` (`WRITE_LOCK_UNITS`) |
| Per signature | **720 CU** | same (`SIGNATURE_COST`) |
| Loaded account data | **8 CU per 32 KiB page** | `program-runtime/src/execution_budget.rs` (`DEFAULT_HEAP_COST`) |
| Instruction data | **len / 4** | `cost_model.rs` (`INSTRUCTION_DATA_BYTES_COST`) |
| Default requested CU | **200,000** | `execution_budget.rs` (`DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT`) |
| Block limit (observed 2026-09-03) | **75,000,000 CU** | 300 ms regime × SIMD-0286 gate; see below |
| Per-account cap (observed 2026-09-03) | **30,000,000 CU** | same derivation |

**These two ceilings are not constants.**
[SIMD-0525](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0525-reduce-slot-times.md)
steps mainnet slot time down, and
[Agave v4.2.2](https://github.com/anza-xyz/agave/blob/v4.2.2/runtime/src/slot_params.rs)
rescales both ceilings with it so CU-per-second stays fixed at 250M. The static figures in
`block_cost_limits.rs` (24M account / 60M block) are the **400 ms baseline**;
the
[SIMD-0286](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0286-raise-block-limits-to-100M.md)
gate then scales both by 100/60:

| Regime | base account / block | with the 100M gate |
| --- | --- | --- |
| 400 ms | 24M / 60M | 40M / 100M |
| 350 ms (mainnet 2026-08-21) | 21M / 52.5M | 35M / 87.5M |
| **300 ms (mainnet 2026-08-28, current)** | 18M / 45M | **30M / 75M** |
| 250 ms | 15M / 37.5M | 25M / 62.5M |
| 200 ms | 12M / 30M | 20M / 50M |

So "100M blocks" is the 400 ms figure and was superseded a week after it
activated. Hopper derives both ceilings from `SlotTimeRegime` and records the
observed mainnet regime with a date (`MAINNET_OBSERVED_REGIME` /
`MAINNET_OBSERVED_ON`) rather than hardcoding a number that silently rots.

Two consequences worth internalizing:

- **Write locks are counted, not priced by contention.** There is no
  per-account base fee: SIMD-0110 (per-account fee markets) is not
  activated (SIMD-0110 was CLOSED unmerged, 2025-01-14). What exists is the
  flat 300 CU per writable account plus the per-account cap (30M today), and
  the scheduler's per-`Pubkey` serialization.
  "Local fee markets" today are an emergent effect of that cap and
  priority fees, not a price on the account.
- **The scheduler serializes on whole accounts.** Agave's greedy
  scheduler (default since v2.3) keys its locks on
  `AHashMap<Pubkey, AccountLocks>`. Byte-range disjointness — Hopper's
  specialty — buys correctness and auditability, but the protocol does not
  currently reward it with parallelism. Anyone claiming otherwise is
  describing a SIMD that has not merged.

## `Proven RO`: the column a real write set makes possible

Under a **mutation-complete** context (`strict_writes` + `lamports(...)`),
both mutation dimensions are declared and enforced across Hopper's supported
governed APIs. For a non-signer account with no declared byte range and no
lamport permission, `Proven RO` means that those APIs cannot mutate it.
Marking such an account writable buys nothing on that governed surface: it
costs a flat 300 CU write lock and makes every transaction touching that
account serialize against yours.

In the sentinel, `collect_fees` declares `mut(revision)` on `config` and
`lamports(fee_sink)`. `treasury` appears in neither dimension, so it is
read-only under the installed Hopper policy and the tool reports it. This is
stronger than an advisory write set, but it is not whole-program proof against
arbitrary Rust, FFI, dependency, direct-substrate, or unchecked-CPI paths.

Note what the sentinel row does **not** show: a `Saved` figure. Demotion
only produces a saving when a manifest declares an account writable that
the write set clears — and a correct Hopper manifest already marks
`treasury` read-only, so there is nothing to demote and the honest saving
is 0. `Proven RO` is the actionable number, and it is aimed at client
authors: hand-rolled clients and ports from frameworks where "when in
doubt, mark it writable" is the safe default.

### Why the proof holds

"Never mutated" is a strong claim, so here is the whole chain. For a
non-signer account with no declared byte range and no lamport permission,
under a mutation-complete context:

- **Supported data writes** are refused by the ambient write gate. A bound
  strict context installs it, and it governs the `Context` surface plus the
  runtime `AccountView` paths documented as policy-aware (`segment_mut`,
  `resize`, `close`, and the extension region). Direct calls into the raw
  Hopper Native backend are outside this boundary.
- **Lamport moves** are refused by the same gate's lamport dimension,
  which a mutation-complete context declares.
- **Writable CPI hand-offs** are refused by `check_lamport_delegation`,
  which requires *both* a whole-account data grant and lamport permission
  before an account may be passed writable to a callee. An account with
  neither cannot be delegated, so a callee cannot mutate it either.
- **Lifecycle cranks** cannot reach it: every attribute whose bind-time
  work rewrites an account (`init`, `init_if_needed`, `realloc`, `close`,
  `migrate(...)`, `epoch_migrate`) contributes a declared range, which
  disqualifies the account from `Proven RO` in the first place. That
  coupling is load-bearing and pinned by
  `epoch_migrate_declares_the_range_its_bind_crank_rewrites` and
  `migrate_without_mut_is_rejected` in the context macro's tests.

The residual surface includes the documented `*_unchecked` tier, direct
Hopper Native access, arbitrary unsafe or FFI code, dependencies, and other
ways to avoid the supported runtime surface. `hopper lint --deny-escapes`
rejects known ledger-bypassing accessor spellings in scanned project source;
it is a textual review aid, not semantic whole-program analysis. Treat
`Proven RO` as a governed-surface result and require explicit review before
using it to demote hand-written client metas.

Signers are excluded from `Proven RO` even when the write set clears them.
Writability is a transaction-level flag and the fee payer is a signer that
must stay writable to be debited, whatever a given instruction declares. A
costing hint that told you to send the fee payer read-only would produce
broken transactions.

The claim also requires the ranges to be **enforced**, not merely
declared: a manifest is JSON, and `mutationComplete` without
`strictWrites` proves nothing. The tool refuses to demote or to claim
`Proven RO` in that case, matching what generated clients do.

## `Rem`: what a declaration cannot bound

An instruction that accepts `remaining_accounts` takes caller-supplied
suffix accounts whose writable flags the client chooses. Each writable one
is a real 300 CU lock the declaration cannot constrain, so the ceiling is
reported in its own column and deliberately left out of `Fixed max` — the
gated figure stays deterministic without pretending caller-selected keys
are known.

## Using it as a CI gate

```bash
hopper contention hopper.manifest.json --max-block-cost 2000
```

Exits 1 if any instruction's `Fixed max` exceeds the ceiling. It gates a
fixed-role upper bound, so it is deterministic: it cannot
flake, and it moves only when someone changes the account set or the write
declaration.

It fails closed. A manifest that parses to zero instructions is refused
rather than reported under budget (far more often the wrong file than a
real program), two positional manifests are an error instead of
last-one-wins, a repeated `--max-block-cost` is refused rather than
silently taking the last value, and a path that does not exist says so
instead of surfacing a JSON parser's confusion.

## What this is not

It is **not** a compute-unit budget. The CU a handler burns cannot be
derived from a declaration, and Hopper does not fabricate it: `cuEstimate`
stays `0` in the manifest until something measures it, and every emitter
treats `0` as absent. For measured compute, use the benchmark lane
(`hopper profile bench`, which carries per-case `budget_cu` baselines with
a tolerance and a regression flag) rather than this command.
