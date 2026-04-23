# Compute-unit cost reference

Every operation Hopper emits, grouped by axis, with a measured or structural cost. Numbers come from `hopper profile bench` runs against a local validator with a Solana 2.1.x SBF toolchain, or from first-principles counting of syscalls where a benchmark is not meaningful (there is no way to benchmark a construct that compiles out).

The point of this page is that every CU claim elsewhere on the site has a line here it refers back to. If a number in the marketing drifts from this table, the table wins.

## Baselines

Three references to calibrate everything below.

| Reference | CU |
| --- | --- |
| Empty `sol_log_(0)` | ~100 |
| `sol_log_64_(...)` (five `u64`) | ~100 |
| `sol_invoke_signed_c` syscall, no-op recipient | ~600 |

Solana runtime charges these regardless of framework. Every number below is additive on top.

## Account access

| Operation | CU | Notes |
| --- | --- | --- |
| `AccountView::address()` | 0 | pointer read into the SVM input region |
| `AccountView::lamports()` | 0 | direct field access |
| `AccountView::data_len()` | 0 | direct field access |
| `AccountView::try_borrow()` | ~2 | borrow-flag check plus slice construction |
| `pod_from_bytes::<T>(data)` | ~3 | length check + pointer cast |
| `Account::load::<T>()` | ~5 | owner + disc + length + pointer cast |
| `Account::load_mut::<T>()` | ~7 | load plus writable + borrow-registry update |
| `ctx.field_segment_ref::<T>(i, o)` | ~4 | const offset + length check |
| `raw_ref()` / `raw_mut()` (unsafe) | 0 | identity pointer cast |

Anchor's zero-copy path via `AccountLoader<T>` measures ~12 CU for the equivalent of `load`, because the RefCell bookkeeping is not compile-time folded. Hopper's segment-level registry is compile-time folded, so the mut flag update resolves to a field write with no branch.

## Token / Mint reads

Every constraint that ends in a `require_*` helper reads the exact bytes it needs and nothing more.

| Constraint | CU | Notes |
| --- | --- | --- |
| `token::mint = expr` | ~8 | 32-byte compare on bytes `[0..32]` |
| `token::authority = expr` | ~8 | 32-byte compare on bytes `[32..64]` |
| `token::token_program = expr` | ~4 | owner pubkey compare |
| `mint::authority = expr` | ~12 | COption tag check plus 32-byte compare |
| `mint::decimals = N` | ~3 | single-byte compare at offset 44 |
| `mint::freeze_authority = expr` | ~12 | COption tag check plus 32-byte compare |
| `associated_token::mint = expr` | ~60 | full ATA PDA derivation (one `create_program_address`) plus compare |

Anchor routes every token constraint through `InterfaceAccount<TokenAccount>` which deserializes the full 165-byte account via Borsh before any check. Measured ~80 CU for the same `token::mint` check. Hopper is 10x cheaper on this path.

## Token-2022 extension TLV

Each extension constraint is one TLV walk from the start of the extension region.

| Constraint | CU | Notes |
| --- | --- | --- |
| `extensions::non_transferable` | ~35 | scan mint TLV, find type byte 9 |
| `extensions::mint_close_authority` | ~45 | scan + 32-byte compare |
| `extensions::transfer_hook::authority` | ~45 | scan + 32-byte compare on offset 0 of payload |
| `extensions::transfer_hook::program_id` | ~45 | scan + 32-byte compare on offset 32 of payload |
| `extensions::metadata_pointer::*` | ~45 | same shape as transfer_hook |
| `extensions::permanent_delegate` | ~45 | scan + 32-byte compare |
| `extensions::transfer_fee_config::*` | ~50 | scan + one 32-byte compare |
| `extensions::interest_bearing::rate_authority` | ~45 | scan + 32-byte compare |
| `extensions::default_account_state::state` | ~35 | scan + single-byte compare |
| `extensions::immutable_owner` | ~35 | scan token-account TLV, find type byte 7 |

Anchor's `InterfaceAccount<Mint>` path plus Borsh deserialize hits ~400-600 CU for the same check, depending on extension count. Hopper is 10x to 17x cheaper across the board. Every number is a single TLV scan plus a compare; there is no allocator call anywhere.

## PDAs

| Operation | CU | Notes |
| --- | --- | --- |
| `seeds = [...]` + `bump` (inferred) | ~1500 to ~3000 | `find_program_address` walks bumps 255 down |
| `seeds = [...]` + `bump = stored_field` | ~25 | `create_program_address` one iteration |
| `seeds::program = expr` override | +0 | identical cost with a different program id |
| `seeds_fn = Type::seeds(...)` | same as above | the sugar is a caller-side indirection |

Store the bump in the account whenever you can. The 60x speedup is the single biggest CU win available to program authors.

## Instruction dispatch

| Shape | CU | Notes |
| --- | --- | --- |
| Single-byte `match data[0]` | ~3 | jump table, compiler-folded |
| Multi-byte `data.starts_with(&[...])` chain | ~3 per arm | ordered longest-first |
| `ctx_args = K` forward to `bind_with_args` | +0 | inlined |

Quasar's dispatch is identical in shape. Pinocchio's is hand-written per program; the same cost when written correctly.

## Logging

| Call | CU | Notes |
| --- | --- | --- |
| `hopper_log!("literal")` | ~100 | direct `sol_log_` syscall |
| `hopper_log!("label", u64_value)` | ~200 | one `sol_log_` plus one `sol_log_64_` |
| `msg!("text")` | ~100 | same syscall, no format |
| `msg!("fmt {}", x)` | ~300 to ~600 | depends on format string length and arg count |
| `emit!(Event { ... })` | ~250 | `sol_log_data` with one segment |
| `hopper_emit_cpi!(...)` | ~1500 | self-CPI path, reliable indexing |

The log vs emit_cpi tradeoff is the same in Anchor. Use `emit!` for cheap telemetry, `hopper_emit_cpi!` for the events indexers must not drop.

## Receipts

| Operation | CU | Notes |
| --- | --- | --- |
| Receipt scope begin | ~40 | snapshot capture on mutable segments |
| Receipt finish (success) | ~250 | hash + write + `sol_log_data` |
| Receipt finish (invariant failure) | ~300 | plus the failure-stage stamp |

Anchor has no equivalent today. The ~300 CU buys you "Invariant balance_nonzero failed" attribution in every indexer without a per-program lookup table.

## CPI

| Op | CU | Notes |
| --- | --- | --- |
| `invoke(&ix, &accounts)` | ~600 + recipient | native-backend direct syscall |
| `invoke_signed(&ix, &accounts, &signers)` | ~750 + recipient | syscall plus seed setup |
| `HopperDynCpi::invoke_signed` | ~750 + recipient | same cost; stack-only build |
| Anchor `CpiContext::new_with_signer` | ~850 + recipient | extra bookkeeping around the same syscall |

Signer seeds threading costs ~150 CU no matter who sets it up.

## How to measure yourself

Use the CLI shipped in this release:

```
hopper profile bench --fail-on-regression 2
```

against any program that links `hopper-bench`, or point `hopper profile elf` at a compiled `.so` for static size analysis plus a flamegraph.

## Why these numbers hold up

Two reasons. First, Hopper compiles handler code into straight-line accessors with const-folded offsets; there is no runtime dispatch to pay. Second, the Token-2022 TLV readers walk the extension region directly instead of deserializing the parent account; the scan is shorter than the Borsh alternative by construction, not by micro-optimization.

Nothing on this page is a benchmark cherry-pick. If a number in Hopper's emitted code shifts, this file moves with it.
