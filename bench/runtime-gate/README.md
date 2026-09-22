# Ambient gate SBF regression

This fixture exercises the compiled runtime through its public account APIs.
It covers byte-range enforcement, foreign-account refusal, nested guards,
failed installation at maximum depth, out-of-order guard drops, parametric
cells, leaked guards, lamport writes, account transitions, and fresh VM state
on the next invocation. Every failed instruction must leave both accounts
unchanged; successful instructions must change only the expected bytes.

```powershell
cargo build-sbf --manifest-path bench/runtime-gate/program/Cargo.toml --sbf-out-dir target/hopper/runtime-gate
$env:HOPPER_GATE_SBF = (Resolve-Path target/hopper/runtime-gate/hopper_runtime_gate_fixture.so).Path
cargo test -p hopper-framework-verifier --test ambient_gate_sbf --locked -- --ignored --nocapture
```

Repeat with `--arch v3` and a separate output directory to verify indirect
dispatch on SBF v3. The Solana SBF workflow runs both versions in Mollusk.
The test is explicitly ignored in ordinary host runs because it requires a
freshly compiled ELF; the commands above and CI explicitly execute it.

These checks are separate from the cross-framework benchmark table. The
fixture intentionally installs policies, so it retains the policy evaluator
and exercises the function-pointer relocation and invocation on SBF.

For a live check, deploy that same fixture to devnet and run
`scripts/test-runtime-gate-devnet.py --program <id> --payer <devnet-keypair>
--hopper <hopper-binary> --elf <deployed-fixture.so>` from a clean committed source tree. The script
uses only the public devnet endpoint, creates two test accounts, waits for
finalized transactions, checks complete account snapshots, and writes a
public receipt plus transaction records under `target/hopper/`. It verifies
the deployed ELF against the local file before and after the run. Private
keypairs stay in its `keys/` subdirectory and must never be archived.
