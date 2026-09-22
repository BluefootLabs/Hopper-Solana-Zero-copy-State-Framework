# PDA seed-domain regression

The SHA-based and syscall-based paths must accept the same seed domain:
at most 16 total seeds, each at most 32 bytes. Helpers that append a bump
accept at most 15 base seeds. No helper may truncate a caller's seed list.

The fixture accepts dynamic seed inputs and exercises 12 native/runtime
entry points. The Mollusk test derives reference addresses with the Solana
SDK and compares exact success/error results and unchanged account state.
It covers empty base seeds, a 32-byte seed, 15 base seeds plus bump,
excess empty seeds, an ignored nonempty suffix, and a 33-byte seed.
The invalid empty-seed and long-seed vectors hash to valid SDK addresses
when resegmented: they detect missing structural validation directly.

```sh
cargo build-sbf --manifest-path bench/pda-boundaries/program/Cargo.toml \
  --sbf-out-dir target/hopper/pda-boundaries-v0 -- --locked
HOPPER_PDA_SBF="$PWD/target/hopper/pda-boundaries-v0/hopper_pda_boundaries_fixture.so" \
  cargo test -p hopper-framework-verifier --test pda_boundaries_sbf --locked -- --ignored --nocapture
```

The Solana workflow executes the fixture on SBF v0 and v3. These are VM
regressions, not a benchmark or proof that supplied bumps are canonical.
SHA-only verification still requires the documented account-ownership or
creation binding; unchecked accounts need curve-checked derivation.
