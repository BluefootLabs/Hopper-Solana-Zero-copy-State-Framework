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

PDA helpers enforce Solana's seed domain before hashing: at most 16 total
seeds, each at most 32 bytes. Helpers that append a bump accept at most 15
base seeds. Excess seeds are refused, never truncated. The compile-time
`program_address_const` checks the same bounds but does not check the curve
or find a canonical bump. SHA-only verification requires an address already
bound to validated program-owned state or to a signed creation CPI; use the
curve-checked path for unchecked addresses. The
[compiled-SBF boundary fixture](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/bench/pda-boundaries)
checks hash-equivalent invalid inputs against the Solana SDK.

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

Apache-2.0. See [LICENSE-APACHE](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/LICENSE-APACHE).

## Native execution hardening

The 0.4.1 implementation uses the SVM abort syscall for no-allocation failures
and `no_std` panics. It requires no experimental inline assembly and terminates
without a compute-burning spin loop. Failure rolls back the transaction; it is
not a recoverable `ProgramError` return.

Specialized System and token CPI helpers reject a required signer locally when
neither an outer signature nor PDA signer seeds are supplied. Nonempty seeds do
not prove authority: Solana still derives and verifies the PDA at the CPI boundary.

`invoke_and_read<T>` verifies that the invoked program produced the return data
and that it contains an aligned `T` prefix. A nested program's unforwarded result
is rejected. `ReturnData::as_type_from<T>` offers the same producer check for an
existing snapshot; applications still validate the payload's meaning.

## Account lifecycle safety in 0.4.2

Segment guards retain their native account borrow on SBF. Conflicting access,
resizing, closure, and writable checked CPI are refused until the guard drops.
Runtime callers edit multiple fields through the checked `split_segments_mut`
API; its raw constructor is no longer public. Close refusals preserve account
state even when caught. Direct self-transfers are balance-checked net zero.

Native `Ref` / `RefMut` mapping keeps the original lease while selecting a
field. Native `batch::ResizeWithPayer` (feature `cpi`) grows program-owned state
using live rent and a checked System transfer from a wallet or System-owned
PDA. It checks growth before charging, zeroes exposed bytes, and retains excess
rent on shrink. The application must authorize the operation. Runtime write
policies do not govern APIs deliberately called at the native layer.

[Compiled and devnet evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-lifecycle-2026-09-26).
