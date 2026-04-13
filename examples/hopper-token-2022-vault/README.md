# hopper-token-2022-vault

This example shows a Hopper-authored Token-2022 treasury flow built on Hopper-owned companion crates:

- `hopper_associated_token::CreateIdempotent`
- `hopper_token_2022::MintTo`
- `hopper_token_2022::Transfer`
- whole-layout Hopper state via `load_mut()`

It also includes a package-local `hopper.manifest.json`, so the CLI transparency flow can be exercised directly from this directory.

## Try It

```bash
cargo check -p hopper-token-2022-vault

cd examples/hopper-token-2022-vault
cargo run -p hopper-cli -- compile --emit rust
cargo run -p hopper-cli -- compile --emit rust --out lowered.rs --force
cargo run -p hopper-cli -- explain context @hopper.manifest.json
```

The first compile command uses current-package manifest inference. The second writes the lowered preview to disk so you can inspect the generated accessor surface.

## Devnet Proof

Use the bundled PowerShell flow to run Hopper's transparency path first and then deploy the example to devnet through `hopper-cli`:

```powershell
pwsh ./examples/hopper-token-2022-vault/devnet-proof.ps1 -SkipDeploy
pwsh ./examples/hopper-token-2022-vault/devnet-proof.ps1
pwsh ./examples/hopper-token-2022-vault/devnet-proof.ps1 -KeypairPath C:\temp\hopper-devnet.json
```

The first invocation verifies the local DX path only: lowered Rust preview, context explanation, and SBF build. The second invocation also calls `hopper deploy -p hopper-token-2022-vault --url https://api.devnet.solana.com --output json` and writes the deploy receipt next to the example.
Use `-KeypairPath` when you want an explicit devnet-only deploy authority instead of the keypair configured in `solana config get`.