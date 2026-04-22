# Policy Guarantees Matrix

Formal reference for what each `HopperProgramPolicy` lever guarantees and what it drops. Read this before flipping a lever from `STRICT` toward `RAW`.

## Named modes

| Mode | `strict` | `enforce_token_checks` | `allow_unsafe` |
|---|---|---|---|
| `HopperProgramPolicy::STRICT` | true | true | true |
| `HopperProgramPolicy::SEALED` | true | true | false |
| `HopperProgramPolicy::RAW` | false | false | true |

`STRICT` is the shipping default returned by `HopperProgramPolicy::default_policy()`.

## What each lever controls

### `strict`

Documents that every handler in the module uses a typed context (`Context<MyAccounts>`), so `MyAccounts::bind(ctx)?` runs before the handler body. The bind call chains into the constraint check gauntlet:

1. signer
2. mut / owner / executable / address
3. duplicate-writable / signer rules
4. PDA derivation
5. init / realloc / close preconditions
6. `constraint = expr`

Flipping to `strict = false` is an intent marker: the author plans to use `&mut Context<'_>` handlers and accepts responsibility for calling `validate()` where needed. The macro does not mechanically skip bind based on this flag. The handler's parameter type is the final word.

### `enforce_token_checks`

Promise that every SPL token CPI in the module uses `*_strict` or `*_signed_strict` invoke variants. Those helpers pre-verify:

| Check | Helper | Where |
|---|---|---|
| Authority is a transaction signer | `require_authority_signed_direct` | `crates/hopper-runtime/src/token.rs` |
| Token account's `owner` field matches authority | `require_token_authority` | same file |

The SPL Token program itself re-validates both checks. Hopper's pre-check surfaces a Hopper-branded `ProgramError::IncorrectAuthority` or `MissingRequiredSignature` before the CPI so a misrouted signer or mismatched owner fails with a specific error instead of an opaque SPL failure. This closes the exploit class "attacker passes correct pubkey but wrong signer".

Flipping to `enforce_token_checks = false` drops the pre-check promise. The SPL program's checks still run. Only reach for this when the program has its own validation flow that makes the pre-check redundant.

### `allow_unsafe`

When true (default), handler bodies can contain `unsafe { ... }` blocks and the `hopper_unsafe_region!` macro.

When false, the program macro emits `#[deny(unsafe_code)]` on every handler that does not carry `#[instruction(N, unsafe_memory)]`. Any stray `unsafe { ... }` fails to compile. The per-instruction override restores unsafe for a single handler without affecting the rest of the module.

## What each policy drops

| Policy | Dropped invariant | What this means |
|---|---|---|
| `strict = false` | Auto-injected `bind(ctx)?` when handlers use raw `&mut Context<'_>` | Author must call constraint checks manually. Typed-context handlers still bind. |
| `enforce_token_checks = false` | Hopper-branded pre-check on token CPIs | Only the SPL program's checks run. Any Hopper-side ownership mismatch surfaces as a generic CPI failure. |
| `allow_unsafe = false` | Raw pointer access in handler bodies | `unsafe { ... }` and `hopper_unsafe_region!` fail to compile unless the handler opts in via `#[instruction(N, unsafe_memory)]`. |
| `#[instruction(N, unsafe_memory)]` | Program-level `#[deny(unsafe_code)]` for this handler only | Raw pointer access restored for this one handler. Other handlers stay sealed. |
| `#[instruction(N, skip_token_checks)]` | Program-level token-check promise for this handler | Author documents why the checks are upheld elsewhere (or not needed). |

## Zero-cost property

Every lever is a compile-time `bool` on a `Copy + const` struct. Readers call `HOPPER_PROGRAM_POLICY.<lever>` in `const` context; the branches fold to a single code path during codegen when the lever is known. There is no runtime state, no thread-local, no syscall. A program compiled with `HopperProgramPolicy::RAW` pays zero CU for Hopper's safety envelope.

## Grep receipts

An auditor lands in the tree and wants a one-command inventory of every raw-pointer region:

```sh
grep -rn "hopper_unsafe_region!" crates/ examples/
```

Every Hopper-authored unsafe segment surfaces. The macro expands to `unsafe { ... }`, so the actual codegen is unchanged; the name is the indexing hook.

For the stricter "every unsafe region in the tree, Hopper or otherwise":

```sh
grep -rn "unsafe " crates/ examples/ tools/
```

Hopper's internals use `unsafe` for the zero-copy core (pointer casts, syscall wrappers, Pod overlays). Those regions are documented in [UNSAFE_INVARIANTS.md](UNSAFE_INVARIANTS.md).

## Worked examples

- `examples/hopper-policy-vault/src/lib.rs::strict_vault`, `HopperProgramPolicy::STRICT` for a conventional vault.
- `examples/hopper-policy-vault/src/lib.rs::sealed_vault::fast_sweep`, `SEALED` program with one handler opting into `unsafe_memory`.
- `examples/hopper-policy-vault/src/lib.rs::raw_vault::hybrid_bump`, `RAW` program demonstrating the safe -> unsafe -> safe mixed pattern inside one handler.

## Related

- [policy.rs](../crates/hopper-runtime/src/policy.rs), `HopperProgramPolicy` and `HopperInstructionPolicy` definitions.
- [program.rs](../crates/hopper-macros-proc/src/program.rs), policy parser + handler emission.
- [UNSAFE_INVARIANTS.md](UNSAFE_INVARIANTS.md), framework-level unsafe inventory.
