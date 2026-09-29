# Devnet evidence, 2026-09-28: the token lab, round ten

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is
`examples/hopper-token-lab`, built with `cargo build-sbf` in a clean worktree at commit
`2ec0930` (local ELF SHA-256
`aa293e467f0b0b89cbc1dfabdd3885c7340b0b97839d783c2224e1beebe09f5f`, 78,136
bytes) and deployed fresh as `4MWp9iQM1qxLrz9sYj4jdf9BU4waEo58MEPs7R4j7m28`
at slot 505,404,217. The on-chain dump equalled the local ELF before the
first transaction and after the last one (`before-onchain.so`,
`after-onchain.so`).

What this round proves: the token-metadata and token-group builders against the
deployed Token-2022, next to the lanes of round eight. A mint is created as its
own metadata account, topped up for the metadata it is about to hold, and
initialized; a key is set, replaced with multi-byte text, and removed; the
token is renamed and the update authority given up, after which an update is
refused. After each step the mint's `TokenMetadata` entry equals the expected
Borsh bytes, and the return data of `Emit` equals the stored entry. A mint is
made a group of at most three members and a second mint a member of it; the
group's size reads 1 afterwards. The live program accepted all three
(`metadata`, `token-group`, `token-group-member` in the receipt's findings).

49 transactions, slots 505,404,225 to 505,405,058, each sent with `hopper tx send` (the logs are
the CLI's own output, unedited) and re-fetched at `finalized`.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| deploy | `5WRuZjUNbuEZ5jNCWGUA6Li7RByQQnor7b6HdftKD37a816nFnkDvfmzt7Zf717hnJXnceQB5cs2kPSva7vERpe` | 505,404,217 | program `4MWp9iQM...`, upgradeable, authority = payer | |
| `create-mint-legacy` | `2GYShbQkKM5W57W4Qh8nX7DZwfn2F93NpaQcPsu4U93rtaMiNdTMoYTPhZ66hSh69dyJEur8Jpzworp4rUnZdRR` | 505,404,225 | ok | 3,839 |
| `immutable-a-legacy` | `63BRRDKHVyjN5v6hp8iuErg8v3BnU2nE7VBS1H1rzS6sTGBMd5S8oYJTvzXZAju3bpbFct3FZ98TjyGGCqEtVnBz` | 505,404,276 | ok | 5,880 |
| `immutable-b-legacy` | `BpJqU8EkRtDnjKZFUVbGoq3op63BdGqEcfnxh4xkUX2DbqfaFMYW68QAx8trBrXonqRg948KSFtmqsZm37e7qfj` | 505,404,296 | ok | 5,880 |
| `mint-to-legacy` | `2WDkcWpTqQymfunB69koCABnePWLwYznsqRE83dQWbrzc6o1gVf3QTYykXB6GKQgqgnBdGqz7xJB6D2V392dPmeY` | 505,404,306 | ok | 1,654 |
| `batch-legacy` | `2abcUBncKTy2VkZHp8MPBjLF7DWv79jCbtNfhZM8Fecq9w7ucsR2ZfNUJAxnMqEScNwJH9WLu7mbgwG34GxsPQ85` | 505,404,317 | ok | 2,471 |
| `ui-amount-legacy` | `4wjsEoqkkdmfx72YCij4JvHVWkVeMAaeFpYxzbWq8E8LKrV2fy5Sc2ApRwME8vyzDf1jA6UTLYCdzmtqsoeUddP5` | 505,404,324 | ok | 4,150 |
| `prefund-legacy` | `RhFciNspYL7TTjNhXGhYUfEeouk3pMchArPcXs2W7xmwv7J2X7TRnzfRg3qujQMrLtny6s1HhxyCFsEXEGs9Qfr` | 505,404,373 | ok | 150 |
| `withdraw-excess-legacy` | `2AnGdKW4NZUUB7AQivbpwR49fYhTvhk1XRG1EMpnQG9kRND3WNVHkeLzZQBGM57A1A7aTfr1ucpmeV3brmBs2KLg` | 505,404,383 | ok | 1,740 |
| `multisig-legacy` | `b7BY3oC7mnvb1Ao3vogidW8ycLg8fyzpDZy87q7pH4Mj5VwuGMexe6zb76LsYizNsST8pCHf7AXE31k8ihc3UcG` | 505,404,392 | ok | 3,314 |
| `create-mint-token-2022` | `3c61718XpwvpgnuQoeGe8mzGpwvSHMwybmTHmVAkbSauhEMvCPyqWbNxHF9X6A6XkFGFM4okaNCmWVyyqEugPogW` | 505,404,400 | ok | 4,881 |
| `immutable-a-token-2022` | `4Df3vSKCavdWsy5dRo5xqNgMYTKuoygsPcHjMyTmAuUdeSxtg7Q3KvEFugdAu1uUjy6LB7ESgWu8shR66WPLX423` | 505,404,452 | ok | 8,575 |
| `immutable-b-token-2022` | `24U8gEU5ujTLUN4YoeUkSvmiK8f61ecB4Vc6QpjppkqgFh5AXxhqv8WKpEZrubHrb8DuamBRCt968Z6bY2LokzpZ` | 505,404,462 | ok | 8,575 |
| `mint-to-token-2022` | `4BJHQQ3Z2VoUKfpqwn7fSKc7Rg396VZ9W7oBeZ9BYYcqTvegs1LDxDPdUXhTQpaYDMnDBTYpiUNPD5D7nQKoJCeL` | 505,404,481 | ok | 2,796 |
| `batch-token-2022` | `3Atm3pE8AumE66e347LJoCk8QocZnAetUzcug9fN4ngKf8wywaW2tiNAzu67CpzPX9MLK2fqBDhkJQx9jeWZNkde` | 505,404,492 | ok | 5,574 |
| `ui-amount-token-2022` | `5Z7Br6UfBbCGsEoRUvUeu1TXQFWyAe2Cymkg5ahCyXBiGrTvMWk1W5HMwPiHSdDTpTnwd766L8CYoNfbFTPD3WtQ` | 505,404,506 | ok | 5,758 |
| `prefund-token-2022` | `28y6TWN8zakUfzsfVXxTefnRqTgm9WZ5qcgpqSK8dQMWmsyt51jmcjgstmiSXPALHBNyFjcfSpXri4daebU3qGwu` | 505,404,514 | ok | 150 |
| `withdraw-excess-token-2022` | `269VnpvJYaWTEs9TWqdPvT2zjmuHQwKARVoWjhjbn4VBn8gpxpnhPmGZP4FYdTUddFKakwSnqDC6otyobB2GkXY8` | 505,404,564 | ok | 2,994 |
| `multisig-token-2022` | `2qxkmhDYKn7PK2wbGd2RhzYnuq53i9U8zwFvf2SYcdXyd2qDUMbY8n7y73N7xgbwGVVDQnNNgksoykJjYoW7vy6x` | 505,404,574 | ok | 4,498 |
| `wrap-and-unwrap` | `64CUgkZWh5yeQRzZ5WXqphgb8HDwEmBDzkzpUCoCmTAovUyUA6K7LyZAPKvBEudHZ6FG73WNrSSTiDph9ssnq2ph` | 505,404,581 | ok | 4,640 |
| `extended-mint-01` | `2PewTEgiH1rbHxBfye9hDbnWFhDST3bDUgDk1N4YjpkfkD1Mejv87w8uEGxMRq97swxpkcqdxCdUZTJunyLgtDoW` | 505,404,594 | ok | 8,942 |
| `extended-mint-02` | `49qpQUjhQcvuh9VrLKvLQHCjiLqfmtHexbE8Txd18F6SPwSn6ZgQ7tar63JZ1mS7u1c8uCFr3kzUDh3Eg9ZWWPYK` | 505,404,607 | ok | 9,306 |
| `extended-mint-04` | `MermaAjpWHwLioC45rEveGKEwB9W6JxKftMecZxziRZQt5gmQM9gpRy5yapPtjk9eru3YYf76ZVGNytCM2VFTiA` | 505,404,620 | ok | 9,491 |
| `extended-mint-08` | `4xrWRbRNrixxayk5XeEhwFKNZeede6k274Cw8YSB7DM76mNdRAaL1FuxzYeK5N6bfZJTcUhAwGYCZjFhYWMt83FA` | 505,404,633 | ok | 9,437 |
| `extended-mint-10` | `5Q2w33qK4vRtnoJhXERsTib1UNyULUE4XAEhwFY3x7baC35drAmy2i93GqPaqPjYwWwNMkfbTKXAa6WLEG6Dex2k` | 505,404,647 | ok | 9,372 |
| `extended-mint-20` | `4dngy7AUeGyUC1zV7Ansduw8nFTTLGDje4ByWoSgYr69WcofSaUVcCukfvAKgaf7S1qGhtcLobDgpNmAPGTCUEQK` | 505,404,660 | ok | 9,302 |
| `extended-mint-40` | `4BeEJQVFBDEwPVyEHNuQARAwzYg3T86EPDkRD8Q9233rW7CbSFrkQSYJMLyLemumxRN9bgugwt7cfZ6HsBmTEnTZ` | 505,404,673 | ok | 9,311 |
| `extended-mint-80` | `HCHzgEmLNtgKUWVUgCmdrcY8BtUQo2mRnhPBvBgtwHtQR2dTMYEGqm2ux9TStC1Aah2qsp418ZSankjqsMJfX91` | 505,404,700 | ok | 9,372 |
| `extended-mint-grow-01` | `2rBen3B3b6ZdkjSBgM3naR75SsWickpKbUqs2FMKBZQCC3CfakEJxJqdhnXnu6R1FD4QptTYU8nofQ8yrYk15x3R` | 505,404,713 | ok | 8,942 |
| `extended-mint-grow-03` | `uAGcgHTMfDrDwBNtTfBf8NsFNJet23qyQdwycYTAgeCARfba3jAgHYJEb3TtLxB9whFSwo8KA8xMXrbi2Kh23zo` | 505,404,726 | ok | 12,126 |
| `extended-mint-grow-07` | `4q7BQDvQdwUQ6aGR6gSYiY7NXHC25UPF2gCqfHVjq9Rwu5wmTzvL7jwUrDTQszixK969tTefEtu63HrS8DmDPTvh` | 505,404,744 | ok | 15,553 |
| `extended-mint-grow-0f` | `4mZLTNWmxNHUMKRmr1MK54YwkgbQyq4wghjGWLwYqS3npqk1tUfxqRhXvu1gAMVB9aw1f4oYUNkni71j6sA71n8R` | 505,404,752 | refused: `InvalidArgument` | 440 |
| `extended-mint-grow-17` | `38v5Az94fhvVr3URzucNaHTuKmcm7EeYKgjQHpA4kAnky34Fw4hmJdtNNrDdAgpADAQwPzRzyNWnssrDghiwHYar` | 505,404,764 | ok | 18,912 |
| `extended-mint-grow-37` | `29tnZLk7Mc5u2sGSSqrk79PbHpD6UH5fJPKehDVsZinGrW37NhX92hDetiiZoKErGcWh3MEqEep2VC6u3WrcpcpC` | 505,404,789 | ok | 22,360 |
| `extended-mint-grow-77` | `DNs4pRE4nHdo2kForqTb1wJwiQAD7GqTHSXz2FUVRa6hBjZbYbAzNu7ptV2kR6rtRE9rabEjsTwfeFcjtsSrRo6` | 505,404,808 | ok | 25,766 |
| `extended-mint-grow-f7` | `3i5SSwkFD2vg7EnViS3FCRmQs3KHS1qi3Xhpsaa2TQjrGZFZXk2BxL5MeJwjB5QSYZrGFW8qz2sWR7J2LiwA2Gjv` | 505,404,826 | ok | 29,295 |
| `extended-mint-final` | `fzovbGb1z7MG1NEHq3LssPGxxiVPHKGvtSfjQFNG45nxkPWm5dp15id4thcj67tvBD1hpWUxBh3ktxShs8tW4zQ` | 505,404,839 | ok | 29,295 |
| `immutable-a-extended` | `4wUF3oWFhyYcaaEyjrqpmbef5pBP1vawmfrcpAjxFNhBTLp9pSgmEKoLfcpBFRVkHEjEJ6txitUD7Ew5pNpDQzj1` | 505,404,846 | ok | 12,380 |
| `mint-to-extended` | `9DmAJgiRwxcAmZ73gLEo1n5GpftsmMQEkxi68QR4cci6qHshhQM7oSirWCzSrMP6wrFFwH6pjFCmXjQ4MFZeVA8` | 505,404,864 | ok | 4,189 |
| `ui-amount-extended` | `JdFV4o9PrnE7oXind3cVqEbc5a6UdcJ5aPQzda66AspN25otRcewRQHfjAJtYyc3XEJ1yREZodfxBVzQGdZvdqL` | 505,404,870 | ok | 10,338 |
| `pause-resume` | `3rHzjtAQg7P8YnqxbY82wxHPuP72gZfGKbQcxTdus6KE9AA1AYx6bCzrRZtcfVRv9EJdCzA4ymnw2bRyiBFvNoKd` | 505,404,930 | ok | 5,930 |
| `update-multiplier` | `5iLd6vSQvpNHdjsDpM72zky7E2ATVqNKVAA5CBJXVuC3vhibnspC2rMapLuB32aJny7tQM9ZPWogMekeDrdZvR85` | 505,404,936 | ok | 3,278 |
| `ui-amount-extended-x3` | `2F75i4s9FSRZSHwxHD1ta2C5q8bF2dPEdyYYpgwEZCwGEDrCcrzoBbgBgJ3cp49BGwFkkePetRBZto6UNruYMW3n` | 505,404,944 | ok | 10,317 |
| `create-metadata-mint` | `3JVyGbWYDnfN7bUKgvKXD1CV8V5HL6BZwnSbxkM6Sz1TMtZgXK6uPvGo5jrZibREjaUjqinMuWzhRKxP4GMxqX1e` | 505,404,950 | ok | 17,783 |
| `set-metadata-key` | `5dhJuz9PQdL1qzJ5y9dkaFdEh3KT8jys9e7k3s1d3WxWEktY5LBmw32Tn1avALCinWHoxDPrYdywVu5HHYgXmR8u` | 505,404,960 | ok | 12,144 |
| `replace-metadata-key` | `52GvPebFLsfePmbemhyYmpkRDFKC9xFEDKET3WxE9XcS4iDxPRjNtSJMjpsy76EQV9XGoXkniDScBQvM5pntoWWX` | 505,405,012 | ok | 12,911 |
| `finalize-metadata` | `yQ9eS2buzn7SAdGdAzDiUXK2TGWCTzLcDSZpJ7f7SdTLuGnHVCHNUWw86Qns7qiuzWtCcZCYDTa6pqgmHqossi8` | 505,405,022 | ok | 32,837 |
| `set-metadata-key-without-authority` | `5uyQ47xr28JAYSMTwLzp1U4PGH5xL4zUvdNE5g5K6yjSqhLzPbDxxfkej68GPiw6caB2xjdHaYGgoBLpzKJia3sK` | 505,405,036 | refused: `Custom(901952961)` | 5,158 |
| `create-group` | `5C7eoMbvYR3B2KjvY7ugdhweCuokbMK6TAGYrRXNSGxpswJQ2J6FHBtGzs3k7eveWdLMzpVZtuMRLp2n9u4NzSZd` | 505,405,044 | ok | 15,425 |
| `create-group-member` | `3ebyVEiqWzPcteaBd2QkCtTdajvzdk12kgj6pR1VZaiV4J8o7AEzScdWPjvKm9C8PpszikZibnivQvhWDJdwjnQ` | 505,405,058 | ok | 16,630 |

`receipt.json` is the runner's own record; `SHA256SUMS` lists every file here.
