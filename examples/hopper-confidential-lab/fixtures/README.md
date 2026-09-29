# Fixtures

`token_2022_v11.0.0.so` is the Token-2022 program mainnet-beta runs, dumped
for the confidential-transfer tests:

| | |
|---|---|
| Program | `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` |
| ProgramData | `DoU57AYuPFu2QU514RktNPG22QhApEjnKxnBcu4BHDTY` |
| Last deployed in slot | 427,147,035 |
| Dumped | 2026-09-29, mainnet-beta slot 451,763,132 |
| `security.txt` source release | `program@v11.0.0` |
| Size | 1,382,016 bytes |
| SHA-256 | `0999dbf708971e723b08d1caafc988826a59c6001ed6dc02260da07defbe1469` |

Reproduce with:

```text
solana program dump TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb token_2022.so --url https://api.mainnet-beta.solana.com
sha256sum token_2022.so
```

The file is byte for byte what the cluster executes, so a test that passes
against it says what mainnet's Token-2022 would do. The test checks the hash
before it loads the file. Devnet ran `program@v11.1.0` on the same day
(deployed in slot 503,153,936); `CONFIDENTIAL_LAB_TOKEN_2022=<path>` runs
the tests against any other dump.

Token-2022 is Apache-2.0 licensed, from
[solana-program/token-2022](https://github.com/solana-program/token-2022).
