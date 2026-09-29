# Devnet evidence, 2026-09-29: the confidential-transfer flow, round eleven

Public devnet (`https://api.devnet.solana.com`), signer `4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and upgrade
authority). The program is `examples/hopper-confidential-lab`, built with
`cargo build-sbf` in a clean worktree at commit `be4f535`
(SHA-256 `7e5ae588ac446589fe43e8429d885b4c921a2839e47a016decdffa2717aed6cf`). The program `2vHnFzA1tZUtcUAFur14BMGPpq9YzpYFtBJ43Bbz9pqt`
was deployed at slot 505,682,198 from the same source at commit `25ed970`;
the runner checked that the deployed program equals this ELF byte for byte
before the first transaction and after the last one. The runner is
`examples/hopper-confidential-lab/runner`, built at the same commit.

Devnet's Token-2022 was `program@v11.1.0` (711,008 bytes, SHA-256
`b781b71d230a9910de4d3c103e72608f597fc73c83ee207242ac82b708382a6a`). Every confidential-transfer instruction below went
through the lab program, and so through Hopper's builders, to that program;
every proof was made off chain by the runner with `solana-zk-sdk` and
`spl-token-confidential-transfer-proof-generation` and verified by devnet's
ZK ElGamal proof program: into a context-state account, by instruction
offset in the same transaction, or, for the u128 and u256 range proofs that
do not fit in a transaction next to the compute-budget instruction a
verification needs, from an SPL Record account.

A first attempt at commit `046195e` stopped at the u128 range proof: with
the compute-budget instruction its transaction was 1,245 bytes, 13 over the
limit, and the RPC refused it before it reached the cluster. Its 20
finalized transactions are not part of this bundle. The runner now sends
that proof through a record account as well; this is the second run.

What the runner checked after each step (from `receipt.json`):

- create-mint: a 235-byte confidential mint, approval required, auditor key stored
- open-alice: configured with the holder's ElGamal key
- open-alice: not approved yet, as the mint requires
- approve-alice: approved
- update-mint-auto-approve: new accounts are approved when configured
- create-carol-registry: the registry program stored Carol's key
- open-carol-from-registry: configured from the registry, grown by Token-2022, approved
- open-dave-with-carols-registry: refused, no account left behind
- open-bob-proof-by-offset: configured and approved
- deposit: 300,000 left public, the pending balance decrypts to 700,000
- apply-pending: the available balance is 700,000
- withdraw: 200,000 back to the public balance, 500,000 confidential
- withdraw-replayed: the same proofs are refused, nothing moved
- transfer-verify-range-u128: the SPL Record account holds the 1000-byte proof
- transfer: Bob and the auditor both decrypt 123,456; Alice keeps 376,544
- apply-pending-bob: Bob's available balance is 123,456
- credit toggles: each flag flips; a public transfer is refused while they are off
- withdraw-rest: the whole confidential balance is public again
- empty-proof-by-offset: Token-2022 accepted the zero-ciphertext proof
- open-erin: configured with the holder's ElGamal key
- open-frank: configured with the holder's ElGamal key
- fund-erin: the pending balance decrypts to 500000
- fund-erin: the available balance is 500000
- fee-transfer-verify-range-u256: the SPL Record account holds the 1064-byte proof
- transfer-with-fee: Frank credited 99000, 1000 withheld under the withdraw authority's key
- 15 context-state accounts and 2 record accounts closed, rent returned

60 transactions, re-fetched at `finalized`.

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| `00-create-mint` | `35zrnd6zvRBoeXcQCDZC7QF7wj7MbSFwqySMt3qC8osBrqxC69pUHRAqpSdKqBicL1fGW7yH412faMfxzo7uHWww` | 505,687,948 | ok | 7,321 |
| `01-open-alice-verify-pubkey-validity` | `FKrbTrAAF5t5nNYjECzsZeqNehYXU27JGtsE4BTGDqnPdaDFdQffJY2YkjSHZUoPY3LXE9a16HpBW7VKf8MDnat` | 505,687,961 | ok | 2,900 |
| `02-open-alice` | `3Mp9bRbvHfwYYWtET4CPVA76RyB7dyJaeWjYXx1reUcBPghKt7LjAnkjKR7uQybceAaVLBqHxfCEhwt61mfEYNjm` | 505,687,972 | ok | 12,626 |
| `03-approve-alice` | `4a85NPKMwUoTy16mGmQGnkrSbpMhSX98WC3pozRMGtkm6DRkomiN2f38KjXad2cirPkGJcwzNfq71gpt2cqohz89` | 505,687,984 | ok | 3,275 |
| `04-update-mint-auto-approve` | `2E43BVm4yAX2SdZH2VH8YWqwokxDwmJKdYFBoT5dvFgC3jntz4XWUTLy8hY4q24jh8Wb9if1ouMfskL8U4AnqLPh` | 505,687,997 | ok | 2,854 |
| `05-create-carol-registry` | `4n8ZLehRkcNwJDbUmhmQDLn3HtzkmUCxFap27fqiKdig3CnHJqrCuziqbiceczMifrWMtbeqzQt1BKBCB7fc7ALF` | 505,688,010 | ok | 11,832 |
| `06-open-carol-from-registry` | `SmdSmfTxghvkxhdonDZjhZ3Qw3ggoR3ykDukHccGaK5BoyPFp8V3Dv5JsLrfnxhxCnGeNB2X5WzBRXAURWamb8Z` | 505,688,021 | ok | 15,588 |
| `07-open-dave-with-carols-registry` | `2tVnBXAXTgSVzT75xLNrQzdeVimcGcsep3av5QTcyMPxTU4bEzubh3aMGCVuwWii2sJjaZo82wv5DDMWRXGwz5Uj` | 505,688,033 | refused: `Custom(4)` (instruction 0) | 14,666 |
| `08-open-bob-proof-by-offset` | `tq1aLkKHp8anMwr7FVAQzj75JmsSkNSgQEowqeeCTwibibs8uvytVnLuQb2vPmJPxrVXYgTmmeNrxaeqm15Chnx` | 505,688,045 | ok | 15,705 |
| `09-mint-to-alice` | `2Lx7nqbPFM3oSZC4nGSeqDvmEH9vXxH8cYp3pp87RaxCnHNk8NLxEqEj3SJf3FE9FAZpakfeMz66RhPZUTYhdXdm` | 505,688,057 | ok | 1,970 |
| `10-deposit` | `TToXi8aNzUPGYS4UyVH8ofb3gggoqsxaXfJ4gUwNk1f2AW1Hi7zUayCjABWBD7k9BrqJF4WMBhzPzjKAwux4RQA` | 505,688,068 | ok | 11,732 |
| `11-apply-pending` | `5xuvzSxJKU3ncqJTre9ZubxCXkePn717GdqUd6XH9W3nvr2Fbx9k1vwnJoSKDpKjbNjd7v7YMZS776Gz3PBYGXtx` | 505,688,082 | ok | 9,317 |
| `12-withdraw-verify-equality` | `5e9y9rsVdLpGQVcsPoHHsikXKiohzLpnH5SF7Z9dxztyk8ap8c2qWGk1657KcK82Ko7hiSK85o5MKE9HncoLKRjh` | 505,688,096 | ok | 6,700 |
| `13-withdraw-verify-range-u64-context` | `zfg6WwayCKjfkHkX7f5GyUXmt142hAKGvhdKwXVztYHvkqsKskEqkBw1BFpWCm1SQmfxoYXDLEYcmCDApr7RTsE` | 505,688,107 | ok | 150 |
| `14-withdraw-verify-range-u64` | `4d1tWbrpLzdaNNbhuobqtZjibmyiD33yHPJhX4vCLdSGi59qerKThE7ELSHc2YqFXuKEeqd9KKfWC6YYqjSt2Qrt` | 505,688,119 | ok | 111,150 |
| `15-withdraw` | `4KxUUjY5Vqzd8rcsAVeUkc4WqC8c6X4vbKfqM9ZZajbpZccSBRjHRD9EDpzMkqST7By9Z1hrTKDPPuF8sQSsfvU5` | 505,688,130 | ok | 7,822 |
| `16-withdraw-replayed` | `33wFk9ngbnM2cFhbtj7sjY2c8gVkJuaNnW3ipFqaY9ZyVtfwb2NsWwtHnFuYVdamTSj55KYtJbAHnQuwdB5t748` | 505,688,143 | refused: `Custom(27)` (instruction 0) | 7,925 |
| `17-transfer-verify-equality` | `xuRoFJCoQMhdbEhZ7GSk9zFXUbvWr1fU4Wb1Ar4nNoDsmnzxYqkR6Wdgu3EULUpqJcPMbVc3vHuHU1K1XpuAkQy` | 505,688,160 | ok | 6,700 |
| `18-transfer-verify-validity` | `5MVzMnLZuNnYPkth4mXBiJDDLh1XyqQVrQBKChkfSRvjD5Jc68pTfr4XHsBZWYayDAcvYLHRYKGZMjbuoyhr2Pb2` | 505,688,168 | ok | 16,700 |
| `19-transfer-verify-range-u128-record` | `4fTnr9JGG7pDYixe3Ey4cF46oPGkDvb48nmFzcmK9khMWbB2PGX92biUsuqsb7PgRQFK86y8VtYMpquEKRbgEmvW` | 505,688,188 | ok | 696 |
| `20-transfer-verify-range-u128-record-write-0` | `2J2BwwKPsetpzXxW6ojKyuoY3WyeZnLYbKVvgTnukwtu5U29Vtuw9yWfwSEQdpNPPtmu3YAckRdLnUKLv9qrL9LR` | 505,688,199 | ok | 649 |
| `21-transfer-verify-range-u128-record-write-1` | `3UfKWowmzkLRBuqRBDUuCiaC91FxyjzSHeK5TY6jgHbP9iouwLfEuVP5Ss3tzEiapgSPCsC9QojvU6iqcqj2mer5` | 505,688,210 | ok | 649 |
| `22-transfer-verify-range-u128-context` | `4SSQYkjMfWgNLkYoGUD9MTGgsGwtcdb9X3PzKVGtfUUV9mnzUHW5L9Ny4CrkBTDDTQn9dSycW2QgsCzWL1wd3Qqa` | 505,688,223 | ok | 150 |
| `23-transfer-verify-range-u128` | `tNM2oN7febG578YTMRFooxxRQ1hQ6xGfz7PGV5PFU73CBRVTsBRdaQgvwQ44gaZe3qLRVsRi6kwDXkoSGb75pDo` | 505,688,234 | ok | 200,150 |
| `24-transfer` | `5bQXuSgJYoBM9EW3VJiyZjA5Z3m7TqrVvDghh3WkgvhKgwHYH4rZsdsuehhrccHoanKEVR4YaSp52XhWrWBPcUe2` | 505,688,245 | ok | 17,134 |
| `25-apply-pending-bob` | `3QUPyv5YVECng99qE53rJhqWfHceJiiH87tvKXFrGvHN1TYbp5iC8o5wdw6MZVkQ9WvRhVsDaFgS6UbnaJGM4vfc` | 505,688,260 | ok | 9,317 |
| `26-disable-confidential-credits` | `3LSt2cRW6B9wZND17nvuEkV88pzt5VozTSY4WHReX4YbdFXmD5HvwYezkMPuEJFPU3E7YFXTAAQn1Do4JGQNffSi` | 505,688,271 | ok | 2,347 |
| `27-enable-confidential-credits` | `3RNy7QPKWFNALDtJdMsVBA3QRBAmKqYbA4dr7wm2WFAeqvjhbBWiA9AM9hWGZ8NPFVh9MLF3jrCyWmtjxwTwpNYV` | 505,688,283 | ok | 2,346 |
| `28-disable-non-confidential-credits` | `5SqTsmjDssVXMGNpYL6SgiVLoKf6UkaH8UbFm1o2pNUmztjaM1Z91b6uc9RAvDa8PQfxL5yEserEAspk2jEtPxRP` | 505,688,296 | ok | 2,347 |
| `29-public-transfer-refused` | `2mE5goAP95owPxQ532HB3vcP2oStg3oAoTJ5eJ26a34xHHYL6guD92efTejJBdHbXze8jVW7Z6NaQyY5FHTjnWgu` | 505,688,308 | refused: `Custom(49)` (instruction 0) | 2,887 |
| `30-enable-non-confidential-credits` | `2ePuuwgkf4VtT8gPPCbxwD4bFq3Zh9zNFMQ5dL8QUaxo3W32c6LX8wEacVbWdzUKemitmt4386HNwCMHyEyXi5zE` | 505,688,320 | ok | 2,345 |
| `31-public-transfer` | `2XPmikgY6E5BZXsPqUziTuf26bk8gcazNzUuWXjqNGpRWGbnr7Duh6Ttk5qDfUrX2eGhLLQnU5MivA1M59qa4RhD` | 505,688,330 | ok | 2,775 |
| `32-withdraw-rest-verify-equality` | `41xafkDLT4JA9qjmgPnyy9ntMJXyMRfgtFP94XLFtC4K5FGnLx5wJUc5TjqFGfdJPrgNZ6e3L1YTiYiMyFNNG2xZ` | 505,688,344 | ok | 6,700 |
| `33-withdraw-rest-verify-range-u64-context` | `2pKLkj3mvE1WYEztA7A1rUjquPNhmhAHzxhWYN6uRkeKaPmX46nYanwyXdhxwGmf22gYBWVouKkcc4Q87tLN6htd` | 505,688,360 | ok | 150 |
| `34-withdraw-rest-verify-range-u64` | `59mcHSMNPMkGCVg6j9xFhKqgERgZCmCFRR7tyHRcNZ8TpUntuvCMY4FBf26AT5kR1xkFmwGYoxLhndi2ThemkVeX` | 505,688,368 | ok | 111,150 |
| `35-withdraw-rest` | `4BgBWLL2ewWwcpML7MzridhhC3ycrL2E81T2SJwR9C3k9ju9mNaCNZv9ucFrtpsyGFgr9qJhDzXVuXds3wsD7e79` | 505,688,379 | ok | 7,822 |
| `36-empty-proof-by-offset` | `3nXJArfjE2bUNEmVXWTsuxWsZXDQNr9KPSSev2DASSjTzaYu678QAapKMbTWAdjtCXDtQD2ZCKQAX3zx3nphXEnr` | 505,688,396 | ok | 9,394 |
| `37-create-fee-mint` | `4t7g1iQptDLKTLKpxULqcTkvYH5FEyY69JrKyPSvgUVmvPpzfP2bDHqTViDAyKXSe1kJFu9ZCRWWaU4SZrNCDsTn` | 505,688,415 | ok | 7,061 |
| `38-open-erin-verify-pubkey-validity` | `ZB56TcTrvUSbEs2tCWN1TroAahzf2YwqxEVY663cBRtkKf63A6jdckM4hTvugb1E9HRCZj2zr7bF3UmTbUu6eo4` | 505,688,428 | ok | 2,900 |
| `39-open-erin` | `FHPjoqK9BfQYECFS8L8VhLD8BsD2XJNREzV6GfrMJpAbdX6BD2N1h193x6ur74TJKAML3fJckoWzhsQ9J1xGZLj` | 505,688,440 | ok | 14,051 |
| `40-open-frank-verify-pubkey-validity` | `hGDQ3nK9CKW5gTV6x2nHHfFx2nvC8ghdwzWG2mPqQkoCq1uUumzysrZfCsKKZSsh1h79Fbnxbt6by7SEVCE1X8D` | 505,688,452 | ok | 2,900 |
| `41-open-frank` | `5V7AMmH7yR4AHERev7KWyXG1QkcWDUe6pEVnnf7p64MHu9gY8f9VyabMZS2ZjS4yQFZCMiT5MFrhAq8T5zTUq7sq` | 505,688,464 | ok | 14,051 |
| `42-fund-erin-mint-to` | `DLxcysPHqm1uqYLvpT3yjzS38AxyAQvKPtDXmDfpef6fq5ZEVb8ruG6Scbs8nheAmmyHRHfMKeQD2NRDxpcLYwp` | 505,688,476 | ok | 2,294 |
| `43-fund-erin-deposit` | `3A7hwayjGxAPTD9FCd9nJac754w3QYyV7DnwJACyPECiCp2YSPXh5KKiNNcaXmpUxJd4VcnPmaTM5x27pwvqb5cV` | 505,688,488 | ok | 12,112 |
| `44-fund-erin-apply` | `4WyyDuTzSM52RKAqatv1v9DfzZhrDfCDJuPLRHkQ6TUutXgh5AavfFPNFe6tywEipSoxou4KaY4NSDP72bbNknQy` | 505,688,501 | ok | 9,372 |
| `45-fee-transfer-verify-equality` | `4NE5Xy2HuaABXikcqps1MFBZ8TZ6LUBvzMGZNXo92tZT2fGyyDieAMLtdrt7VeodoseLDBpKcBmnWm9ArA91NPDj` | 505,688,514 | ok | 6,700 |
| `46-fee-transfer-verify-amount-validity` | `3MBxvjCuvHsd4jQGcQSbrpxHAadrqnBUbKSfWzn7oeR9M7n3YfbSkGzyNKmtMF4yJjqo9uDYtzGPssWUSutUC6Vq` | 505,688,526 | ok | 16,700 |
| `47-fee-transfer-verify-fee-sigma` | `3kLhoBnutmsSq2Z1ADH5xb5mEqHVAjvmFbnUvoJmqVtuwdVTdUGpa3ZHgrJ8eUk5fmFfTp7GW1ad8JMc6MbBfB2G` | 505,688,539 | ok | 6,800 |
| `48-fee-transfer-verify-fee-validity` | `vsE93Pdh5iCTYKecG3xpaSrFPBBkkTVTzfiuqJmk3U9VwCoqxMKCysjS11tZ17Fs5foU686kJSn8vPrHqsQYnF6` | 505,688,550 | ok | 13,300 |
| `49-fee-transfer-verify-range-u256-record` | `3qmQzQVoqB3gCDjYb1Eq86hCnKp3Mr9VTXgonxpnv62BkhgnYnYwuEJr2wbzX13ay6Xyi7YCkN3yRzaHzaaq8o1T` | 505,688,563 | ok | 696 |
| `50-fee-transfer-verify-range-u256-record-write-0` | `Y7evi5ZJmt71wAyqHytgJD9yd8Gs6WQN835k9VjJADmfBk5zxDVnHWoePz5CbZhVDDBBaNWHdpDzpnZ9b6AUtoF` | 505,688,583 | ok | 649 |
| `51-fee-transfer-verify-range-u256-record-write-1` | `48NEevF5LG8tGMc1HvBH2U5PbHMKV9KntyXWPtD1zPgkCa9yZkwR4Y3XvuqReUPHs3zdZnvDwvrmgZALBVCEn9Ym` | 505,688,594 | ok | 649 |
| `52-fee-transfer-verify-range-u256-context` | `3NVtDe6431A2JCaSRkvJqAiQgwqjUStgdrVzuc7KzTsbTh8rwZuoVFf5aNURyKEKp8rUGDmij48t49pScZTrepyh` | 505,688,607 | ok | 150 |
| `53-fee-transfer-verify-range-u256` | `2kk7ByECQivMSWxeBSKTqr2wpivtSvnpUyV1zTRYnMHGRisAu2sBq2nmr7eoANPG2FuXaVH2N9vvz27hXAvr5sgT` | 505,688,618 | ok | 368,150 |
| `54-transfer-with-fee` | `48Ch4m8MvbfYkYcXyaiVPsSxYu71pBLy8sdnM4YD5ZNcxg9vi6WQxB8GXRhz51zBRjkasbVeam8KiqYz5waLaQBC` | 505,688,631 | ok | 46,974 |
| `55-close-contexts-0` | `3XYJgo8vAZ4EcTrwYdp3rLxBB3mMgwcD2qmFWTkfjPaTrtLXcPDtTJJgFYxHqG2UX84QFQpQWytSUwCnpnFr1Uas` | 505,688,647 | ok | 19,800 |
| `56-close-contexts-1` | `4uu8Fu9uuni2AkwBEGvKtkXHwv9MKDYmqGLMZdyQDikfhg1o5WaED5DeR1qkV8TyrJ8hTKc9QkuhmVEhL1AKhbHe` | 505,688,659 | ok | 19,800 |
| `57-close-contexts-2` | `JdTWeAa2SBByM8oJU8RrQg43qx9MGrG47kKRVJu2SjfmephhC2SUJrwjAm5n6vXDd79pW94CaAkpD78YmzDs5yR` | 505,688,674 | ok | 9,900 |
| `58-close-record` | `5HY7LeMryEDo4UZg9wBuv7JVB7z7wApykbPgZn116hHL168BKQ8Mo8VZcbdmi7kwayBEbW53pBeg8xJYM9xXTypn` | 505,688,688 | ok | 699 |
| `59-close-record` | `4j72zDqtWQ6zexiZMgS5kVJAXtRyXyWEUPC6bmPAtCy4xHbNqQzV2tDCRAHZ2VczRViegjYSwHRVRKrGWQbxuqQ1` | 505,688,702 | ok | 699 |

`receipt.json` is the runner's own record; `SHA256SUMS` lists every file here.
