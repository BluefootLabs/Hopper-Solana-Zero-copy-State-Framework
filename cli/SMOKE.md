# Hopper CLI smoke runbook

A copy-paste sequence that exercises the CLI end to end: build → host test →
SBF build → deploy → decode. Run it after any change to `tools/hopper-cli` or
the lifecycle commands. Steps that touch a cluster are clearly marked; the
host-only steps need no network and no keypair.

The `hopper` binary used below is the workspace CLI:

```bash
cargo build -p hopper-cli
HOPPER=target/debug/hopper   # or `cargo run -p hopper-cli --`
```

## 1. Help surfaces resolve

Every command prints usage without touching the network:

```bash
$HOPPER help
$HOPPER build --help
$HOPPER deploy --help
$HOPPER upgrade --help
$HOPPER close --help
$HOPPER migrate --help
$HOPPER explain --help
```

Expected: each prints its usage block and exits 0.

## 2. Host build + unit tests (no network)

```bash
$HOPPER build --host -p hopper-counter
$HOPPER test -p hopper-counter
```

Expected: clean `cargo build` / `cargo test` for the package.

## 3. SBF build (no network)

```bash
$HOPPER build -p hopper-counter
ls -l target/deploy/hopper_counter.so
```

Expected: a `target/deploy/hopper_counter.so` artifact and a printed size delta.
The counter `.so` is ~4.7 KiB.

## 4. Mainnet guard (no network, must refuse)

The CLI must refuse to target mainnet from default config:

```bash
# Should error: "refusing to target mainnet from default config"
$HOPPER deploy --url https://api.mainnet-beta.solana.com/ -p hopper-counter
```

Expected: non-zero exit with the mainnet-guard message. (It never reaches a
deploy.) Naming `--cluster mainnet-beta` explicitly is the only way past the
guard, and even then a confirmation prompt fires unless `--yes`.

## 5. Devnet deploy (network + keypair)

> Requires a funded devnet keypair. Devnet only — never mainnet.

```bash
$HOPPER deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_counter-keypair.json \
  -p hopper-counter

solana program show <PROGRAM_ID> --url devnet
```

Expected: a deploy signature, then `solana program show` reporting the program
as an upgradeable program with the expected data length.

## 6. Decode a real transaction (network)

```bash
$HOPPER explain <CONFIRMED_SIG> \
  --manifest examples/hopper-escrow/hopper.manifest.json
```

Expected: one decoded line per instruction — program id, disc byte, matched
instruction name, touched account slots — and the CU consumed.

## 7. Migration plan + bytecode upgrade (network + authority keypair)

```bash
$HOPPER plan -p hopper-migration                       # read-only layout plan
$HOPPER migrate --program-id <PROGRAM_ID> \
  --cluster devnet --keypair /abs/path/devnet-keypair.json \
  -p hopper-migration
```

Expected: `plan` prints the field-level diff; `migrate` prints the migration
banner and an upgrade signature.

## Devnet reference ids (this pass)

| Example | Program id | `.so` bytes |
|---|---|---:|
| counter | `D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F` | 4 688 |
| escrow | `5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN` | 18 736 |
| versioned-state | `EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt` | 25 664 |
| orderbook | `CK3XYYsbFducx9UEEWWLGAVnSAhGkMtM1TKLe8PDP6dJ` | 18 408 |
