# RFC: Field Behaviors (I16)

Status: **runtime core shipped** (`hopper-runtime/src/behavior.rs`);
macro attachment **proposed** (this document).

## Problem

Quasar's `AccountBehavior` is the one place its DX is genuinely ahead of
Hopper: a protocol can author a reusable, parameterized lifecycle plugin
once and attach it per context field —

```rust
#[account(fee_vault(max_bps = 30))]
vault: Vault,
```

Hopper's nearest tools don't cover this. `constraint = expr` is ad-hoc
and copy-pasted, not packageable; `#[validate]` is one method per
context, not per-field and not reusable across protocols; the lifecycle
helpers (init/realloc/close) are framework-owned, not protocol-extensible.

## Shipped runtime core

`hopper_runtime::behavior` defines the plugin contract and the phase
runners the macro will lower to (hand-wirable today, test-covered):

- `HopperBehavior<T: LayoutContract>` — unit-struct plugin over a layout
  type. Phases `check` / `update` / `exit`, gated by
  `RUN_CHECK` / `RUN_UPDATE` / `RUN_EXIT` consts so generated code emits
  only live phases (Quasar-parity codegen cost). `Args` carries the
  per-attachment parameters; `CheckOutput` is a behavior-defined payload.
- `BehaviorChecked<B, O>` — the proof token minted by `run_check`.
- `BehaviorWrite` + `const WRITES` — the field-relative byte ranges the
  behavior's mutating phases write.
- `run_check::<B, T>` / `run_update::<B, T>` — typed-path runners;
  `run_update` **takes the proof token**, making check-before-update
  structural rather than conventional.

## Where this surpasses Quasar (by design, not accident)

Quasar behaviors are opaque side-effect hooks. Hopper behaviors are
accountable to the machinery only Hopper has:

1. **Proof tokens.** `check` mints `BehaviorChecked<B, _>`; downstream
   APIs can require the token (or an `AccountProof` composed with it),
   so "this account passed the fee-cap behavior" is a type, not a hope.
2. **I12 composition.** Under `strict_writes`, the macro folds each
   attachment's `B::WRITES` (resolved to the field's account index) into
   the context's static `WritePolicy`. Plugins *extend* the declared
   write surface explicitly; they cannot silently widen it, and an
   undeclared behavior write is refused at acquisition time like any
   other.
3. **Ledger visibility.** Behavior mutations run through the standard
   typed paths, so they appear in the I7 touch map and the receipt
   system: `hopper explain` can show which plugin touched which bytes.

## Proposed macro surface

```rust
#[hopper::context(strict_writes)]
struct Collect<'info> {
    #[account(mut(collected), behavior(fee_cap, max_bps = 30))]
    vault: Vault,
    #[signer]
    authority: AccountView<'info>,
}
```

Lowering (all inside the generated `bind`):

1. Resolve `fee_cap` to a type `FeeCap: HopperBehavior<Vault>` in scope
   (convention: snake-case attr name → PascalCase type, same as Quasar's
   module convention but one item instead of three).
2. Build `FeeCapArgs { max_bps: 30 }` from the attribute payload.
3. Emit `let __vault_fee_cap = behavior::run_check::<FeeCap, Vault>(
   ctx.account(IDX)?, &ARGS)?;` after built-in validation; store the
   token on the bound context (`ctx.proofs.vault_fee_cap`).
4. If `RUN_UPDATE`: emit the `run_update` call in the pre-handler
   sequence (after check), passing the stored token.
5. If `RUN_EXIT`: emit the exit call in the epilogue, alongside the
   existing lifecycle epilogue.
6. Under `strict_writes`: extend the static `WritePolicy` with
   `WriteRange::new(IDX, w.offset, w.size)` for each `w in FeeCap::WRITES`.
7. Compile-fail checks (trybuild): behavior on a non-layout field;
   `REQUIRES_MUT` behavior on a non-`mut` field; duplicate attachment of
   the same behavior to one field.

## Open questions

- Multiple behaviors per field: ordered left-to-right; token names
  suffixed by behavior. (Quasar allows one `SETS_INIT_PARAMS` per field;
  we have no init-param phase yet — see below.)
- Init-phase hooks (`set_init_param` / `after_init` equivalents): defer
  until the behavior system meets `init` fields; the lifecycle helpers
  already own creation, so the natural seam is an `after_init` hook that
  receives the freshly-written state. Tracked, not designed here.
- Cross-field behaviors (one plugin reading two accounts): out of scope
  for v1; the `has_one`/`constraint` surface still covers pairwise
  checks.

## Migration/compat

Purely additive. No existing attribute changes meaning; contexts without
`behavior(...)` compile byte-identically.
