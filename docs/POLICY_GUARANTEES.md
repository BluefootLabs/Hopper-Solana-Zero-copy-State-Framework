# Policy Guarantees Matrix

Formal reference for what each `HopperProgramPolicy` lever guarantees and what it drops. Read this before flipping a lever from `STRICT` toward `RAW`.

## Named modes

| Mode | `strict` | `enforce_token_checks` | `allow_unsafe` |
|---|---|---|---|
| `HopperProgramPolicy::STRICT` | true | true | true |
| `HopperProgramPolicy::SEALED` | true | true | false |
| `HopperProgramPolicy::RAW` | false | false | true |

`STRICT` is the shipping default returned by `HopperProgramPolicy::default_policy()`.

Naming is intentionally literal:

- `STRICT` means validation and token-policy checks are enforced by default.
	It still permits explicit `unsafe` blocks because some high-performance
	programs need a reviewed escape hatch.
- `SEALED` means `STRICT` plus `allow_unsafe = false`; handler bodies cannot
	contain unsafe code unless an instruction explicitly opts into
	`unsafe_memory`.
- `RAW` means Hopper's automatic validation/token envelope is not promised.
	The author owns every signer, owner, layout, PDA, token, and aliasing check.

In short: choose `STRICT` for normal audited Hopper programs, `SEALED` when a
module must be unsafe-free by default, and `RAW` only for hand-validated expert
paths.

## What each lever controls

### `strict`

Documents that every normal handler in the module uses a typed context (`Ctx<MyAccounts>`), so `MyAccounts::bind(ctx)?` runs before the handler body. The bind call chains into the constraint check gauntlet:

1. signer
2. mut / owner / executable / address
3. duplicate-writable / signer rules
4. PDA derivation
5. init / realloc / close preconditions
6. `constraint = expr`

Flipping to `strict = false` is an intent marker: the author plans to use raw `&mut Context<'_>` handlers or other hand-validated paths and accepts responsibility for calling `validate()` where needed. Typed `Ctx<T>` handlers still bind. The handler's parameter type is the final word.

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
| `strict = false` | Framework guarantee that handlers are all typed `Ctx<T>` paths | Author must call constraint checks manually on raw `&mut Context<'_>` paths. Typed-context handlers still bind. |
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

## Growable `Seq<T>` tails under `strict_writes`

A `Seq<'a, T>` tail (see [DYNAMIC_TAILS_FROM_QUASAR.md](DYNAMIC_TAILS_FROM_QUASAR.md))
is an open-ended, growable list. Under `#[hopper::context(strict_writes)]` it is
declared with `tail(<field>)`, which compiles to a single **open-ended write
range** — `WriteRange::tail_from(idx, HEADER_LEN + <Layout>::<FIELD>_OFFSET)` =
`{ offset: TAIL_PREFIX_OFFSET, size: u32::MAX }`.

What this guarantees:

| Property | Guarantee |
|---|---|
| Tail is writable and growable | Any write at or past `TAIL_PREFIX_OFFSET` is admitted, at any account length. `push`/`set`/`swap_remove` through the gated `ctx.tail_seq_mut::<T>(idx, off)` cursor pass the policy check; growth via `realloc` needs no re-declaration (the range is already open-ended). |
| Fixed head stays byte-protected | The range starts *past* the head (`offset != 0`), so every head byte lies outside it. A write to any head field is refused at acquisition with `Custom(0xD000 \| idx)`. |
| CPI writable-meta delegation stays refused | `allows_whole_account_write` requires a range containing `[0, u32::MAX)`. An open tail range anchored past the head fails that test, so handing the account writable to a CPI callee (unbounded both-dimension delegation) is refused — the same guard the byte-range policy uses everywhere. |
| One touch record per acquire | Acquiring the cursor registers exactly ONE segment lease over the whole tail region (`[TAIL_PREFIX_OFFSET, region_len)`), not one per element, so overlap detection and the `touch-map` never overflow `MAX_TOUCH_RECORDS` on a large sequence. |

Structural rules:

- **One growable tail per account.** The `[count][elems]` framing is the whole
  tail; put other dynamic data in the fixed head or a separate account.
- **Growth cap: 10,240 B per instruction.** Growing the tail is a `realloc`, so
  it inherits Solana's `MAX_PERMITTED_DATA_INCREASE`.

The honest limit (per-element isolation):

> The declared `tail_from` range covers the **entire tail region** as one grant.
> The write policy therefore isolates the *head from the tail*, and the tail of
> one account from every other account — it does **not** isolate one tail element
> from another. Per-element / sub-range exclusion within the tail is the
> **segment registry's** job (`segment_borrow`): a `TailSeqMut` acquire takes one
> exclusive tail-region lease, so two live cursors over the same tail conflict
> (`AccountBorrowFailed`), but a single cursor may freely mutate any element. If
> you need independently borrowed sub-regions of the variable data, reach for
> named extension segments instead of a `Seq` tail.

## Related

- [policy.rs](../crates/hopper-runtime/src/policy.rs), `HopperProgramPolicy` and `HopperInstructionPolicy` definitions.
- [write_policy.rs](../crates/hopper-runtime/src/write_policy.rs), `WriteRange::tail_from` and the byte-range / lamport gate.
- [tail.rs](../crates/hopper-runtime/src/tail.rs), `Seq<T>` cursors (`TailSeq` / `TailSeqMut`) and `SeqElement`.
- [program.rs](../crates/hopper-macros-proc/src/program.rs), policy parser + handler emission.
- [UNSAFE_INVARIANTS.md](UNSAFE_INVARIANTS.md), framework-level unsafe inventory.
