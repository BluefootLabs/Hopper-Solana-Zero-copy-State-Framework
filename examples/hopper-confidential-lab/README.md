# Confidential lab

Every Token-2022 confidential-transfer instruction Hopper builds, behind
an instruction of this program, so the whole flow runs through Hopper's
builders against the real Token-2022 program and the ZK ElGamal proof
program. The program holds no key and builds no proof: the tests make the
keys, ciphertexts, and proofs off chain with `solana-zk-sdk` and
`spl-token-confidential-transfer-proof-generation`, as a wallet would, and
pass them in.

| Tag | Instruction | Builders |
|---|---|---|
| 0 | `create_mint(auto_approve, auditor)` | `InitializeConfidentialTransferMint`, then `InitializeMint2` |
| 1 | `update_mint(auto_approve, auditor)` | `UpdateConfidentialTransferMint` |
| 2 | `open_account(max_pending, zero_balance, proof_offset, extra_extension)` | `GetAccountDataSize`, `InitializeAccount3`, `ConfigureConfidentialAccount` |
| 3 | `approve` | `ApproveConfidentialAccount` |
| 4 | `deposit(amount, decimals)` | `ConfidentialDeposit` |
| 5 | `apply_pending(expected_credits, new_balance)` | `ApplyPendingConfidentialBalance` |
| 6 | `withdraw(amount, decimals, new_balance, offsets)` | `ConfidentialWithdraw` |
| 7 | `transfer(new_balance, auditor_lo, auditor_hi, offsets)` | `ConfidentialTransfer` |
| 8 | `credits(which)` | the four credit toggles |
| 9 | `empty(proof_offset)` | `EmptyConfidentialAccount` |
| 10 | `open_from_registry` | `ConfigureConfidentialAccountWithRegistry` with a payer, so Token-2022 grows the account |
| 11 | `transfer_with_fee(new_balance, auditor_lo, auditor_hi, offsets)` | `ConfidentialTransferWithFee` |

A zero proof offset means the proof is in the context-state account
passed; a nonzero one means it is in another instruction of the same
transaction, and the account passed is the Instructions sysvar.

## Run it

From this directory:

```text
cargo build-sbf
cargo test --test flow -- --nocapture
```

The tests load `fixtures/token_2022_v11.0.0.so`, the Token-2022 that
mainnet-beta runs, and refuse the file if its SHA-256 is not the pinned
one. `fixtures/README.md` says where it came from and how to dump it
again. `CONFIDENTIAL_LAB_TOKEN_2022=<path>` runs the tests against another
dump; devnet's `program@v11.1.0` passes them too.

## What the tests cover

`the_confidential_flow_runs_through_hoppers_builders`: a mint that needs
approval and has an auditor; Alice configured with her proof in a
context-state account and approved; the mint switched to auto-approve; Carol
configured from her ElGamal registry, the account grown by Token-2022 at
the payer's cost (a registry that belongs to someone else is refused); Bob
configured with the proof in the instruction before, in the same
transaction; a deposit and its pending balance decrypted by Alice; the
balance applied; a withdraw with equality and range proofs, and the same
proofs replayed and refused; a transfer with three proofs, the amount
decrypted by Bob and by the auditor from the ciphertexts the builder
carried; the four credit toggles, with a public transfer refused while
non-confidential credits are off; the rest withdrawn and the account
emptied with a zero-ciphertext proof by instruction offset.

`a_transfer_on_a_fee_mint_carries_five_proofs`: a mint with a 1% transfer
fee capped at 5,000; 100,000 transferred with five proofs; Bob credited
99,000 and 1,000 withheld in his account, decrypted with the withdraw
authority's key.

The Token-2022 that Mollusk bundles (v7.0.0) cannot run this flow; its
ciphertext operations are compiled out, which is why the tests pin the
mainnet build.

## On a public cluster

The ZK ElGamal proof program verifies proofs on mainnet-beta, testnet, and
devnet, so `runner/` runs the same flow for real: every step a transaction
through this program, the proofs made off chain and verified by the
cluster, the accounts read back and decrypted after each step. The u128
and u256 range proofs do not fit in a transaction next to the
compute-budget instruction a proof verification needs, so they are written
to an SPL Record account and verified from there. At the end the
context-state and record accounts are closed and their rent returned.

```text
cargo run --release -p hopper-confidential-lab-runner -- \
  --program <deployed lab> --elf ../../target/deploy/hopper_confidential_lab.so \
  --payer <keypair> --rpc https://api.devnet.solana.com \
  --out ../../target/hopper/confidential-flow-devnet
```

Round eleven ran it on devnet on 2026-09-29: 60 finalized transactions
against Token-2022 `program@v11.1.0`, all fifteen builders, every check
passed. The receipt, each transaction, and the checks are in
`audit/devnet-evidence-2026-09-29/confidential-flow-round11`.

| Step on devnet | CU |
|---|---:|
| `open_account`, proof in a context-state account | 12,626 |
| `open_account`, proof by instruction offset (with the verification) | 15,705 |
| `open_from_registry` (Token-2022 grows the account) | 15,588 |
| `deposit`, `apply_pending` | 11,732, 9,317 |
| `withdraw` | 7,822 |
| `transfer` | 17,134 |
| `transfer_with_fee` | 46,974 |
| `empty`, proof by instruction offset (with the verification) | 9,394 |
| Verify a u64, u128, u256 range proof | 111,150, 200,150, 368,150 |
