# hopper-native

Low-level runtime backend for Hopper programs on Solana. This crate owns raw
loader parsing, syscall wrappers, entrypoint glue, the substrate `AccountView`,
and duplicate-account resolution.
It exposes SHA-256, Keccak-256, BLAKE3, curve, and secp256k1 syscall bindings
without pulling framework code into the raw substrate.

Hopper's hash wrappers reject too many segments instead of silently dropping
bytes. The public crypto matrix is maintained in
[`docs/CRYPTO_CAPABILITIES.md`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/CRYPTO_CAPABILITIES.md).

Part of the **[Hopper](https://hopperzero.dev)** framework.

## Entrypoints

- **`hopper_program_entrypoint!`** (alias `program_entrypoint!`) - standard
  eager parse. Stack-allocates `[MaybeUninit<AccountView>; MAX]`, scans the
  whole input up front for low-overhead account access.
- **`hopper_fast_entrypoint!`** (alias `fast_entrypoint!`) - uses the SVM
  two-argument entrypoint register and reads instruction data directly when
  the `simd-0321` feature is enabled. Controlled Hopper fixtures measured the
  current dual path as CU-neutral while adding about 368 bytes of `.text`, so
  it remains an explicit size/toolchain choice rather than a claimed CU win.
- **`hopper_lazy_entrypoint!`** (alias `lazy_entrypoint!`) - defers account
  parsing and passes the handler a `LazyContext` that materialises accounts on demand.
  It can reduce parsing work when an instruction touches only a subset of the
  supplied accounts; measure the actual program because the result is shape-
  and dispatch-dependent.

## Safety posture

The internally inventoried unsafe surface is enforced by
[`scripts/check-unsafe-safety-comments.py`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/scripts/check-unsafe-safety-comments.py):
every `unsafe` block needs a nearby `SAFETY:` comment, and every public unsafe
function needs a rustdoc `# Safety` section. The full inventory is at
[`docs/UNSAFE_INVARIANTS.md`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/UNSAFE_INVARIANTS.md).

The duplicate-account marker parser rejects forward references, self-loops,
and invalid offsets instead of resolving them to account zero. See the
`malformed_duplicate_marker` trap in `src/raw_input.rs` and its
`forward_duplicate_marker_is_rejected` and `self_duplicate_marker_is_rejected`
regression tests.

Docs: <https://docs.rs/crate/hopper-native>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

MIT OR Apache-2.0. See [LICENSE-MIT](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/LICENSE-MIT) and [LICENSE-APACHE](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/LICENSE-APACHE).
