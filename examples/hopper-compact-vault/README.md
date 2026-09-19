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

The example tests prove:

- `Vault::COMPACT_LEN == 41`
- field offsets are `authority = 1`, `balance = 33`
- the Tier-2 binary registry round-trips and marks the layout compact
- generated manifest, IDL, TypeScript, and Kotlin clients carry `437141907c09344f`
- generated TypeScript/Kotlin clients decode fields from offsets `1` and `33`

The `hopper-schema` generator suite separately pins Python, Go, C, and
off-chain Rust compact decoders to the same exact-size/discriminator guard, so
all six emitted SDK targets now share that fail-closed boundary.

## Devnet Proof

Build and deploy the program with the Solana toolchain, then run the opt-in test:

```powershell
$env:HOPPER_DEVNET='1'
$env:HOPPER_COMPACT_VAULT_PROGRAM_ID='<deployed-program-id>'
$env:HOPPER_KEYPAIR='C:\absolute\path\to\devnet-keypair.json'
$env:SOLANA_RPC_URL='https://api.devnet.solana.com'
cargo test --manifest-path ..\..\Cargo.toml -p hopper-compact-vault --test devnet -- --nocapture
```

When `HOPPER_DEVNET` is set it must be exactly `1`; any other value fails
instead of silently skipping. The test verifies the devnet genesis hash and
deployed program account, creates and initializes an exact 41-byte account,
and polls transactions and account state at finalized commitment. It then
submits a wrong-authority deposit, proves the full account snapshot is
unchanged, and sends an authorized deposit with an exact balance assertion.

A passing run emits one line prefixed with
`HOPPER_DEVNET_EVIDENCE_JSON=`. The deterministic JSON shape binds the cluster
genesis and node version, program id, public account ids, finalized signatures
and slots, exact balances, and pre/post account SHA-256 values. It contains no
keypair path or RPC URL; custom RPC endpoints are reported as `redacted`.

`account_sha256` hashes this canonical byte sequence: lamports as little-endian
u64, owner pubkey, executable as one byte, rent epoch as little-endian u64,
data length as little-endian u64, then the complete account data.

Historical deployment record (not a fresh finalized attestation):

```text
Program Id: 6aKUB52fa1KmGTh11GCuMhixKk9Sgo2nDrmsmMz8DZvs
Deploy Signature: 9QpuQjtQ8B85tDnMa7HEj7fejVLQ3SWaqMjWEjfc21CS5aeTzZcGYrqwCy5qWj7ts6BGgQxgZGxyYhPXhpggB46
Artifact Size: 4400 bytes
```

Historical devnet run (superseded by the structured evidence format above):

```text
Account: EvrdGfn3vYrFm8s3SggCCn5MYkViPT4utrrXEDAWSdat
Signature: 4v72Kp9ewbxFUkgEKDL16FcApuW87HTy9X2HkdFp4FjXZe4Pq3iyNuBXiPcmzEUiaUxBgQ49ttJPFR18WZLhNjgT
```

## Instructions

`initialize` uses instruction data `[0]` and accounts `[vault(w), authority(s)]`. The vault account must already exist, be program-owned, and be exactly 41 bytes.

`deposit` uses instruction data `[1][amount:u64-le]` and accounts `[vault(w), authority(s)]`. The authority signer must match the pubkey stored at byte offset `1`.
