# `hopper explain`

Decode on-chain artifacts into human-readable form. `explain` is a family:

- `hopper explain <signature>` (alias for `hopper tx explain`), decode a
  confirmed **transaction**.
- `hopper explain account <hex-data>`: decode supplied **headered account bytes**.
- `hopper explain receipt|compat|policy|layout|program|context|instruction`,
  explain the corresponding manifest artifact.

This page covers the transaction decoder, the headline of the devnet pass.

## `hopper explain <signature>`

```
hopper explain <signature> [--rpc <url>] [--manifest <file>] [--tree] [--raw-logs]
```

| Flag | Meaning |
|---|---|
| `<signature>` | Confirmed transaction signature to fetch and decode. |
| `--rpc <url>` | RPC endpoint (default from config / env). |
| `--manifest <file>` | Local fallback mapping discriminator bytes to instruction names when no application-provisioned legacy `MANIFEST_SEED` PDA is available. |
| `--tree` | Render the CPI call tree with per-frame CU and emitted touch maps. |
| `--raw-logs` | Print the full `Program log:` stream verbatim. |

### What it prints

For every top-level instruction in the transaction, `explain` reports the
target program id, the discriminator byte, the matched Hopper instruction name
(from an application-provisioned legacy manifest PDA, or the `--manifest`
file), and the account slots supplied to the instruction. Unrecognized programs
fall back to a terse line rather than masking the rest of the trace.

### How it talks to RPC

`explain` issues raw JSON-RPC (`getTransaction` with `encoding: jsonParsed` and
`maxSupportedTransactionVersion: 0`) rather than the typed `solana-client`
decoder. Devnet/mainnet periodically add response fields and version shapes that
a pinned SDK struct rejects with an opaque deserialize error; parsing the
`jsonParsed` response directly keeps `explain` working across RPC upgrades and
versioned (v0) deploy/upgrade transactions.

### Example (devnet)

Decoding a real escrow `make` transaction against the checked-in manifest:

```bash
hopper explain <ESCROW_MAKE_SIG> \
  --manifest examples/hopper-escrow/hopper.manifest.json
```

On devnet this decoded the escrow `make` instruction; which self-initializes a
fresh `Escrow` via the `init` lifecycle and writes four typed fields, at
**1 761 CU**.
