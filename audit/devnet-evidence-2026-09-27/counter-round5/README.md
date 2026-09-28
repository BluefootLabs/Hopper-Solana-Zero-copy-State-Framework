# Devnet evidence, 2026-09-27: the framework-comparison counter, round five

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is the
macro-path counter fixture from `bench/framework-comparison/programs/counter/hopper`,
built by `scripts/bench-framework-comparison.py` in a clean worktree at commit
`c5317a6b56be273b992f026f589025c07125fb4d` (local ELF SHA-256
`209558bb0015e9273088609dc251927e349deb146ac8c663104a1f6f54b8cad6`, 8,584
bytes) and deployed fresh as `6LjqFgiBazYXDYadHeeXq8ff85MiaPDLiDd4J62aexCq`.
The on-chain dump hashed to the same value before the first transaction and
after the last one (`onchain-dump.sha256`, `onchain-dump-after.sha256`).

This lane re-proves the round-three paths on the commit that moved every
sysvar read to `sol_get_sysvar`, added the per-invocation rent cache, and
made the count-exact entrypoint refuse accounts past the matched bound. Each
transaction was sent with `hopper tx send` (logs are the CLI's own output,
unedited) and re-fetched at `finalized`; the compute units are the runtime's
and equal the Mollusk numbers in
`bench/framework-comparison/results/RESULTS.txt` for this commit to the unit.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| deploy | `5DdjHkqMUD3he4Tyh3nzK8BBoPu42EKz8yBCH3VdXFghpgumsGPKZYdU1mWSSoYC8J3vUw1aRhgfPTrKLS1qk36k` | 505,024,761 | program `6LjqFgiB…`, upgradeable, authority = payer | |
| 1. `initialize` with another authority's PDA as the counter (unsigned, wrong address) | `A1XFN95oUeT1mgZbCNuxZE3Lw8PKFz9Du7hPWyPTAhepGDtE1XaSqopNdG18Sd98C8cS4GQvJhFPBHu4gM3zK8b` | 505,024,790 | refused at the creation CPI: `PrivilegeEscalation` | 1,337 |
| 2. `initialize` with a signing keypair at a non-PDA address as the counter | `3dDdHaRJNgVmWKvVRDRBRj27dobsqtBUx2TZgcmGUbyzWtdr7nPQEStmntAKhHvUpJxD22bCD6YXR4SVcW16cRU3` | 505,024,802 | refused at bind by the out-of-line hash: `InvalidSeeds` | 312 |
| 3. `initialize` A (PDA `3Lp4oCGN…`, bump 255, zero lamports before) | `K72ScggDHeww3K1pbWeGSYsz9qQbkPAejSPKAAMyDQSyskoctVNsb69cQe2K8obiaUywi78wuVmEfmN8SxP7SK4` | 505,024,813 | 25-byte account, discriminator 1, bump 255 at offset 16, count 0; 777,240 lamports, the live rent minimum read through `sol_get_sysvar` | 1,542 |
| 4. `increment` A | `4VzzrZ7wbR4BT4HpFbQymCuDZiBdw83zP3jRKtdKaxvkWAAwgXmqy4SMz9do3X5sFx92KiatFXcHfJLhqne84q5U` | 505,024,819 | count 1 | 348 |
| 5. `initialize` A again | `TYfU6LCNwBUXVREUP2fLt6E8qJ1brdisgTxw6u6Pqm4p7fsvc6jmabrahfNFmXLGj3ghWGiZKWfGHsXB2pRoFpW` | 505,024,826 | the account holds data, so the hash ran and passed, then `init` refused: `AccountAlreadyInitialized` | 502 |
| 6. `initialize` C (authority `HYaGa93L…`, PDA `BXkW9N15…` pre-funded with 2,000,000 lamports) | `2QE8eFvytC9U2TZjo299krxfNJPEgvuPX4pWPCmFxy6yYA4ZxzQPDGSytTHtq45aqPxKi5Woodzk1FV1Gn3YDWYQ` | 505,024,896 | zero-delta shape: the payer lost only the 5,000-lamport fee; the PDA kept its 2,000,000 and got the 25-byte layout | 1,540 |
| 7. `increment` C | `2aaiuyPg9124J2QDhH1kCpkmgc2DZvmmnpcJb6i935GbftP4SqXWeb8x6itCLaVQRGRAYQeiXrbS5zmmvJj3FwAK` | 505,024,905 | count 1 | 348 |
| 8. `increment` A with a third, surplus account | `5jKkshDnYjKvKFi2eWBmFQnMfjRovgy4JEa2SJnVhEeczrGQeMKH5aQwFDrPju8WjVYqESevz26fqYSGZLeH9ttq` | 505,024,913 | refused before any account was walked: `Custom(45057)`, `ERR_TOO_MANY_ACCOUNTS`; the count stayed 1 | 14 |

Account bytes after each step are in the `*-account-*.txt` and `*-pda-*.txt`
files (length, hex, lamports). Steps 1, 2, 5, and 8 were sent with
`--allow-failure`, so each refusal is a finalized ledger entry. The two funding
transfers for authority C are `6a-fund-c.json` and `6b-prefund-pda-c.json`.
`finalized-slots.txt` is the `getTransaction` re-fetch (slot, error, compute
units) for every signature.

The program is left deployed and upgradeable so the lane can be re-run. No
keypair other than the payer's public key appears in this directory.
