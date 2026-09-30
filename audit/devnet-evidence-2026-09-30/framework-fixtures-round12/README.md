# Devnet evidence, 2026-09-30: the framework-comparison fixtures, round twelve

Public devnet (`https://api.devnet.solana.com`), signer `4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and upgrade
authority). The programs are the four Hopper fixtures
`scripts/bench-framework-comparison.py` measures, built with pina's release
recipe at commit `f6333ae` (the ELFs whose SHA-256s
`bench/framework-comparison/results/results.json` records). Each was deployed
fresh, and its on-chain dump equalled the local ELF before the first
transaction and after the last one (`before-*-onchain.so`, `after-*-onchain.so`).

- Hello world, raw layer: `652cg6QATsysXdWhSwmcJVkZv9okE4cghWY1piENmwcM`, 1,456 bytes, SHA-256
  `6cbf1135ae48dba3cc6928b6def539faaa6518c4c956ddcbc91d824a85389b24`, deployed at slot 505,862,062.
- Hello world, `#[program]`: `76wjzJUCvfV1jS1ndpQdtBc7xzXXBr57FckZrEN2DvmQ`, 1,824 bytes, SHA-256
  `a3006aa0b07a373be9ba5132bb61a2844a98609958f6483b706783927daecd8b`, deployed at slot 505,862,139.
- PDA counter, raw layer: `E4mKDytrmQrhFh5rsepKhuCQxosgRD216fq3VEHZmQNQ`, 6,608 bytes, SHA-256
  `cc1b5cc94d746cc8f76e5438ca0ef74f42817dc0e27f7ba7ec18272d2c1156a1`, deployed at slot 505,862,211.
- PDA counter, `#[program]`: `CQzbQVdPqL1yi51CRUiooAtkS2nEFwzwKHMzBwk8SmCA`, 8,744 bytes, SHA-256
  `8aaca49d5ea5777690efc7ddbfeeb061658d4d776fadee653ae7d77fa7771ae1`, deployed at slot 505,862,275.

Commit `f6333ae` changed the account walk every entrypoint runs, the CPI
tier the System builders use, and the count-exact entrypoint of
`#[program(profile = "tiny")]`. This run puts all three on a live
validator: hello once each, and for each counter one create and two
increments, with the account's size, owner, discriminator, bump, and count
checked after every step.

| Step | Devnet CU | Mollusk CU (results.json) |
|---|---:|---:|
| `hello-raw` | 111 | 111 |
| `hello-framework` | 127 | 127 |
| `counter-raw-create` | 1,514 | 1,514 |
| `counter-raw-increment-1` | 1,722 | 1,722 |
| `counter-raw-increment-2` | 1,722 | 1,722 |
| `counter-framework-create` | 1,471 | 1,471 |
| `counter-framework-increment-1` | 325 | 325 |
| `counter-framework-increment-2` | 325 | 325 |

What the run checked (from `receipt.json`):

- before the run: every deployed program equals its local ELF byte for byte
- hello-raw: logged `Hello, Solana!`
- hello-framework: logged `Hello, Solana!`
- counter-raw create: 10 bytes, discriminator 1, bump 254, count 0
- counter-raw increment 1: 10 bytes, discriminator 1, bump 254, count 1
- counter-raw increment 2: 10 bytes, discriminator 1, bump 254, count 2
- counter-framework create: 25 bytes, discriminator 1, bump 254, count 0
- counter-framework increment 1: 25 bytes, discriminator 1, bump 254, count 1
- counter-framework increment 2: 25 bytes, discriminator 1, bump 254, count 2
- after the run: every deployed program equals its local ELF byte for byte

8 transactions, re-fetched at `finalized`.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| `hello-raw` | `52M1dbH99NTrvaUcKtGjizvnvbuiyo5pXYHzJbm4osNCy6HQBPXioFBqYAs2vGE8gJkucA3vBxWCggADiNRURfiu` | 505,862,284 | ok | 111 |
| `hello-framework` | `35kyu1BKwG89RJHRENJMADnvZG6vCLRp1AY5qHLq5tgNqP2JwL2ysuJLkN6UvzTsy8fwPwuBHo3aycAV5jWoiSxn` | 505,862,292 | ok | 127 |
| `counter-raw-create` | `5NgGiSNCnepUD1r4j4tiKstspbSeGidDsdcsC3PipHQdjt7o1Hp1pT2GrFL8GdUhe32Q1ZFyMMgkq5NRoFyX4wGs` | 505,862,341 | ok | 1,514 |
| `counter-raw-increment-1` | `61yHdYNXVR4naHNCQQVbtd8EbTanuUjKgu99HsCdg2xZYvngNcDajG5amxNt8CbUakxh1C89uY3AdGaqbN3e7HTZ` | 505,862,350 | ok | 1,722 |
| `counter-raw-increment-2` | `CGUJzYNUrcPXYwgfUM5wzSKu3q4TqzvTnHf3SXKZSFZUQTT8CudtXcQKSP8ktyia6f5weUzNLsYSceEDd4ZxX6r` | 505,862,362 | ok | 1,722 |
| `counter-framework-create` | `5ndKyFZdz79b3Po1ZuuKZJ88A6tDC85khnsfTBcw8UNCRWk4nv8gr8kK6KYu3df7oapKnrjCZAbxxJ337K5zDm7K` | 505,862,368 | ok | 1,471 |
| `counter-framework-increment-1` | `5UMZ7S81vSai6EmQbd4VowZKMrs8j4RTawPBe92HVZecfQH9BSgF7b6gGw7HFgj95cfLtrkSze9AzjK5B1moBP5T` | 505,862,375 | ok | 325 |
| `counter-framework-increment-2` | `65xyYRkXY6ospL8FZcvuvbbnSRUeQeRvypYTyrgJnVanz7S1R6huxuJPx3ReEhrZ3BEi83aiP9EpoRKvC2rJgAYo` | 505,862,424 | ok | 325 |

`receipt.json` is the runner's own record; `SHA256SUMS` lists every file here.
