# hopper-token-2022-vault

This example shows a Hopper-authored Token-2022 treasury flow built on Hopper-owned companion crates:

- `hopper_associated_token::CreateIdempotent`
- `hopper_token_2022::MintTo`
- Hopper's checked interface transfer into the canonical Token-2022 program
- whole-layout Hopper state via `load_mut()`

`PrepareVaultAta` is authority-continuous: the authority stored by initialization
must sign, and the mint plus ATA binding can be established once but never
rebound. The mint and token-account shapes, Token-2022 ownership, executable
program accounts, arithmetic, and stored policy are validated before CPI.
`SweepRewards` includes the bound mint and uses `TransferChecked` with its
verified decimals. The shared `check_safe_token_2022_mint` policy accepts base
mints and reviewed metadata/group descriptor extensions only. It rejects
balance-, authority-, transfer-, burn-, display-, and account-state-affecting
extensions, malformed or duplicate TLVs, and unknown future extension ids.

The relevant account tails are:

- `PrepareVaultAta`: System Program, Token-2022 Program, Associated Token Program
- `SweepRewards`: destination ATA, bound mint, Token-2022 Program

The raw dispatcher exports a source-owned `PROGRAM_MANIFEST`. The checked-in
`hopper.manifest.json` and `lowered.rs` are generated from that static. Because
this example does not use an enforced typed context, the manifest deliberately
publishes no policy pack, receipt expectation, strict-write contract, or CU
estimate.

## Try It

```bash
cargo check -p hopper-token-2022-vault
cargo run --locked -p hopper-cli -- compile --emit manifest --package hopper-token-2022-vault
cargo run --locked -p hopper-cli -- compile --emit rust --package hopper-token-2022-vault --out examples/hopper-token-2022-vault/lowered.rs --force
cargo run --locked -p hopper-cli -- explain instruction @examples/hopper-token-2022-vault/hopper.manifest.json prepare_vault_ata
```

Regenerate the manifest first. Every other emitter consumes that package-local
JSON rather than reading the Rust static directly. Unit coverage requires the
checked-in JSON to equal the current source rendering and pins each raw
instruction's tag, argument shape, account order, signer bits, and writable
bits.

## Devnet Proof

The compiled-SBF test drives the real Hopper artifact against Mollusk's
canonical Token-2022 and Associated Token Account ELFs. It covers unauthorized
prepare, one-time binding, rebind refusal, mint, and checked sweep:

```bash
cargo build-sbf --manifest-path examples/hopper-token-2022-vault/Cargo.toml -- --locked
HOPPER_REQUIRE_TOKEN_2022_VAULT_SBF=1 \
  cargo test -p hopper-token-2022-vault --test sbf_e2e -- --nocapture
```

After deploying that fresh artifact to a fresh devnet program id, the opt-in
transaction harness verifies the public-devnet genesis, waits for finalized
signatures, re-reads state and canonical token accounts at finalized, and emits
a redacted JSON receipt:

```bash
HOPPER_DEVNET=1 \
HOPPER_REQUIRE_DEVNET=1 \
HOPPER_TOKEN_2022_VAULT_PROGRAM_ID=<FRESH_PROGRAM_ID> \
HOPPER_KEYPAIR=/abs/path/devnet-only-keypair.json \
HOPPER_DEVNET_RECEIPT=/abs/path/token-2022-vault-receipt.json \
cargo test -p hopper-token-2022-vault --test devnet -- --nocapture
```

The PowerShell release helper requires an explicit devnet payer and program
keypair, builds into a new isolated output directory, deploys the exact ELF,
writes the deployment receipt, and captures the `Before` evidence phase. The
program keypair and every generated output path must be below this repository's
`target` directory, while the payer keypair may remain outside the repository:

```powershell
New-Item -ItemType Directory -Force .\target\release-evidence
solana-keygen new --no-bip39-passphrase --silent --outfile .\target\release-evidence\token-2022-vault-program.json

pwsh ./examples/hopper-token-2022-vault/devnet-proof.ps1 `
  -KeypairPath C:\path\to\devnet-only-payer.json `
  -ProgramKeypairPath .\target\release-evidence\token-2022-vault-program.json `
  -SbfOutDirectory .\target\release-evidence\token-2022-vault-sbf `
  -DeployReceipt .\target\release-evidence\token-2022-vault-deploy.json `
  -EvidenceDirectory .\target\release-evidence\token-2022-vault-before
```

All five paths are mandatory. The helper refuses a dirty tree, a non-public
devnet endpoint, an unexpected Solana CLI version, pre-existing output paths,
or an SBF build without `--locked`. Use the printed program id in the finalized
transaction test above. After the test succeeds, capture the `After` phase and
seal the archive by following
[`docs/DEVNET_RELEASE_EVIDENCE.md`](../../docs/DEVNET_RELEASE_EVIDENCE.md).
