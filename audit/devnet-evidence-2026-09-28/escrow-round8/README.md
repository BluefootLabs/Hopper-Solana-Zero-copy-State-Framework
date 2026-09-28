# Devnet evidence, 2026-09-28: the token escrow, round eight

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is
`examples/hopper-escrow`, built with `cargo build-sbf` in a clean worktree at
commit `fcc66e9103ffd775f2657e36d91aee886e96a821` (local ELF SHA-256
`48706fcb65a7bfa41776dc4b30c583781f34897146ffbf8387c19b941c1d61f1`, 42,232
bytes) and deployed fresh as `9vnEXTpz7TC2ugC5LEDLdJ6i8bBNEradv996hUwFSfPw`
at slot 505,152,698. `before-onchain.so` and `after-onchain.so` are the
`solana program dump` of the deployment before the first transaction and
after the last one; both equal the local ELF byte for byte.

The runner is `scripts/test-token-escrow-devnet.py` at the same commit. It
sent 33 transactions (slots 505,152,759 to 505,153,308), re-fetched each at
`finalized`, and compared every touched account with the exact state it
expected: the setup (funding, two mints, four token accounts, supply), the
two makes, and then every refusal the program owns with the state snapshot
unchanged: `make-insufficient-funding` (`Custom(1)` from the token program),
`make-zero` (`Custom(6104)`), `reinitialize` (`AccountAlreadyInitialized`),
`stale-quote` (`Custom(6101)`), `unsigned-take` (`MissingRequiredSignature`),
`wrong-cancel-maker` (`InvalidAccountData`), `cancel-trailing-data`
(`InvalidInstructionData`), `wrong-vault-authority` (`InvalidSeeds`),
`wrong-token-program` (`InvalidArgument`), and `wrong-payment-recipient`,
which the program refuses at bind with `Custom(45058)` at 241 CU: the
taker's payment account passed in the maker's receiving role is one account
in two undeclared mutable roles. Round seven stopped at that case because
the runner still expected the later `InvalidAccountData`; this round's runner
expects the bind-time refusal. `donate-take` and `donate-cancel` fund the
vaults past the offer, then `take` (8,707 CU) and `cancel` (4,122 CU) settle
and close with the surplus refunded and rent recovered.

`receipt.json` lists every signature, slot, error, compute-unit count, and
fee; `*.transaction.json` are the finalized transactions as the RPC returned
them; `*.snapshots.json` are the account states before and after each step;
`*.log` are `hopper tx send`'s own output, unedited. `SHA256SUMS` covers every
file here except itself and `BUNDLE.SHA256`, which is its digest.
