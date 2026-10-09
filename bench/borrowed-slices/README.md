# Borrowed batch fixture

A Solana program exercising `BoundedSlice` through manual (tag `0`) and
generated (tag `1`) dispatch. Both paths consume `[tag][u16 count][orders][u16
nonce]`; each order is a two-byte optional enum and an eight-byte little-endian
amount. At most 32 orders are allowed, and the nonce must equal 513.

The program uses no allocator. It checks that the element view points to byte
3 of the original instruction, sums the amounts with checked arithmetic, and
increments a 16-byte state account's item count and total. The state must be
writable and owned by the program; a second account must sign. Any signer is
accepted in this parser fixture. It is not a custody program.

```sh
cargo build-sbf --manifest-path bench/borrowed-slices/program/Cargo.toml --arch v3 -- --locked
HOPPER_BORROWED_SLICES_SBF="$PWD/target/deploy/hopper_borrowed_slices_fixture.so" \
  cargo test -p hopper-framework-verifier --test borrowed_slices_sbf --locked -- --ignored --nocapture
```

Repeat with `--arch v0`. Use a fresh Cargo target when collecting evidence from
a changed source checkout. The test requires an explicit ELF and fails if it
cannot load it. It checks exact return data and state, every option/enum byte
in a later element, truncations, extra bytes, excessive counts, zero and maximum
batches, authority/owner/writable errors, and arithmetic overflow. Refusals
must preserve every supplied account. Sampled CU values describe this fixture
only; manual and generated paths perform the same transition.

Generated clients can be checked with locally installed compiler/SDK packages:

```sh
python scripts/test-borrowed-slice-clients.py --out target/slice-clients \
  --typescript /path/to/node_modules/typescript \
  --web3 /path/to/node_modules/@solana/web3.js
```

The check generates clients from typed handler metadata, compiles the Rust
output against Solana SDK instruction/key types, and executes the TypeScript
output with the real web3.js SDK. It verifies independent expected bytes,
counts, element widths, UTF-8 lengths, and account privileges.
It also executes the dependency-free Python builders against the same wire
contract, including aliased vector, string, scalar and fixed-array arguments.

## Devnet

```sh
python scripts/test-borrowed-slices-devnet.py snapshot --out target/batch-run/source.json
# Build the fixture and deploy the resulting ELF to public devnet.
python scripts/test-borrowed-slices-devnet.py run \
  --program PROGRAM_ID --payer DEVNET_PAYER --hopper target/debug/hopper \
  --elf target/deploy/hopper_borrowed_slices_fixture.so \
  --source-snapshot target/batch-run/source.json --out target/batch-run/manual
```

Use a fresh output directory and `--generated-dispatch` for tag `1`. Each run
creates a fresh state account, checks the devnet genesis and deployed ELF
before and after execution, and records finalized transactions, exact errors,
return-data provenance, and whole-state snapshots. Source changes after the
pre-build snapshot stop the run. Private keys stay in ignored output.

`--owned-aliases` selects tag `2`: aliased `BoundedVec<u64, 32>`,
`BoundedString<8>` and `u16` arguments. This path checks UTF-8, capacities,
following-argument offsets, account privileges, overflow and refused writes.
It intentionally owns its decoded elements; only the borrowed paths assert
pointer identity with the input buffer.
