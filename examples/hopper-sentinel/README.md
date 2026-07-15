# SENTINEL — the byte-segment authority a compromised handler cannot escape

> **The one thing SENTINEL proves, on real compiled bytecode:**
> A privileged handler can mutate **only the byte ranges it declared**. A
> **compromised** handler that tries to write outside its declared segment is
> **refused by the framework at mutable-borrow acquisition — before any byte
> changes — with `Custom(0xD000 | account_index)`.**

This is the Drift-class rug shape: a `pause()` that has been tampered with to
**also rotate the admin key**. Hopper stops it *structurally*. Anchor, Quasar,
and Pinocchio cannot — none has a per-handler, per-byte-range write authority
that is the *same const* the runtime enforces.

---

## The flagship (two pause handlers, one context)

Both handlers share **one** `strict_writes` context, [`Pause`], that declares
`mut(paused, revision)` on the config account. The declared write set compiles
to a `&'static [WriteRange]` — the *same slice* the runtime installs at
`bind()` and checks at every Context-mediated write acquire
(`Context::check_write_policy`). **Published == enforced.**

```rust
#[derive(Accounts)]
#[accounts(strict_writes, emit_touch_map)]
pub struct Pause<'info> {
    pub admin: Signer<'info>,                         // slot 0
    #[account(mut(paused, revision), has_one = admin)]
    pub config: Account<'info, Config>,               // slot 1
}
```

**`honest_pause` (instruction 1)** — writes `paused` and `revision` through the
bound context and **succeeds**. Its Ok-path touch map records *exactly* the two
declared ranges.

**`malicious_pause` (instruction 2)** — the **same** context, the **same**
honest body, then the tampered tail rotates the admin key **through the
context**:

```rust
pub fn malicious_pause(ctx: &mut Context) -> ProgramResult {
    // 1) the honest pause runs first, through the bound context...
    {
        let mut bound = Pause::bind(ctx)?;          // installs the WritePolicy
        { let mut paused = bound.config_paused_mut()?; *paused = 1u8; }
        { let mut revision = bound.config_revision_mut()?; revision.checked_add_assign(1)?; }
    } // `bound` drops; the policy PERSISTS on `ctx`

    // 2) TAMPERED: rotate admin. `admin` is not in the declared set, so this
    //    acquisition is REFUSED at the gate — the `?` returns Custom(0xD000|1)
    //    and the assignment below never runs. No admin byte is ever written.
    let attacker = Address::new_from_array([0xAAu8; 32]);
    let mut admin = ctx.segment_mut::<Address>(1, Config::ADMIN_ABS_OFFSET)?; // ← REFUSED
    *admin = attacker;
    Ok(())
}
```

There is **no accessor for `admin`** on the bound context — a `mut(seg)` field
only generates its declared per-segment accessors (`config_paused_mut`,
`config_revision_mut`). The *only* way to reach the admin bytes is the raw
`Context`, and that path is policy-gated. `malicious_pause` uses
`ctx.segment_mut` (a **governed** path), never `AccountView::get_mut`.

---

## Proven, on two levels

### Level 1 — host (`tests/flagship_host.rs`, hopper-svm) — 8/8 pass

Drives the **real generated dispatcher** against live `AccountView` memory:

| test | proves |
|---|---|
| `flagship_honest_pause_succeeds_and_leaves_admin_untouched` | honest pause → `paused=1`, `revision=1`, admin untouched |
| `flagship_honest_pause_touch_map_records_exactly_paused_and_revision` | touch map = `[paused@114 (1B), revision@115 (8B)]` on slot 1 |
| `flagship_malicious_pause_is_refused_with_0xd001_and_admin_is_unchanged` | `Err(Custom(0xD001))` **and admin byte-unchanged** |
| `published_write_set_is_the_enforced_policy` | `WRITE_RANGES == SCHEMA_METADATA.write_ranges == installed policy.allows` |
| `columnar_record_entry_writes_the_exact_cell_and_bumps_count_and_revision` | the columnar runtime-offset pattern |
| `two_step_admin_transfer_promotes_the_pending_admin` | `accept` *declares* `mut(admin)`, so the promotion lands — the contrast |
| `collect_fees_context_demotes_the_over_declared_treasury` | the demotion payoff (below) |
| `initialize_config_runs_the_real_init_lifecycle` | the real `#[account(init)]` CreateAccount CPI |

### Level 2 — compiled SBF (`tests/refusal_sbf_e2e.rs`, Mollusk) — 2/2 pass — **THE NEW EVIDENCE**

Runs the real `hopper_sentinel.so` bytecode. **The refusal has never been
proven on compiled bytecode before.**

```
MEASURED honest_pause:    1203 CU  (SUCCEEDS; touch map = paused + revision)
MEASURED malicious_pause:  616 CU  (REFUSED with Custom(0xD001); admin unchanged)
```

- `honest_pause_succeeds_on_chain_and_emits_the_declared_touch_map` — real
  execution succeeds; the touch map is decoded from the `Program data:` log
  stream exactly as `hopper tx explain` would, and carries exactly
  `paused` + `revision`.
- `malicious_pause_is_refused_on_chain_with_0xd001_and_admin_is_unchanged` —
  the transaction **fails** with `custom program error: 0xd001` (decoded from
  the result), emits **no** touch map (Ok-path only), and the admin bytes are
  byte-for-byte unchanged after rollback.

> Run `cargo build-sbf` in this directory first, then `cargo test -p
> hopper-sentinel`. The SBF tests skip (with a notice) if the `.so` is absent.

---

## The other three proofs

**Published == enforced.** The `strict_writes` macro compiles `mut(paused,
revision)` into ONE `&'static [WriteRange]` const that backs three surfaces:
the authored `Pause::WRITE_RANGES`, the manifest
`Pause::SCHEMA_METADATA.write_ranges`, and the runtime `WritePolicy` installed
at `bind()`. The host test asserts all three are the *same slice*.

**The touch map (access, not modification).** `emit_touch_map` makes a
successful pause self-describing: the dispatcher emits one `sol_log_data`
record on the **Ok path only**, listing every `(account, offset, size, R/W)`
range the instruction touched. SENTINEL asserts the flagship's map is exactly
`paused` + `revision`, on host *and* on compiled SBF.

**The demotion payoff.** `collect_fees` uses a *mutation-complete* context
(`strict_writes` + `lamports(fee_sink)`) with a `treasury` account it touches in
**neither** dimension. `InstructionDescriptor::effective_writable` — read off
the *shipped* generated descriptor — demotes a client-over-declared `treasury`
from writable → read-only, while `config` (data range) and `fee_sink` (lamport
permission) survive.

**The columnar pattern.** `record_entry` declares `mut(entries, count)` — the
*whole* `entries` array is one range. The per-cell offset is computed at
**runtime** (`entries + i * size_of::<LedgerEntry>()`) and policy-checked
against that column, so a write to a different field is refused; per-cell
isolation comes from the segment registry; the touch map records the exact
cell. (`Ledger` is sized so the whole account — 9,620 bytes — fits a single
`init` CreateAccount CPI; the arithmetic is in the source.)

---

## Proven vs deferred

**Proven now (host + compiled SBF + LIVE DEVNET):**

- the refusal, on real bytecode, with the exact `Custom(0xD000 | account_index)`
  error and the admin bytes provably unchanged;
- published == enforced (one const, three surfaces);
- the Ok-path touch map, exact ranges, on host and SBF — and decoded from the
  live transaction with `hopper tx explain` + the from-source manifest;
- writable → read-only demotion via `effective_writable`;
- the columnar per-record write and the two-step admin transfer.

## Live on devnet (2026-07-14)

| Field | Value |
|---|---|
| Program | `CqkFhE8UVHRTJZLirEBVS1xcsZNtuNop8HniRRDWVJFC` |
| Config account | `13G2wXZ9kbX9rjcS184nGafSkddPnqq9cpjX3HkBzS5s` |
| `initialize_config` | `o3SoGa3FJtqbBWv9jV4U4E1homnShPnmSbt46Rxr78okwvbAX1BR3aS2cvk6cEFfU1yjWsAJ7YUxoKF4psy3Q7f` — 1,724 CU |
| `honest_pause` (Ok) | `yrowCoAHkd1BsTj3vRgFomExU7YBTvkr6GpqZc3JaZLC24ShYqtc3xTcz8N1yvWbHEHbe9atEokb2uABj4uxZnY` — **1,203 CU, exactly the Mollusk figure** |
| `malicious_pause` (REFUSED) | `TszXg6YGWNGfzrfbd2ekCMcN2BzjcTKxkPEmWo8dMWjcu4qHXSkJS6QSq8fhGZU77e4zk6HJBaaFVM47fEx6X9Z` — `Custom(53249)` = `0xD001`, **616 CU, exactly the Mollusk figure** |

The refusal landed on-chain via `hopper tx send --allow-failure` (skips
preflight so a transaction the program is going to refuse can reach the
cluster and become citable). Post-refusal state, fetched from devnet:
`paused = 1`, `revision = 1` (the honest writes), admin byte-for-byte equal
to `HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT` — not the `0xAA…AA`
attacker constant the tampered handler tried to write. The honest
transaction names itself: `hopper tx explain <sig> --manifest
hopper.manifest.json` matches `honest_pause (tag 1)` and decodes the live
touch map to exactly `W [114..115) → Config.paused` and
`W [115..123) → Config.revision`.

## The honest boundary

- **`strict_writes` governs the `Context` surface only.**
  `AccountView::get_mut` / `try_borrow_mut` **bypass** the policy — they are the
  documented systems-mode escape hatches. SENTINEL never uses them: every write
  routes through `ctx.config_*_mut()` or `ctx.segment_mut(...)`. The bypasses
  are grep-able (`grep -rn 'get_mut\|unsafe' src/` finds only the sanctioned
  `get_mut_after_init` init-lifecycle write and doc-comment references). A demo
  that bypassed the policy would prove nothing — so this one doesn't.
- **Touch maps record *access*, not *modification*.** A record says "this range
  was borrowed writably," which is the honest and useful thing to log; it does
  not diff bytes.
- The refusal is enforced at `Context::check_write_policy`, a single choke
  point, **not** by any hand-written `if` in the handler.

---

## Layout

```
examples/hopper-sentinel/
├── Cargo.toml                    # depends on the workspace `hopper` crate
├── README.md                     # this file
├── src/lib.rs                    # state, contexts, #[hopper::program] (9 instructions)
└── tests/
    ├── flagship_host.rs          # 8 host proofs (hopper-svm)
    └── refusal_sbf_e2e.rs        # 2 compiled-SBF proofs (Mollusk) — the new evidence
```

Zero `unsafe`. Real Hopper APIs only.
