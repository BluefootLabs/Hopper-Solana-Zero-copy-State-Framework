# hopper-policy-vault

Three sibling programs that differ only in their `#[hopper::program(...)]` attribute. Exercises every lever on [`HopperProgramPolicy`](../../crates/hopper-runtime/src/policy.rs) and the per-instruction override flags.

## The three modes

| Module | Attribute | Policy constant | Intent |
|---|---|---|---|
| `strict_vault` | `#[hopper::program(strict)]` | `HopperProgramPolicy::STRICT` | Production default. Typed contexts, full constraint gauntlet, token-check promise, unsafe allowed but isolated. |
| `sealed_vault` | `#[hopper::program(sealed)]` | `HopperProgramPolicy::SEALED` | External-audit mode. Strict + `#[deny(unsafe_code)]` on every handler. One opt-in fast path via `#[instruction(N, unsafe_memory)]`. |
| `raw_vault` | `#[hopper::program(raw)]` | `HopperProgramPolicy::RAW` | Pinocchio-parity throughput. Every lever off. Handlers receive `&mut Context<'_>` directly. |

## Handlers

| Handler | Mode | Policy flags | What it shows |
|---|---|---|---|
| `strict_vault::deposit` | strict | inherited | Typed-context vault credit through `ctx.vault_balance_mut()` |
| `strict_vault::sweep` | strict | inherited | Typed whole-layout mutation via `ctx.vault_load_mut()` |
| `sealed_vault::deposit` | sealed | inherited + `#[deny(unsafe_code)]` | Compile-rejects any `unsafe { ... }` in this handler |
| `sealed_vault::fast_sweep` | sealed | `unsafe_memory` | Per-instruction override restores `unsafe` for exactly one handler |
| `raw_vault::deposit` | raw | inherited | Raw `&mut Context<'_>` handler, no `bind(ctx)?` auto-injection |
| `raw_vault::raw_sweep` | raw | `skip_token_checks` | Opts out of the token-check promise for this handler |
| `raw_vault::raw_pointer_reset` | raw | `unsafe_memory` | `hopper_unsafe_region!` wraps a raw pointer write at a compile-computed offset |
| `raw_vault::hybrid_bump` | raw | `unsafe_memory` | The canonical MIXED pattern: safe segment write -> `hopper_unsafe_region!` -> safe `require!` check |

## Compile-time policy assertions

Each program emits `pub const HOPPER_PROGRAM_POLICY: HopperProgramPolicy = ...;`. The example contains five `const _: () = { assert!(...); };` blocks that lock the emitted values against the named `STRICT`/`SEALED`/`RAW` constants plus per-handler policy constants (`FAST_SWEEP_POLICY`, `RAW_SWEEP_POLICY`, `RAW_POINTER_RESET_POLICY`, `HYBRID_BUMP_POLICY`). A codegen regression that shifts any lever fails the build instead of waiting for a test to execute.

## Running

```sh
cargo test -p hopper-policy-vault
```

3 runtime tests compare each program's emitted const against the shipping `HopperProgramPolicy::STRICT`/`SEALED`/`RAW` constants. The compile-time `assert!` blocks have already executed by the time `cargo test` finishes linking.

## Related docs

- [docs/POLICY_GUARANTEES.md](../../docs/POLICY_GUARANTEES.md) - what each lever guarantees and drops
- [docs/WHY_HOPPER.md](../../docs/WHY_HOPPER.md) - how the policy-driven runtime fits into Hopper's positioning
- [crates/hopper-runtime/src/policy.rs](../../crates/hopper-runtime/src/policy.rs) - `HopperProgramPolicy` / `HopperInstructionPolicy` definitions
