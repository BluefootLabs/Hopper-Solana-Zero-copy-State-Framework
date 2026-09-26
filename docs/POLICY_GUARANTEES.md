# Policy Guarantees Matrix

Reference for `HopperProgramPolicy` intent markers, handler unsafe-code lints,
and the separate byte/lamport write policies. The intent markers do not insert
account validation or token CPI checks by themselves.

## Named modes

| Mode | `strict` | `enforce_token_checks` | `allow_unsafe` |
|---|---|---|---|
| `HopperProgramPolicy::STRICT` | true | true | true |
| `HopperProgramPolicy::SEALED` | true | true | false |
| `HopperProgramPolicy::RAW` | false | false | true |

`STRICT` is the shipping default returned by `HopperProgramPolicy::default_policy()`.

Naming is intentionally literal:

- `STRICT` declares the author's intent to use typed account validation and
	explicit token pre-checks. Handler types and helper calls determine which
	checks execute. It permits explicit `unsafe` blocks.
- `SEALED` means `STRICT` plus `allow_unsafe = false`; handler bodies cannot
	contain unsafe code unless an instruction explicitly opts into
	`unsafe_memory`.
- `RAW` means Hopper's automatic validation/token envelope is not promised.
	The author owns every signer, owner, layout, PDA, token, and aliasing check.

Choose `STRICT` for programs using typed validation, `SEALED` to deny unsafe
code in handler items by default, and `RAW` to declare hand-validated paths.
None of these labels establishes that a program has been audited.

## What each lever controls

### `strict`

Declares the author's intent to use typed contexts (`Ctx<MyAccounts>`). The
macro does not enforce that every handler is typed: a raw `&mut Context<'_>`
handler remains raw even when `strict = true`. A typed handler always runs
`MyAccounts::bind(ctx)?` before its body, regardless of this flag. Binding
performs the checks declared by that context, including:

1. signer
2. mut / owner / executable / address
3. duplicate-writable / signer rules
4. PDA derivation
5. init / realloc / close preconditions
6. `constraint = expr`

Flipping to `strict = false` is an intent marker: the author plans to use raw `&mut Context<'_>` handlers or other hand-validated paths and accepts responsibility for calling `validate()` where needed. Typed `Ctx<T>` handlers still bind. The handler's parameter type is the final word.

### `enforce_token_checks`

Author-maintained token-check intent. The constant does not rewrite CPI code
or insert checks. For `TransferChecked`, `BurnChecked`, and `ApproveChecked`,
the explicit strict invocation methods provide these pre-checks:

| Invocation | Pre-check | Where |
|---|---|---|
| `invoke_strict()` | Authority has signer privilege, and the token account's `owner` field matches it | `require_authority_signed_direct` and `require_token_authority` in `crates/hopper-runtime/src/token.rs` |
| `invoke_signed_strict(seeds)` | Token account's `owner` field matches authority | `require_token_authority`; Solana validates the supplied PDA signer seeds during CPI |

The signed strict path does not require the PDA authority to arrive with
signer privilege. The direct strict path returns `MissingRequiredSignature`
when that privilege is absent; both return `IncorrectAuthority` on owner-field
mismatch. This field comparison does not establish token-program ownership or
validate the complete token account layout. The invoked SPL Token program
still validates the operation and its authorization. These owner-specific
helpers do not cover every valid delegate or multisig operation.

The policy constant does not rewrite or statically inspect arbitrary CPI code.
Calls that bypass the strict helpers are outside this promise and require code
review; `#[instruction(..., skip_token_checks)]` records an intentional
per-handler exception.

Flipping to `enforce_token_checks = false` changes the declared intent; it does
not remove checks from explicit helper calls. The SPL program's checks still
run whenever it is invoked.

### `allow_unsafe`

When true (default), handler bodies can contain `unsafe { ... }` blocks and the `hopper_unsafe_region!` macro.

When false, the program macro emits `#[deny(unsafe_code)]` on every handler that does not carry `#[instruction(N, unsafe_memory)]`. Any stray `unsafe { ... }` fails to compile. The per-instruction override restores unsafe for a single handler without affecting the rest of the module.

The lint applies to the handler item. It does not audit called helpers,
dependencies, or other module items, and `SEALED` is not a proof that the
whole program contains no unsafe implementation. Ordinary Rust lint override
rules still apply.

## What each policy drops

| Policy | Dropped invariant | What this means |
|---|---|---|
| `strict = false` | Declared intent to use typed contexts throughout | Raw handlers own their validation under either flag value. Typed-context handlers still bind. |
| `enforce_token_checks = false` | Declared token pre-check intent | Explicit helper checks remain unchanged; the flag does not remove or insert CPI validation. |
| `allow_unsafe = false` | Default permission for unsafe code in handler items | The macro adds `#[deny(unsafe_code)]` unless the handler opts in via `#[instruction(N, unsafe_memory)]`. |
| `#[instruction(N, unsafe_memory)]` | Program-level `#[deny(unsafe_code)]` for this handler only | Raw pointer access restored for this one handler. Other handlers stay sealed. |
| `#[instruction(N, skip_token_checks)]` | Program-level token-check promise for this handler | Author documents why the checks are upheld elsewhere (or not needed). |

## Zero-cost property

Every lever is a compile-time `bool` on a `Copy + const` struct. Readers call
`HOPPER_PROGRAM_POLICY.<lever>` in `const` context; branches that actually
consult a lever can fold to one code path during codegen. The policy value has
no runtime state, thread-local, or syscall. `RAW` does not remove validation
from a typed `Ctx<T>` handler: typed handlers still bind. A raw `&mut Context`
handler that deliberately omits Hopper validation can avoid that validation
cost, while the author assumes every omitted invariant.

## Grep receipts

An auditor lands in the tree and wants a one-command inventory of every raw-pointer region:

```sh
grep -rn "hopper_unsafe_region!" crates/ examples/
```

This finds named `hopper_unsafe_region!` calls. It does not enumerate ordinary
unsafe blocks elsewhere. The macro expands to `unsafe { ... }`, so the name
provides a review hook without changing that block's codegen.

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

A `Seq<'a, T>` tail (see [DYNAMIC_TAILS.md](DYNAMIC_TAILS.md))
is an open-ended, growable list. Under `#[hopper::context(strict_writes)]` it is
declared with `tail(<field>)`, which compiles to a single **open-ended write
range**, `WriteRange::tail_from(idx, HEADER_LEN + <Layout>::<FIELD>_OFFSET)` =
`{ offset: TAIL_PREFIX_OFFSET, size: u32::MAX }`.

What this guarantees:

| Property | Guarantee |
|---|---|
| Tail is writable and growable | The policy permits writes within the allocated tail; ordinary bounds, writability, framing, and borrow checks still apply. `push`/`set`/`swap_remove` use the gated `ctx.tail_seq_mut::<T>(idx, off)` cursor. Growth via `realloc` needs no range re-declaration, but still follows resize and funding requirements. |
| Fixed head stays byte-protected | The range starts *past* the head (`offset != 0`), so every head byte lies outside it. A write to any head field is refused at acquisition with `Custom(0xD000 \| idx)`. |
| Writable CPI delegation under `strict_writes, lamports(...)` | With the lamport dimension declared, writable delegation through validated CPI helpers requires both a whole-account data grant and lamport permission. A tail-only range fails that test. Bare `strict_writes` intentionally leaves writable CPI delegation and direct lamport mutation ungoverned. |
| One tail lease per acquire | Acquiring the cursor registers one segment lease over the whole tail region, not one per element. Large element counts therefore do not create per-element touch records. The instruction-wide touch log still has a finite capacity and can overflow from other distinct acquires. |

Structural rules:

- **One growable tail per account.** The `[count][elems]` framing is the whole
  tail; put other dynamic data in the fixed head or a separate account.
- **Growth cap: 10,240 B per instruction.** Growing the tail is a `realloc`, so
  it inherits Solana's `MAX_PERMITTED_DATA_INCREASE`.

The honest limit (per-element isolation):

> The declared `tail_from` range covers the **entire tail region** as one grant.
> The write policy therefore isolates the *head from the tail*, and the tail of
> one account from every other account; it does **not** isolate one tail element
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
