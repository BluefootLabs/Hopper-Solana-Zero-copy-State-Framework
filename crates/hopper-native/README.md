# hopper-native

Low-level runtime backend for Hopper programs on Solana. This crate owns raw
loader parsing, syscall wrappers, entrypoint glue, the substrate `AccountView`,
and duplicate-account resolution.
It exposes SHA-256, Keccak-256, BLAKE3, curve, and secp256k1 syscall bindings
without pulling framework code into the raw substrate.

Hopper's hash wrappers reject too many segments instead of silently dropping
bytes. The public crypto matrix is maintained in
[`docs/CRYPTO_CAPABILITIES.md`](../../docs/CRYPTO_CAPABILITIES.md).

Part of the **[Hopper](https://hopperzero.dev)** framework.

## Entrypoints

- **`hopper_program_entrypoint!`** (alias `program_entrypoint!`) - standard
  eager parse. Stack-allocates `[MaybeUninit<AccountView>; MAX]`, scans the
  whole input up front for low-overhead account access.
- **`hopper_fast_entrypoint!`** (alias `fast_entrypoint!`) - uses the SVM
  two-argument entrypoint register; reads instruction data directly. Saves
  roughly 30 to 40 CU per call vs the eager variant.
- **`hopper_lazy_entrypoint!`** (alias `lazy_entrypoint!`) - defers account
  parsing entirely. Returns a `LazyContext` that materialises accounts on
  demand. Substantial CU win on dispatch-heavy programs where most variants
  touch a subset of supplied accounts.

## Safety posture

The audited unsafe surface is enforced by
[`scripts/check-unsafe-safety-comments.py`](../../scripts/check-unsafe-safety-comments.py):
every `unsafe` block needs a nearby `SAFETY:` comment, and every public unsafe
function needs a rustdoc `# Safety` section. The full inventory is at
[`docs/UNSAFE_INVARIANTS.md`](../../docs/UNSAFE_INVARIANTS.md).

The duplicate-account marker handler traps on forward references, self-loops,
or any invalid offset rather than silently falling through to account zero
(a real footgun that would have made attacker-supplied account substitutions
possible). See `raw_input.rs::malformed_duplicate_marker`.

Docs: <https://docs.rs/crate/hopper-native/0.2.1>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
