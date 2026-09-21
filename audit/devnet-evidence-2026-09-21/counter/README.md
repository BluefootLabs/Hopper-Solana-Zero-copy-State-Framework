# Devnet evidence, 2026-09-21: the framework-comparison counter, round three

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is the
macro-path counter fixture from `bench/framework-comparison/programs/counter/hopper`,
built by `scripts/bench-framework-comparison.py` from a clean tree at commit
`a91462c002fd4d6a57a6755a1f6a7f8c61887f4d` (local ELF SHA-256
`01c795552b585aa50b326c4e2510c4027a286520f37fd20f62cc9ac085c6b08b`, 9,960
bytes) and deployed fresh as `F4Um7PWsnZfN7y8WFzu1aPYJwqGduJTa4zuCGY9EUqMy`.
The on-chain dump hashed to the same value before the first transaction and
after the last one (`onchain-dump.sha256`, `onchain-dump-after.sha256`).

This lane exists because the commit changed three things on the `init` path
that no example lane exercises: an `init` PDA with a supplied bump is proven
by the creation CPI instead of a sha256 at bind, `CreateAccountAllowPrefund`
keeps the payer in the instruction with a zero delta when the account is
already funded, and the rent product no longer goes through `__multi3`. Each
transaction was sent with `hopper tx send` (logs are the CLI's own output,
unedited) and re-fetched at `finalized`; the compute units below are the
runtime's, and they equal the Mollusk numbers in
`bench/framework-comparison/results/RESULTS.md` to the unit.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| deploy | `3Ffw7cvmLx7HESUFarRr3RCtkcpzZp2vak27WfaqTscdRNH71vvz2D8CuEou7rxZaDsqDSGwD1BEYAWWC18ZB886` | 502,112,265 | program `F4Um7PWs…`, upgradeable, authority = payer | |
| 1. `initialize` with another authority's PDA as the counter (unsigned, wrong address) | `2F6GDXQmdwE6sPhTGXpmAAFQJ4K99WqxqMpXnFG6DPWKvMe2te153e7GsFR8uFyfxoPEA5afpVD4xYtKwp5iUMvi` | 502,112,352 | refused at the creation CPI: `PrivilegeEscalation` (the runtime's signer check is the PDA check) | 1,347 |
| 2. `initialize` with a signing keypair at a non-PDA address as the counter | `4UvJXVJApAQp47Ztr8XMaDDeSPU9ZPhgnjiip7fVQ9Zcozpx9w3AKk3owSJcAANF8dASZCxNezyBpyukyuknBBZT` | 502,112,368 | refused at bind by the out-of-line hash: `InvalidSeeds` | 305 |
| 3. `initialize` A (PDA `Cn3JBYNB…`, bump 252, zero lamports before) | `4HfBzkCrT2UncCgvky81qruXHAJHjMQAwhHizPjdou3M8zPUPmQtpCQ7YZiJVZa9zWRZviYqTgPaUKtmGuYxSxcc` | 502,112,383 | 25-byte account, discriminator 1, bump 252 at offset 16, count 0; 777,240 lamports, the live rent minimum for 25 bytes | 1,572 |
| 4. `increment` A | `3hicMGiSKHkYFrryJZJBhU3Zz7ccL4FsR6jhj53pdHQMvGK9Y9Y8RTrYMQHqzSLEQjbWRZadQcPcPvJVQ2HGT91p` | 502,112,390 | count 1 | 368 |
| 5. `initialize` A again | `5TmEtPxzY2cUNMtbNFSwxk9aEvMn4Az6ev6AAoHcNoZbNZKKPY6ERd7nJ3pPDn9Pd7H4f6pq3wXTcgk1TFpsskUB` | 502,112,398 | the account holds data, so the hash ran and passed, then `init` refused: `AccountAlreadyInitialized` | 499 |
| 6. `initialize` C (authority `7Qj28pSp…`, PDA `6vh34eBG…` pre-funded with 2,000,000 lamports) | `4SuSwBSmnLyqRpXFaYfR8GWySyWXf2KzdFAeTKiXRNRfYziWbQ2hxDMRthtYNsmHa9PWdmT4LzNw7RpySZdpdxjy` | 502,112,499 | zero-delta shape: the payer stayed in the instruction and lost only the 5,000-lamport fee (10,000,000 to 9,995,000); the PDA kept its 2,000,000 and got the 25-byte layout | 1,570 |
| 7. `increment` C | `2gAkNoYDmwQiLvLXJHLDU28e26mHRoF67fsaL5YA12rN6sdZRSUSQgG1rs7XcxzjL71tgXc28NNAZoX2gdkJDQoB` | 502,112,508 | count 1 | 368 |

Account bytes after each step are in the `*-account-*.txt` and `*-pda-*.txt`
files (length, hex, lamports). Steps 1, 2, and 5 were sent with
`--allow-failure`, so each refusal is a finalized ledger entry, not a
preflight rejection. The two funding transfers for authority C are
`6a-fund-c.json` and `6b-prefund-pda-c.json`. `finalized-slots.txt` is the
`getTransaction` re-fetch (slot, error, compute units) for every signature.

The program is left deployed and upgradeable so the lane can be re-run. No
keypair other than the payer's public key appears in this directory.
