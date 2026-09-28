# Devnet evidence, 2026-09-27: the framework-comparison counter, round six

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is the
macro-path counter fixture from `bench/framework-comparison/programs/counter/hopper`,
built by `scripts/bench-framework-comparison.py` in a clean worktree at commit
`d527a3a86682cbed68899802228287695e975c9a` (local ELF SHA-256
`a6eb234b94fcd867d35ac689a2eb7fb764a68536697a434cc1b3b64be604b3fc`, 8,696
bytes) and deployed fresh as `6yNZne9rJuXVKDwV1wFQv1zvK7msMgvFmPNM42Juu54e`.
The on-chain dump hashed to the same value before the first transaction and
after the last one (`onchain-dump.sha256`, `onchain-dump-after.sha256`).

This lane adds one step to round five: the commit makes `#[derive(Accounts)]`
refuse one account passed in two undeclared mutable roles, and step 9 passes
the payer as both the `authority` (a mutable signer) and the `counter` (the
account to create). The loader serializes that account once and marks the
second slot as its duplicate; bind sees the shared record and refuses before
any signer, PDA, or rent check runs. Each transaction was sent with
`hopper tx send` (logs are the CLI's own output, unedited) and re-fetched at
`finalized`; the compute units are the runtime's and equal the Mollusk rows
in `bench/framework-comparison/results/RESULTS.txt` for this commit to the
unit.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| deploy | `4MJJ1R3pGXTryYG7fKZrU7YWe71sHG9WhDxaCUJo2yttwbSvB9R9RpU41maFqMcYEnBogK2QzHwRXhpBHvkdDMuk` | 505,032,516 | program `6yNZne9r…`, upgradeable, authority = payer | |
| 1. `initialize` with another authority's PDA as the counter (unsigned, wrong address) | `66ut9YsdtkAyTeW41ZFHiPozwmgc5cqecC7uZnAJ8783W4345mW3GipwppNo43ACwNV3GoEuJ9PaUzh96khVdksE` | 505,032,571 | refused at the creation CPI: `PrivilegeEscalation` | 1,338 |
| 2. `initialize` with a signing keypair at a non-PDA address as the counter | `57YDdf7osc94pdaJXkfNe8Eev7Jk2c3PJwL5BwXzRYuqcTVjdsnSzkPqsVREh9K9j9woXTVuk6KFLQVgF47Hr3ew` | 505,032,582 | refused at bind by the out-of-line hash: `InvalidSeeds` | 314 |
| 3. `initialize` A (PDA `HbMGBx7t…`, bump 253, zero lamports before) | `2Hpzd3cUF732LKihUeyt9EVV9WKhBeUACmXiv9J4WozZL5ohmPc1dP9j5a8deV7fKBLWb59Y827RMEVtRJ8o9qt1` | 505,032,593 | 25-byte account, discriminator 1, bump 253 at offset 16, count 0; 777,240 lamports | 1,544 |
| 4. `increment` A | `62UjFtQyviVXYkrWN2rRYWEf2meLhUtZsSBZgd6oaLjUCMoz7jiVZykWh6spXMXrGeGRkHtxsvoyjMNy3ZU9X2X6` | 505,032,601 | count 1 | 348 |
| 5. `initialize` A again | `2NnJAS23MEEuwCdjQksQttsDJ5w8uUdpVADefrCya9GWg6cBvGu96zdDsZwXE2Mw2LgH5j7su4NeUvdjK1rLweaZ` | 505,032,607 | the account holds data, so the hash ran and passed, then `init` refused: `AccountAlreadyInitialized` | 505 |
| 6. `initialize` C (authority `8zCv3Df6…`, PDA `J8jwDMx5…` pre-funded with 2,000,000 lamports) | `3AKCs6VCsEfsj2QYVERCEg3N9X9SQWH83qzAMj4cJQBrgyEYFyqMLjcPrZZBvZK8Fcrsdd4uhTTUJfp9NfX1SsZM` | 505,032,680 | zero-delta shape: the payer lost only the 5,000-lamport fee; the PDA kept its 2,000,000 and got the 25-byte layout | 1,542 |
| 7. `increment` C | `2NhWuTL7jHsj3JKy9ta21YQmXCDkEJ1tc9VscgsneodPgSvfVVEbHndigtkDkDpPe9MDsDtu5Kx5efec2Nhqqcg8` | 505,032,689 | count 1 | 348 |
| 8. `increment` A with a third, surplus account | `2WpmhsmNBqMPuy3Xvh7nYyD7hG38SpU71a5yMpYpAknsrBEvupEmymfSEMDgDnbmyz3G1Z5EGp9vZHziCppMVGCk` | 505,032,742 | refused before any account was walked: `Custom(45057)`, `ERR_TOO_MANY_ACCOUNTS`; the count stayed 1 | 14 |
| 9. `initialize` with the payer as both `authority` and `counter` | `sYnFRmanBiCZupvVVmRLJ16SCtvBK1ZLYvnadWv33YDibvCx8yocjcJ83NCmpm99MRroJmrSTw5azgtLVPR4aYj` | 505,032,754 | refused at bind: `Custom(45058)`, `ERR_ALIASED_MUTABLE_ACCOUNTS`; the payer lost only the fee | 99 |

Account bytes after each step are in the `*-account-*.txt` and `*-pda-*.txt`
files (length, hex, lamports). Steps 1, 2, 5, 8, and 9 were sent with
`--allow-failure`, so each refusal is a finalized ledger entry. The two funding
transfers for authority C are `6a-fund-c.json` and `6b-prefund-pda-c.json`.
`finalized-slots.txt` is the `getTransaction` re-fetch (slot, error, compute
units) for every signature.

The program is left deployed and upgradeable so the lane can be re-run. No
keypair other than the payer's public key appears in this directory.
