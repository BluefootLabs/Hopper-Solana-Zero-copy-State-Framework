# Devnet release evidence

Hopper's devnet release lane is a transaction test and an artifact identity
check. An executable program account alone is not enough evidence because it
may contain an older build.

The lane uses only `https://api.devnet.solana.com` and requires the canonical
devnet genesis hash. It also requires:

- a clean, committed source tree;
- a fresh program id controlled by an explicit devnet-only signer;
- the JSON deployment receipt with its program id and transaction signature;
- a local release ELF built from that source commit;
- a finalized transaction receipt written to `HOPPER_DEVNET_RECEIPT`;
- the loader ProgramData address, deployment slot, and upgrade authority;
- an on-chain program dump whose SHA-256 exactly matches the local ELF; and
- identical program metadata and bytes before and after the transaction test.

## Receipt contract

Every enforced live runner writes `hopper.devnet-evidence.v1` JSON. Common
fields are:

- `example` identifies the exact example lane;
- `commitment` is `finalized`;
- `cluster.genesis_hash`, `cluster.node_version`, and
  `cluster.feature_set` identify the RPC view;
- `program_id`, or `programs` for a multi-program lane, identifies the tested
  deployment; and
- `transactions` contains every signature and its finalized slot, including
  expected rejection cases.

Example-specific state fields record the assertions made by that runner. They
are evidence for those named behaviors only. They are not evidence that every
framework feature has been exercised on devnet.

## Artifact capture

Run the capture script immediately before and after the test. The output
directory must be a new ignored directory, normally below
`target/hopper/devnet-evidence/`.

```powershell
pwsh scripts/capture-devnet-program-evidence.ps1 `
  -Phase Before `
  -Example hopper-migration `
  -ProgramId <FRESH_PROGRAM_ID> `
  -LocalElf target/hopper/release/hopper_migration.so `
  -KeypairPath target/hopper/devnet-release/payer.json `
  -DeploymentReceiptPath target/hopper/devnet-release/hopper-migration-deploy.json `
  -OutputDirectory target/hopper/devnet-evidence/hopper-migration `
  -SolanaCli D:\path\to\agave-v4.2.1\solana.exe

$env:HOPPER_DEVNET = '1'
$env:HOPPER_REQUIRE_DEVNET = '1'
$env:HOPPER_DEVNET_RECEIPT = 'target/hopper/devnet-evidence/hopper-migration-receipt.json'
$env:HOPPER_KEYPAIR = 'target/hopper/devnet-release/payer.json'
$env:HOPPER_MIGRATION_PROGRAM_ID = '<FRESH_PROGRAM_ID>'
cargo test -p hopper-migration --test devnet --locked -- --nocapture

pwsh scripts/capture-devnet-program-evidence.ps1 `
  -Phase After `
  -Example hopper-migration `
  -ProgramId <FRESH_PROGRAM_ID> `
  -LocalElf target/hopper/release/hopper_migration.so `
  -KeypairPath target/hopper/devnet-release/payer.json `
  -DeploymentReceiptPath target/hopper/devnet-release/hopper-migration-deploy.json `
  -OutputDirectory target/hopper/devnet-evidence/hopper-migration `
  -ReceiptPath target/hopper/devnet-evidence/hopper-migration-receipt.json `
  -SolanaCli D:\path\to\agave-v4.2.1\solana.exe
```

The final bundle contains both on-chain dumps, both `program show` records,
the local release ELF, the finalized receipt, a provenance record,
`SHA256SUMS`, and a hash of that checksum list. A release evidence archive is
valid only when it is bound to the final source commit and retained as a
content-addressed release asset.

## Current status

The harness and capture contract are implemented. Final 0.3.0 public-devnet
receipts must still be produced from the release commit before any current
deployment claim is made.
