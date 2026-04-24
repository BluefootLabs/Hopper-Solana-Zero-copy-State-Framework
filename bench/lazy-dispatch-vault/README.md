# Lazy-Dispatch Vault Bench (R3)

Intra-framework benchmark that measures the CU win from Hopper's lazy
entrypoint (`hopper_lazy_entrypoint!`) against its standard eager
fast-entrypoint on dispatch-heavy programs. Closes audit recommendation
R3 from [`../../AUDIT.md`](../../AUDIT.md).

## What it measures

The `parity` vault that powers `framework-vault-bench` uses
`fast_entrypoint!` because eager parsing is what Quasar and Pinocchio
ship too — the cross-framework comparison has to be apples-to-apples.
The lazy entrypoint is a Hopper-only capability with no competitor
equivalent, so its CU win was never visible in that bench. This crate
fixes that: same 8-instruction program, built twice, once with each
entrypoint, and Mollusk runs both to surface the delta.

| Instruction | Accounts touched | Expected shape of the delta |
|---|---|---|
| `ping` (disc 0) | 0 of 8 | Largest lazy win — eager still parses all eight |
| `get_balance` (disc 1) | 1 of 8 | Large lazy win |
| `authorize` (disc 2) | 2 of 8 | Moderate lazy win |
| `counter` (disc 3) | 2 of 8 | Moderate lazy win |
| `deposit` (disc 4) | 3 of 8 | Moderate lazy win |
| `withdraw` (disc 5) | 2 of 8 | Moderate lazy win |
| `sweep` (disc 6) | 8 of 8 | Lazy ≈ eager within noise |
| `flush` (disc 7) | 8 of 8 | Lazy ≈ eager within noise |

## Building both variants

```bash
# Eager variant
cargo build-sbf -p lazy-dispatch-vault --features eager
cp target/deploy/lazy_dispatch_vault.so target/deploy/lazy_dispatch_eager.so

# Lazy variant
cargo build-sbf -p lazy-dispatch-vault --features lazy
cp target/deploy/lazy_dispatch_vault.so target/deploy/lazy_dispatch_lazy.so
```

The two `.so` files share the same cdylib name, so copy-then-rename is
the cleanest way to keep both artefacts side by side. A wrapper script
(`bench/compare-lazy-dispatch.ps1`) handles this on Windows; on Linux
or macOS `bench/compare-lazy-dispatch.sh` does the same thing.

## Running the comparison

The existing `framework-vault-bench` harness is designed for the
four-instruction parity contract and intentionally does not generalise
to eight instructions. A dedicated runner lives at
`bench/lazy-dispatch-bench/` (follow-up work, not shipped in R3's
first cut). In the meantime, a minimal Mollusk invocation looks like:

```rust
use mollusk_svm::Mollusk;

let eager = Mollusk::new(&program_id, "target/deploy/lazy_dispatch_eager");
let lazy  = Mollusk::new(&program_id, "target/deploy/lazy_dispatch_lazy");

for disc in 0u8..=7u8 {
    let ix = Instruction { program_id, accounts: eight_metas.clone(), data: vec![disc] };
    let e = eager.process_instruction(&ix, &accounts).compute_units_consumed;
    let l = lazy.process_instruction(&ix, &accounts).compute_units_consumed;
    println!("disc {}: eager {}  lazy {}  delta {}", disc, e, l, e as i64 - l as i64);
}
```

`eight_metas` is a `Vec<AccountMeta>` of 8 distinct account keys;
`accounts` is the matching `Vec<(Address, Account)>`. Most of them can
be empty `Account::new(0, 0, &SYSTEM_PROGRAM_ID)` placeholders — the
point of the bench is the *parse cost*, not what the handlers do.

## Why two features instead of two crates

A single crate with `#[cfg(feature = "eager")]` vs `#[cfg(feature =
"lazy")]` guards makes it impossible for the two variants to drift —
you cannot, for example, accidentally add a different helper to the
eager path and forget to copy it to the lazy one. The eight handler
functions (`handle_ping`, `handle_get_balance`, etc.) are shared; only
the entrypoint glue and the dispatch wiring differ.

The `compile_error!` at the top of `src/lib.rs` enforces exactly one of
the two features, so a build command missing `--features eager` or
`--features lazy` fails loudly instead of emitting an unlinkable
artefact.
