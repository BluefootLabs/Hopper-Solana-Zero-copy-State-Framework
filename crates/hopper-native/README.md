# hopper-native

Hopper's low-level runtime backend for Solana. Owns raw loader parsing,
syscall wrappers, entrypoint glue, substrate `AccountView`, and
duplicate-account resolution.

Part of the **[Hopper](https://hopperzero.dev)** framework.

## Three entrypoint variants

- **`hopper_program_entrypoint!`** (alias `program_entrypoint!`) — standard
  eager parse. Stack-allocates `[MaybeUninit<AccountView>; MAX]`, scans the
  whole input up front. Pinocchio-class CU efficiency.
- **`hopper_fast_entrypoint!`** (alias `fast_entrypoint!`) — uses the SVM
  two-argument entrypoint register; reads instruction data directly. Saves
  ~30–40 CU per call vs the eager variant.
- **`hopper_lazy_entrypoint!`** (alias `lazy_entrypoint!`) — defers account
  parsing entirely. Returns a `LazyContext` that materialises accounts on
  demand. Substantial CU win on dispatch-heavy programs where most variants
  touch a subset of supplied accounts.

## Safety posture

Every `unsafe` block in this crate has a documented `# Safety` invariant in
the doc comment. The full inventory is at
[`docs/UNSAFE_INVARIANTS.md`](../../docs/UNSAFE_INVARIANTS.md).

The duplicate-account marker handler traps on forward references, self-loops,
or any invalid offset rather than silently falling through to account zero
(a real footgun that would have made attacker-supplied account substitutions
possible). See `raw_input.rs::malformed_duplicate_marker`.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
