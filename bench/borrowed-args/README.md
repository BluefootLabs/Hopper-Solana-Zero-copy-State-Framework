# Checked borrowed arguments on Solana

This fixture exercises the unreleased checked argument parsers in an SBF
program with no allocator. Its 12-byte prefix contains two optional enum
values and a `WireU64` amount. Mode `0` requires an exact prefix; mode `1`
allows a UTF-8 suffix of at most 32 bytes. Both views must point into the
original instruction buffer. A successful call increments a 16-byte test
account's count and total and returns those bytes.

Modes `2` and `3` perform the same update through generated `#[hopper::program]`
handlers accepting `&Update`, with an explicit `&[u8]` parameter in mode `3`.
The shared update function checks pointer identity, account privileges, tail
semantics, arithmetic, and the state transition for both authoring paths.

The state must belong to the program, be writable, and have exactly 16 bytes.
The second account must sign. This is a parser fixture: any signer is accepted;
it is not a custody program or an application authorization model.

## Compiled VM checks

```sh
cargo build-sbf --manifest-path bench/borrowed-args/program/Cargo.toml --arch v3 -- --locked
HOPPER_BORROWED_ARGS_SBF="$PWD/target/deploy/hopper_borrowed_args_fixture.so" \
  cargo test -p hopper-framework-verifier --test borrowed_args_sbf --locked -- --ignored
```

Repeat with `--arch v0` to check the older binary format. The test requires
an explicit existing artifact and fails if it cannot load one. It executes
1,054 instructions per dispatch path, including all 256 values in four tag/payload positions,
every truncation, exact-length surplus, empty and bounded tails, invalid UTF-8,
missing signer, wrong owner, read-only state, invalid account lengths, and both
counter and total overflow. Refusals must leave every supplied account intact.
Both modes run for 2,108 instructions per architecture. Three sampled cases
also print CU measurements; they compare authoring paths in this fixture,
including dispatch, validation, writes, and return data.

## Public devnet

```sh
python scripts/test-borrowed-args-devnet.py snapshot --out target/args-run/source.json
# Build the fixture after taking the snapshot; deploy it to public devnet.
python scripts/test-borrowed-args-devnet.py run \
  --program PROGRAM_ID --payer DEVNET_PAYER --hopper target/debug/hopper \
  --elf target/deploy/hopper_borrowed_args_fixture.so \
  --source-snapshot target/args-run/source.json --out target/args-run/devnet
```

The runner checks the devnet genesis, compares the deployed ELF before and
after testing, validates exact errors and return-data provenance, and compares
complete state account snapshots. Add `--generated-dispatch` to exercise modes
`2` and `3`; the default retains manual dispatch. Each run creates a fresh
state account. It captures the shared quality-gate source inventory, including
Markdown and workflow/configuration inputs, with CRLF/LF normalization, and
refuses a source change after the snapshot. The
snapshot includes uncommitted work; it does not substitute for a clean source
commit or a reproducible release build. Use a fresh snapshot and rebuild after
source changes. The output directory must be new and under ignored `target/`.

The [October 7 verification](../../audit/borrowed-dispatch-2026-10-07/README.md)
records the manual and generated dispatch checks and their measured scope.
The earlier [October 2 devnet receipts](../../audit/borrowed-args-2026-10-02/README.md)
retain the original manual-parser source and binary, successful writes, and
expected refusals.
