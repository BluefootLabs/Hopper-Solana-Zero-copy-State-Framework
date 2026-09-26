# Competitive refresh, 2026-09-22

The remaining counter size gap exposed a linker problem: every mutable
borrow referenced the full ambient policy evaluator even when the program
had no way to install a policy. The evaluator was cold at runtime but still
occupied about 1.7 KB of deployed code.

## Write-policy evaluator registered at installation

SBF gate state now starts with an optional checker function in reserved VM
heap memory. It starts as `None` in the VM's zeroed heap. Successful
installation registers the evaluator; removing the last live guard clears
it. A mutation site reads the checker and calls it only when present.
Only installation references the evaluator's symbol, allowing the linker
to discard that function in programs without an installation path.

Policy semantics remain intact: an inner policy shadows its outer policy,
failed installation leaves the active policy intact, out-of-order drops
cannot clear another guard, and leaking a guard keeps its policy active.
The store still copies account addresses rather than retaining account
references. The callback is an internal program function, not a user-supplied
pointer. The scratch layout grows by one pointer; the touch log follows the
updated end offset and compile-time bounds keep both inside the reservation.

| Fixture | Before | After | Change |
| --- | ---: | ---: | --- |
| Macro counter ELF | 10,056 bytes | 8,304 bytes | 17.4% smaller |
| Macro initialize | 1,552 CU | 1,549 CU | 3 CU less |
| Macro increment | 358 CU | 358 CU | unchanged |
| Substrate counter ELF | 8,256 bytes | 6,728 bytes | 18.5% smaller |
| Substrate initialize | 1,598 CU | 1,606 CU | 8 CU more |
| Substrate increment | 1,741 CU | 1,742 CU | 1 CU more |

Hello rows are unchanged. The rebuilt Pinocchio reference remains identical
to the pinned published row. The substrate tradeoff is explicit: 1,528 bytes
saved at a small compute cost. Installed policies retain the evaluator and
pay an indirect call; the smaller no-policy binary is not a claim that
policy-heavy workloads became faster.

The macro counter is now smaller than the pinned Anchor v2 row (8,696
bytes). Quasar still has the smaller macro counter (7,808 bytes) and cheaper
increment (330 CU). The fixtures differ in account layout and bump handling,
so initialization results do not establish a universal framework ranking.
The complete recipe and table are in
[framework-comparison](../bench/framework-comparison/README.md).

## Source review

The inspected revisions are pinned below. These source revisions are
separate from the older revisions behind the published benchmark rows.

- [Quasar `b0de7db4`](https://github.com/blueshift-gg/quasar/tree/b0de7db4cd271654a2dcf78807dd865e98e0b339):
  the current head still uses generated account parsing and a SHA-256 PDA
  verification path for validated accounts. Hopper already uses the latter
  on its macro path. The old table note saying Quasar did not verify the PDA
  was incorrect and has been fixed in the generator.
- [Pina `b0106aa5`](https://github.com/pina-rs/pina/tree/b0106aa564ab5badb6ae64ca84f84b3b0c0c9dab):
  its framework-comparison page still publishes the same four-framework
  numbers. Its latest commit fixes the scaffolded CPI crate's workspace
  dependency. The controlled fixtures remain useful, but their published
  rows are not measurements of every framework's current head.
- [Anchor v2 `a4890633`](https://github.com/otter-sec/anchor/tree/a4890633c4deccc56585e89accccc8fcfcf6b084):
  `lang-v2/derive/src/pda.rs` and `parse.rs` precompute the canonical bump
  **and address**, then emit a constant address comparison. Hopper's
  `const_pda!` is not unique: it requires a supplied bump and performs the
  hash at compile time. Anchor also has default-on `guardrails` and
  default-off `const-rent`. Pina's pinned Anchor fixture disables default
  features and enables only `alloc`; its table does not measure default
  guardrails. Hopper retains live rent and its ordinary guard APIs here.

## Validation and next targets

The [SBF regression fixture](../bench/runtime-gate/README.md) executes
positive writes and policy refusals, exact-cell selection, nested and
out-of-order guard lifetimes, failed installation, leaks, and fresh VM state.
It checks exact errors and complete account snapshots on both SBF v0 and v3.
The Solana workflow now runs those tests explicitly in Mollusk.

Local validation: the full locked workspace suite passed (2,199 passed,
223 ignored tests/examples), workspace/all-target Clippy passed with warnings
denied, formatting passed, and the unsafe-contract check covered all 29
public packages. The SBF regression also passed with `touch-map` enabled,
exercising the heap consumer that follows the gate store.

The next measured targets are the macro hello's 22 CU overhead over the
substrate, generated parsing that shares work with validation, and automatic
canonical literal-PDA derivation. Each needs a separate fixture and
rejection tests. None is counted as a performance gain in this report.
