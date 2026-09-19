# Hopper Escrow

This example covers the state and close-account semantics of an escrow-shaped
lifecycle. It does not perform SPL Token custody or token-transfer CPIs, so it
must not be used as evidence of a token escrow implementation.

## What It Demonstrates

- zero-copy escrow state via `#[account]`
- typed contexts via `#[derive(Accounts)]`
- `Ctx<T>` handlers with `ctx.accounts.*` business methods
- Hopper account creation and typed state writes
- authority-gated state transitions and close-account lamport recovery
- `UncheckedAccount` for raw accounts that are intentionally not decoded

## Instruction Map

- `0` = `Make`: initialize the state and record maker and offer metadata.
- `1` = `Take`: enforce the stored maker link and close the state account.
- `2` = `Cancel`: enforce the maker signer link and close the state account.

The mint and amount fields are state metadata in this example. None of these
instructions validates token accounts, moves tokens, or performs an SPL CPI.

## Devnet

Historical deployment record (not a fresh finalized attestation):

- Program id: `5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN`
- `.so` size: 18 736 bytes

```bash
hopper build -p hopper-escrow
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_escrow-keypair.json
```

After deploying the final source to a fresh program id, run the state-lifecycle
integration test below. It is gated so the default `cargo test` stays offline:

```bash
HOPPER_DEVNET=1 \
HOPPER_ESCROW_PROGRAM_ID=REPLACE_WITH_FRESH_PROGRAM_ID \
HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
cargo test -p hopper-escrow --test devnet -- --nocapture
```

When `HOPPER_DEVNET` is set it must be exactly `1`; any other value fails
instead of silently skipping. The test verifies the devnet genesis hash and
deployed program account, then polls every transaction and state read at
finalized commitment. It asserts every initialized field exactly, submits a
wrong-maker cancel and proves the complete account snapshot is unchanged, then
closes with the correct maker and polls until the account is absent.

A passing run emits one line prefixed with
`HOPPER_DEVNET_EVIDENCE_JSON=`. The deterministic JSON shape binds the cluster
genesis and node version, program id, public account ids, finalized signatures
and slots, initialized state values, pre/post account SHA-256 values, and the
close result. It contains no keypair path or RPC URL; custom RPC endpoints are
reported as `redacted`.

`account_sha256` hashes this canonical byte sequence: lamports as little-endian
u64, owner pubkey, executable as one byte, rent epoch as little-endian u64,
data length as little-endian u64, then the complete account data.

## Verify

```bash
cargo check -p hopper-escrow
hopper build --host -p hopper-escrow
hopper build -p hopper-escrow
```

## Manifest Path

This example ships a checked-in `hopper.manifest.json` describing the
`Escrow` layout and the make/take/cancel instructions. `hopper explain`
uses it to decode a real devnet `make` transaction without requiring an
on-chain Hopper manifest:

```bash
hopper explain <make-tx-signature> \
  --manifest examples/hopper-escrow/hopper.manifest.json
```

Canonical local generation path:

1. generate and review the checked-in manifest from program source
2. drive `hopper manager` and `hopper client gen` from that local artifact
3. if application-specific tooling provisions the legacy `MANIFEST_SEED` PDA,
   `hopper fetch <program-id>` can read it; Hopper ships no generic publisher

## CLI Walkthrough

```bash
hopper build --host -p hopper-escrow
hopper test -p hopper-escrow
hopper build -p hopper-escrow
hopper profile bench
```
