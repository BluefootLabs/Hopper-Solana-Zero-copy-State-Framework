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

## Heap, panics, SlotHashes, and PDAs off chain

```rust
// A 256 KiB heap. Transactions that allocate past 32 KiB request the frame
// (`hopper tx send --heap-frame 262144`); the others work without it.
hopper_native::default_allocator!(heap = 256 * 1024);
```

The allocator hands out memory forward from above Hopper's scratch region
and grows the most recent block in place, so a vector that grows to 200 KiB
occupies 200 KiB. `heap::mark()` and `heap::release_to(mark)` rewind it
inside a loop; `heap::used()` says how much is taken.

The `panic-location` feature makes a panic report `file:line:column`
through `sol_panic_`, and `panic-message` logs the message. Without them a
panic aborts silently, which is what a production build wants.

`slot_hashes::slot_hash_lookup(slot)` finds a slot's hash with partial reads
of the 20 KB SlotHashes sysvar, one read for a recent slot and two for most
others, and says why there is none: `Skipped`, `TooOld`, or `Ahead`.

`pda::find_program_address` and the other PDA functions run in a plain
`cargo test` with the cluster's answers; the curve check is a const Ed25519
decompression in Rust. `pda::find_program_address_const(seeds, program_id)`
derives an address and bump at compile time.

In 0.5, `AccountView::layout_id` returns `Option<[u8; 8]>` by value and
`DataFingerprint::capture` returns a `Result`; both read under the borrow
rules now. See the
[migration notes](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/docs/MIGRATION_0_5.md).

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
and invalid offsets in every account it turns into a view, instead of
resolving them to account zero. Records past the entrypoint's bound are never
viewed; the walk crosses them by size alone. See the
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

## Processed sibling instructions

`introspect::get_processed_instruction_into` reads a prior sibling into caller-owned data
and account buffers, without heap allocation. It queries exact sizes before
copying, returns only the initialized prefixes, and distinguishes absence from
insufficient capacity. Account records include the address and signer/writable
flags. The list contains earlier calls at the same depth and caller; the
current instruction's parent and children are excluded.

This release corrects the previous wrappers' syscall return-code and length
handling. The owned convenience reader retains its 1,232-byte / 64-account limit;
the new API lets the program choose its scratch capacities. Host calls return
absence because host stubs have no instruction trace. Program-ID inspection does
not authorize a transfer or validate a signature payload.

[Instruction inspection guide](https://hopperzero.dev/docs/instruction-introspection).
