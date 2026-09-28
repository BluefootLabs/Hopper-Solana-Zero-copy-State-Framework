# Devnet evidence, 2026-09-28: the framework-comparison counter, round nine

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is the
macro-path counter fixture from `bench/framework-comparison/programs/counter/hopper`,
built by `scripts/bench-framework-comparison.py` in a clean worktree at commit
`2f25a78` (local ELF SHA-256
`ef2116260aa955773d7e37502b76e0c9979c5fb3fa25cb762a07f794754dd9bc`, 8,352
bytes) and deployed fresh as `Ax5Ntu8G7wrJ5D1tVK5qRnCdengWBhSCzL5h9hVjRqHV`
at slot 505,186,917. The on-chain dump hashed to the same value before the
first transaction and after the last one (`onchain-dump.sha256`,
`onchain-dump-after.sha256`).

What this round proves: at this commit a seeded `init` stores the bump it
signed the creation with in the layout's `#[bump]` field itself, so the
fixture's `initialize` no longer writes the bump or the count. The account
bytes after step 3 show the bump (`ff`, bump 255) at offset 16 written by
`hopper_init!`, and step 4's `increment` verifies the PDA from that stored
byte (`bump = stored`, one sha256) and succeeds. `initialize` costs 1,517 CU
on chain, 27 fewer than round six, and the ELF is 344 bytes smaller; both
equal the Mollusk rows in `bench/framework-comparison/results/RESULTS.txt`
for this commit to the unit. Each transaction was sent with `hopper tx send`
(logs are the CLI's own output, unedited) and re-fetched at `finalized`.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| deploy | `4mnmub5WcQyo1ypuRznr5tko7JWFQ1doncjTswQMH2jANKiYkhiCnbSgQySzFWwzPtZYii65gPdgn1acuMGPhQbQ` | 505,186,917 | program `Ax5Ntu8G…`, upgradeable, authority = payer | |
| 1. `initialize` with another authority's PDA as the counter (unsigned, wrong address) | `3bBcMr6Y3JE1tB7iFfFp6WnvvbHnuDGUuw68qm7v7y1datadtJ2gPiXppxW5Wrsjj2TSKCm7WGXFBCgM8DXD9s4P` | 505,186,933 | refused at the creation CPI: `PrivilegeEscalation` | 1,332 |
| 2. `initialize` with a signing keypair at a non-PDA address as the counter | `2aWq9bUvpXh4c5TgRDB3wgcvSPdsJks31BqUUCo7RSPzURkVUXsHYwsWwBBb61jjsZge7YvidUeyiYXtciBbjYNy` | 505,186,945 | refused at bind by the out-of-line hash: `InvalidSeeds` | 314 |
| 3. `initialize` A (PDA `79cJsZ1E…`, bump 255, zero lamports before) | `4Nj3Rf6YQCS5ZraSfqzCKFxceMfH8Rpe8xqcD7V1koamPVFHZqqvTFaRfFsquHVhvccUTFUnyirv16H8aVoVn7AT` | 505,186,961 | 25-byte account, discriminator 1, bump 255 at offset 16 written by `init`, count 0; 777,240 lamports | 1,517 |
| 4. `increment` A (`bump = stored` reads the byte `init` wrote) | `3TeKmfpQyax1wfYVzHeLU5x5GPPLEydKj2cXbpDXssmdPQq9iHtHaYystycSDUiJp5rga35phVRqzVvkJ96JTkn8` | 505,186,967 | count 1 | 348 |
| 5. `initialize` A again | `PbsgS5WBcz7rLhKKS6FgjYqy4gDk7SbKA8i9yadXZP7NkB7bvRFNLyhzT3p62Dfd7LZwGEHFWxaPZYZtgtmkJgv` | 505,186,973 | the account holds data, so the hash ran and passed, then `init` refused: `AccountAlreadyInitialized` | 505 |
| 6. `initialize` C (bump 254, PDA pre-funded with 2,000,000 lamports) | `5PsSs5rhHXvBJhsJuhD5CQp2BtrVTFE69cdsmJtWFcZkEXPDxhtYXSCabv4ohFeEeDRLg2zkhjNZ63yRgS7WPieW` | 505,187,041 | zero-delta shape: the payer lost only the 5,000-lamport fee; the PDA kept its 2,000,000 and got the 25-byte layout with bump 254 at offset 16 | 1,515 |
| 7. `increment` C | `WhVCwCJNAbJSiu6TK4Fzc9ZCrWZRLFXQtdrHhhe1ZqFGaXfWwqHfnJofjwWmYeMDFwUCdZgJfRJBQq12sHrkatD` | 505,187,048 | count 1 | 348 |
| 8. `increment` A with a third, surplus account | `5MrJVbsxhk5HvfbS9Zf86NFRebvfxgMXQBd5bKRQxcM1iTAeRe1vdxvAELjDjx6LgutntAd92m2WsZFP6vopdAjp` | 505,187,057 | refused before any account was walked: `Custom(45057)`, `ERR_TOO_MANY_ACCOUNTS`; the count stayed 1 | 14 |
| 9. `initialize` with the payer as both `authority` and `counter` | `4Sn4KRsNpCVEA7DwAaCW8E76EN68xRXU8HUSTKdfV4FEiAWjL87K8yb5Q5aiop64estwJM7tp73DfnvubV6FhfBr` | 505,187,070 | refused at bind: `Custom(45058)`, `ERR_ALIASED_MUTABLE_ACCOUNTS`; the payer lost only the fee | 99 |

Account bytes after each step are in the `*-account-*.txt` and `*-pda-*.txt`
files (length, hex, lamports). Steps 1, 2, 5, 8, and 9 were sent with
`--allow-failure`, so each refusal is a finalized ledger entry. `SHA256SUMS`
covers every file here except itself and `BUNDLE.SHA256`, which is its digest.
