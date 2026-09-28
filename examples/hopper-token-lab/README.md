# Token lab

Every SPL Token and Token-2022 builder Hopper ships, run against the real
programs on devnet. One instruction per builder family; every instruction
takes the executable token program as its last account and derives the
target from it, so the same program serves SPL Token and Token-2022.

| Tag | Instruction | Builders it proves |
|---|---|---|
| 0 | `create_mint` | `MintPlan` with no extensions on either program |
| 1 | `create_extended_mint(mask)` | `MintPlan` with any set of default account state, pausable, scaled UI amount, interest bearing, group pointer, permissioned burn, metadata pointer, and group member pointer |
| 2 | `immutable_account` | `GetAccountDataSize` (the size is the return data), `InitializeImmutableOwner`, `InitializeAccount3` through `invoke_for_owner` |
| 3 | `mint_to(amount, decimals)` | `MintToChecked` through `invoke_for_owner` |
| 4 | `batch_round_trip(amount, decimals)` | `TokenBatch` carrying two `TransferChecked`s in one CPI |
| 5 | `ui_amount_round_trip(amount)` | `AmountToUiAmount` then `UiAmountToAmount` on the returned string; the round-tripped amount and the string are the return data |
| 6 | `withdraw_excess` | `WithdrawExcessLamports` |
| 7 | `init_multisig(m)` | `InitializeMultisig2` over two members |
| 8 | `wrap_and_unwrap(lamports, unwrap)` | a native (wrapped SOL) account through `InitializeAccount3`, then `UnwrapLamports` |
| 9 | `pause_resume` | `Pause` and `Resume` on a pausable Token-2022 mint |
| 10 | `update_multiplier(f64 bits)` | `UpdateScaledUiAmountMultiplier` |

The devnet runner is `scripts/test-token-lab-devnet.py`. It builds nothing:
it takes a deployed program id and the ELF it must match, verifies the
on-chain dump before and after, drives both lanes, checks every touched
account byte by byte (mint fields, token account fields, TLV entries and
their lengths, multisig members, lamport balances against the live
rent-exempt minimum), and writes a receipt with the findings: which cases
the live programs accepted and which they refused. A refusal by the live
program (an extension the deployed Token-2022 does not know yet, `Batch` on
Token-2022, `UnwrapLamports` on a program without discriminator 45) is
recorded as a finding, not hidden.

```text
py -3.12 scripts/test-token-lab-devnet.py \
  --program <deployed id> --payer <keypair> --hopper target/release/hopper.exe \
  --elf target/deploy/hopper_token_lab.so --out target/hopper/token-lab-devnet
```

Build with `cargo build-sbf` from this directory. The program is 53,656
bytes as sBPF v0 with the default release profile.
