# Devnet evidence, 2026-09-21: metadata records and transaction v1

Public devnet (`https://api.devnet.solana.com`, Agave 4.3.0-rc.0), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of the sentinel deployment from the 2026-09-19 lanes),
program `7N2pyj1zhn6HSt6A553xJaM5CLJdcLjZXvw9KtSQNmFx`. Every signature below
was re-fetched at `finalized` with `maxSupportedTransactionVersion: 1`; the
logs are the CLI's own output, unedited.

| Step | Signature | Slot | Envelope | Result |
| --- | --- | --- | --- | --- |
| `hopper publish-security` (inline Initialize, 363 stored bytes) | `5hrvXz83dFESSFn7WYy19a8qfhty6ovq8NCtpHE1xDVWHedyF8F8ccmTAecnQHCnaLvk7CS6AHD8vUevccoADfAy` | 502,011,159 | legacy | record at `FCnkgbtAC4zE3h7J6aWz12LcTN5QDhRk9erbnXVkHefZ`, seed `security`, canonical |
| `hopper publish-manifest` Allocate | `3wu9wYbmBfvKM7bHxmZn9mYfNdmVyWeVwTQchc2PCcTc52AVT2jYLJFqBHdtn2CyGD34bptaQDUDN9w5cxiNzhi9` | 502,011,247 | legacy | buffer at `595E2qx9zTBYbHMY6vuVF7sS3YKjiuD9TzgCAYAFeGqi` |
| `hopper publish-manifest` Write 1/2 (offset 0, 900 bytes) | `4FXrQiRCtVan1YZo4YBWAJfsQibvn47gGAbvhp5yrNMgSp7DD2n1sukAMUm86aoXdBhixj6R9z5aneohpRc6hgok` | 502,011,252 | legacy | |
| `hopper publish-manifest` Write 2/2 (offset 900, 854 bytes) | `54TJfDktECMRpTYbDsGopq4cwi3G3FMCoLLJhGBQdaHVDP3dRSegxYyvz1mYfbEMxzssnZhkm1s9NJSW1iZXodfK` | 502,011,256 | legacy | |
| `hopper publish-manifest` Initialize from buffer | `46hnQG3CwjfMKLpTNrQ46zg8YTrYZ1JbcgFucWVS2wFYVhA7Wkgn8mCjzus7b7Qwp55FNeW3AH1PCMH1GzKFgq2M` | 502,011,260 | legacy | record at `595E2qx9…`, seed `hopper-manifest`, 1,754 stored bytes inflating to 13,369 |
| `hopper tx send --v1` (SPL Memo, one signer) | `2oqSeV99Ki1ATSSiQyScCZPaWvVNhFHf57ZjEW2QvNh5JheXomgABkmgp4Qwjbd6C3PVuutCKNcZ411jLpBas8Md` | 502,011,279 | **v1** | 220-byte envelope, 28,292 CU, fee 5,000 lamports |
| `hopper publish-security --overwrite` (inline SetData) | `5rVdNAWYV9Fzk8hUGU45Fdc43H2uss9bdhgo9iypA4FGN9SRYieUBGiWcxZsERTaxenfUcCMtUe9hzvVMGL7wRKf` | see log | legacy | record rewritten in place; the `--read` issued immediately afterwards found it (account reads now at `confirmed`) |

Checks made on the read-back:

- `publish-security --read` decoded the 96-byte header (disc 2, canonical,
  mutable, Utf8 + Zlib + Json, direct) and inflated the payload to the
  minified `examples/hopper-sentinel/security.json` document.
- `publish-manifest --read` inflated the record to a JSON document equal,
  as parsed JSON, to `hopper schema export --manifest` of
  `audit/devnet-evidence-2026-09-19/sentinel-authority-gate/sentinel-v1.manifest.json`,
  the normalized rendering the command publishes.
- `hopper tx explain` on the v1 signature reported the memo instruction and
  the measured compute units; `getTransaction` reports `"version": 1`.
- A `--read` issued in the same second as a publish reported no record,
  because the account read used the RPC's `finalized` default while the
  send confirms at `confirmed`; the read now asks for `confirmed`.

No program was deployed or upgraded. Both records are mutable and owned by
the sentinel's upgrade authority; the buffer path left nothing behind.
