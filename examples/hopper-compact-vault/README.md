# Hopper Compact Vault

Deployable proof for Hopper's 1-byte compact account path.

`Vault` is declared with `#[hopper::state(compact, disc = 1)]`, so its wire bytes are exactly:

```text
[0]      discriminator = 1
[1..33]  authority pubkey
[33..41] balance u64 little-endian
```

There is no 16-byte Hopper header in the account. The layout fingerprint is `437141907c09344f`, but clients get that from `hopper.manifest.json`, generated IDL, or generated SDK constants. Compact account validation checks exact size plus discriminator; it never reads bytes `4..12` as a header layout ID.

## Local Proof

```powershell
cargo test --manifest-path ..\..\Cargo.toml -p hopper-compact-vault --offline
```

The tests prove:

- `Vault::COMPACT_LEN == 41`
- field offsets are `authority = 1`, `balance = 33`
- the Tier-2 binary registry round-trips and marks the layout compact
- generated manifest, IDL, TypeScript, and Kotlin clients carry `437141907c09344f`
- generated clients decode compact fields from offsets `1` and `33`

## Devnet Proof

Build and deploy the program with the Solana toolchain, then run the opt-in test:

```powershell
$env:HOPPER_DEVNET='1'
$env:HOPPER_COMPACT_VAULT_PROGRAM_ID='<deployed-program-id>'
$env:HOPPER_KEYPAIR='C:\absolute\path\to\devnet-keypair.json'
$env:SOLANA_RPC_URL='https://api.devnet.solana.com'
cargo test --manifest-path ..\..\Cargo.toml -p hopper-compact-vault --test devnet -- --nocapture
```

The devnet test creates an exact 41-byte account owned by the deployed program, sends `initialize` and `deposit`, fetches the account back, and verifies the returned bytes match the compact layout. It also checks the generated manifest fingerprint constant separately, so a client cannot accidentally treat bytes `4..12` as an on-account layout header.

Latest verified deployment from this workspace:

```text
Program Id: 6aKUB52fa1KmGTh11GCuMhixKk9Sgo2nDrmsmMz8DZvs
Deploy Signature: 9QpuQjtQ8B85tDnMa7HEj7fejVLQ3SWaqMjWEjfc21CS5aeTzZcGYrqwCy5qWj7ts6BGgQxgZGxyYhPXhpggB46
Artifact Size: 4400 bytes
```

Latest verified devnet run from this workspace:

```text
Account: EvrdGfn3vYrFm8s3SggCCn5MYkViPT4utrrXEDAWSdat
Signature: 4v72Kp9ewbxFUkgEKDL16FcApuW87HTy9X2HkdFp4FjXZe4Pq3iyNuBXiPcmzEUiaUxBgQ49ttJPFR18WZLhNjgT
```

## Instructions

`initialize` uses instruction data `[0]` and accounts `[vault(w), authority(s)]`. The vault account must already exist, be program-owned, and be exactly 41 bytes.

`deposit` uses instruction data `[1][amount:u64-le]` and accounts `[vault(w), authority(s)]`. The authority signer must match the pubkey stored at byte offset `1`.
