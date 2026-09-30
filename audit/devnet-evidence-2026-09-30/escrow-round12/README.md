# Devnet evidence, 2026-09-30: the token escrow, round twelve

Public devnet (`https://api.devnet.solana.com`), signer `4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority). The program is `examples/hopper-escrow`, built with
`cargo build-sbf` from a clean tree at commit `847a0b0`
(local ELF SHA-256 `eff61b6b19447517c2a7cd9595baaeb4ebbe1c4e9f54d51fb2c5e7411b1b6153`, 42,264 bytes) and
deployed fresh as `G3pfUoRq9rNcJC3SpTphsNFBKhXFR4VNxN2AsDy82h2a` at slot 505,888,590.
`before-onchain.so` and `after-onchain.so` are the `solana program dump` of
the deployment before the first transaction and after the last one; both
equal the local ELF byte for byte.

The runner is `scripts/test-token-escrow-devnet.py`, the same one round
eight ran at `fcc66e9`. It sent the same 33 transactions, re-fetched each at `finalized`, and compared every
touched account with the exact state it expected, the refusals included.
Among the changes between the two rounds are the entry walk and the
System builders' CPI (`f6333ae`) and the token builders' CPI (`847a0b0`: borrow
checks, the lamport gate, and the refusal of one account in two writable
roles, with privileges left to the runtime). The escrow's own
instructions, next to round eight's:

| Step | Round eight (`fcc66e9`) | Round twelve (`847a0b0`) |
|---|---:|---:|
| `make-take` | 7,455 | 6,868 |
| `make-cancel` | 7,127 | 6,213 |
| `take` | 8,707 | 7,836 |
| `cancel` | 4,122 | 3,586 |

Every refusal round eight recorded is refused here with the same error
and the same unchanged state.

33 transactions:

| Step | Signature | Slot | Result | CU |
| --- | --- | --- | --- | --- |
| `fund-maker` | `3UPqAQfQr15BvEdYCfwK8kE88TSy1Mr44AJEtAkM5JMWwcqLVM2s8Q4YVhsXK7XaWPXbsapxEpw8YuqEqW4TWvrv` | 505,888,797 | ok | 150 |
| `fund-taker` | `5mBZP4Ek3xyZ1KMboGuCrKsvEfwM6hnEz6N7u3P8cYU6whw2n7YKdawVVT8NKzdMCfEEVBKzLEf1g2DCD3tm2DJy` | 505,888,806 | ok | 150 |
| `fund-outsider` | `4VXhpfxgtefPpzgYNNrCUw5ZSLQUegyo7H6jcjXoBvNfbngPLmJLj1QkNU9o5hEzp4TVKKnpLmnxW5sp5naHfHuo` | 505,888,818 | ok | 150 |
| `create-mint_a` | `5wALt1hYqxYSRzKF8PkTG8TDbntsEiQAWi9CZJEocYGyWe1mjrMGeDpDCC2JxdtYRfZQiBbXNikEi6DV5vLC1yZU` | 505,888,831 | ok | 150 |
| `initialize-mint_a` | `4QWtpcXgHdZ7N73CfS6i7SgGxvaXAZN3MkRsrRuwBpPYuZH1P6yyavQsPfvFrRyJtjvJDwUyNaUbqvtQ8nuuMb87` | 505,888,882 | ok | 202 |
| `create-mint_b` | `39A6K1ZqEpPchashVy6nDJxNxEfp671qG7t7HnABEtU8AP4njAdjPo8RwkaU4W3G6fK6RZvmC6PYT5G6LiKVxryS` | 505,888,896 | ok | 150 |
| `initialize-mint_b` | `4c1wpv5hbpZYuUTRttLUkkyyWDvBEedyQfvAwWpR6TpeEZcnEoqS7niFYEKenqLgMPaL7VzAnmLbFZzoUKJNZF5x` | 505,888,904 | ok | 202 |
| `create-maker_a` | `2mLhxc2h4zXeYrVfPqCkPh6UntA5dR3ZkN3iUE24EkLt7NLCTvNHjHLbsoNcofYnUYB9dHbFCaWP6dCJF3ShqPyb` | 505,888,935 | ok | 150 |
| `initialize-maker_a` | `2j8jr26Hm4mhMsnb9WXXJ8FKypFxXnXLfBLgoMV3APFMf7cxVgH1G5sEofVDs4Q4foKFRQ89MgYCRXmgPSyYwqxu` | 505,888,985 | ok | 233 |
| `create-maker_b` | `3JXZozVNCRgTJW6iTU1tMNGbqDw4VFicrTJj4RPyJTT3sxnCe9DhDrFNYpT9FwkgwYMrZUiUU8Lj8tQAaeu1nAGF` | 505,888,991 | ok | 150 |
| `initialize-maker_b` | `C5fTYdai3ZSPu7fFWci2tVonYie5DZq8Nbd9mokk1ES7EwRGFDhiCdVM9pZCR79PrmDMyeBjeGVNUR3iqwUgD6i` | 505,889,004 | ok | 233 |
| `create-taker_a` | `24r69qVptAxSo5MTzVmDer3Ao9fJz21oaPvMMCQYXyetw7mqnr76XjAM5qkKGEeDD9yND7HZ9GEZfVtJdazLhcSd` | 505,889,058 | ok | 150 |
| `initialize-taker_a` | `vrsAyUNdaeeZEbo3yCwYyQfY92Vfb6bhAQPvapADCBfNw5Zh8SDv9kcckxbc1PoRXnJamrVoye9rCbGzfAy88Dc` | 505,889,066 | ok | 233 |
| `create-taker_b` | `j5AhaPZxK6uQfmR7V9DK4gXkg2N8qUAPRAt2TmeqTGxMf9qvMLzez45f5h3Nmw4LYeMdkeG1hPSyD6jKV2XVSRq` | 505,889,075 | ok | 150 |
| `initialize-taker_b` | `2KmWW639BXnPst9sX94H9QoHgbVsyMHus7LXLhS9i7QP6q9Kruy7VmgPQRv1fQW1cLAajcsaeF8CBKV6F8KUMfbd` | 505,889,084 | ok | 233 |
| `mint-maker_a` | `2sr7mqrnmDePvTy9mJkb8AdK4BJdXozSQtoSAH9Crf9SivcDXUsP7EkG2Nk1yN5u1Qct9NCx7LAiT9Dhma1GBsnA` | 505,889,137 | ok | 153 |
| `mint-taker_b` | `hcYGgMuzzLLANY5W5uSVAx8wQTsjUT9zjxfrVzPQveMkR4BePXyEkZFjJKer1qyiELYWTjqVbBhzqyFUqohcqYE` | 505,889,144 | ok | 153 |
| `make-insufficient-funding` | `5q66eYoJ9MLHKnFTEJSjsawurAsGAzeWDZEfR4FVftSSaWGD2wihJ3oEQuThNVHMi8PAwYgbgt37kCBKnkFdkgxc` | 505,889,152 | refused: `Custom(1)` | 6,939 |
| `make-zero` | `4kLHgwTbNGB7fEdWBfmc49uu6PWK2iTmFrcgrxBU3cGgL66hcPobC5asAGnx5iNWFrfc5rZUk9wAXGxUMsfgZsnb` | 505,889,164 | refused: `Custom(6104)` | 1,291 |
| `make-take` | `5TGaoo7hrbJDWpmz1g9rgYDCpE2RY89HLipRthMQ1XXWnHspuSJUpKYDfm6Zxq6Rme7APQP9An8S2zeGBx38MrVw` | 505,889,178 | ok | 6,868 |
| `make-cancel` | `5Qc7kdr14B4nK6BWMBsMyUCCktMuP21WJrGRRZ5AjyhwG8kxv58x4cgTiB2KCFbbVzJs82xSQWYhY7X1hDJCC4ek` | 505,889,196 | ok | 6,213 |
| `reinitialize` | `bN2aptNLtnE4khPLBkyLw3oyGsSBE23vVBBBk3Lk1Ej4oyyfynoe3Ds1kG9ibTZDpETq1jtoBnv3kct7C54bPrK` | 505,889,208 | refused: `AccountAlreadyInitialized` | 1,822 |
| `stale-quote` | `2HJtKaBqcpD6GL3kyABjduf7XydpqKBQuohPuZ5gvvyuaduquJQv1UhekRkqfNqXcEgaLutHjEVzXUuAEwKfWty9` | 505,889,221 | refused: `Custom(6101)` | 2,296 |
| `unsigned-take` | `3rsTRzMjEuSBs839p5s3iZnQkXChBYcPENCV8tZb7phT4ETiQhZt7guvLxy9krCyV16Um7PuELKMaZjkYSAM6WxN` | 505,889,234 | refused: `MissingRequiredSignature` | 267 |
| `wrong-cancel-maker` | `3fiFF8JdTDTkzeKEitwUmLjYjX4PDUgMMuqVC6oCWnYVGeeDrqFz6yPpwDP2Bj1qCX18px38M4B6mpVW6RXfjGfd` | 505,889,248 | refused: `InvalidAccountData` | 229 |
| `cancel-trailing-data` | `3UaXAtrtdScnFFuS5YfkHNsx4SSA1LSeFkZy7GRfJk1uUzWGkZMTaeZLhbehGwYvNJYmFwuVkVgufsZTwFeKsrME` | 505,889,263 | refused: `InvalidInstructionData` | 720 |
| `wrong-vault-authority` | `5ZguqSDAxzvrTakWMR9NSFpx5y7DxBEbv14rrfjcLBJgvSaWD1aa3XKiWm5w1V4qVvAGACb6SMhG94paezMZTfV3` | 505,889,277 | refused: `InvalidSeeds` | 1,546 |
| `wrong-token-program` | `3exFwj4Waa4pjy1WDSCNgpsB3CP2hSM6Wjtkv5xSfMTvhQQYY5KQtv5yptipB68g6dDMZpWqybZdQfNGaC99DsVD` | 505,889,298 | refused: `InvalidArgument` | 1,600 |
| `wrong-payment-recipient` | `ioCgiRFxeChjgywftGqK7AV7fmvir3ZTRsJWewepsaT3wHTKFdjQ3TqtR1oZCY4JZJzvtx9bE2BrBWYZzuoi5eC` | 505,889,344 | refused: `Custom(45058)` | 252 |
| `donate-take` | `V4sG9UtPTmBbMFdbMfaWG6ZLZB9ffKPpbpgH9RwkmKKujRWf65DyP7GXbqWQLW6BS8gtGRQqzzvnbZ51iS8R1EA` | 505,889,356 | ok | 105 |
| `donate-cancel` | `5cyHdfTzcncRkTWT7YRdGcQwfbZfkbVUGEMrJH8eaDhGZ2vE5ztEGkmdxpKPGkjFGghSdPGq4drZXMYZKFaW9uGa` | 505,889,364 | ok | 105 |
| `take` | `GFbqmtNJnaV2D4HFZxDVDAYRM5AKr4YUPEfSmq68ij5xCv2n5W8umWzXgYH81A247RdQzigsuSuV5fLwVQLqNY1` | 505,889,377 | ok | 7,836 |
| `cancel` | `p4NUMPVNn4wJyRQFjLzFj7F4ZS5HTY19XKkrfaA7W8ZYVef7rN1UKj1uD4bbFBx1sAYb3H3J5eG5vQt4uZjfAMp` | 505,889,385 | ok | 3,586 |

`receipt.json` lists every signature, slot, error, compute-unit count, and
fee; `*.transaction.json` are the finalized transactions as the RPC returned
them; `*.snapshots.json` are the account states before and after each step;
`*.log` are `hopper tx send`'s own output, unedited. `SHA256SUMS` covers
every file here except itself and `BUNDLE.SHA256`, which is its digest.
