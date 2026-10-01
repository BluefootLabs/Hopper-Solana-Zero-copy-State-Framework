# Token lab

Shared token operations, selected Token-2022 extensions, metadata, and
groups, run against the real programs on devnet. The table defines this
lab's scope; other builders have separate fixtures. Token operations take
the executable token program as their last account and derive the target
from it, so one program serves both token programs.

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
| 11 | `create_metadata_mint(name, symbol, uri)` | `MintPlan` with a metadata pointer that names the mint, the rent top-up for the metadata, `InitializeTokenMetadata` |
| 12 | `set_metadata_key(key, value)` | `UpdateMetadataField` on an additional key, then `EmitTokenMetadata` (Token-2022's serialized metadata is the return data) |
| 13 | `finalize_metadata(name, key)` | `UpdateMetadataField` on the name, `RemoveMetadataKey` strict and idempotent, `UpdateMetadataAuthority` to none, `EmitTokenMetadata` |
| 14 | `create_group(max_size)` | `MintPlan` with a group pointer that names the mint, `InitializeTokenGroup` |
| 15 | `create_group_member` | `MintPlan` with a group member pointer, `InitializeTokenGroupMember` |
| 16 | `batch_self_transfer(amount, decimals)` | Deliberate writable alias constructed inside a batch; must fail before a token CPI |
| 17 | `resolve_hook_list([u8; 51])` | Caller-supplied Execute TLV list; literal/PDA resolution and malformed-input refusals |

The devnet runner is `scripts/test-token-lab-devnet.py`. It builds nothing:
it takes a deployed program id and the ELF it must match, verifies the
on-chain dump before and after, drives both lanes, checks token balances,
mint and account fields, TLV entries and lengths, multisig members, return
data, and lamports against live rent. Batch round trips and self-transfer
refusals preserve the checked accounts' complete data, owner, lamports,
and executable flag. Excess-lamport withdrawal preserves token data and
credits the recipient by the expected amount after fees. The runner saves
its finalized account observations and writes a receipt with which cases
the live programs accepted and which they refused. A refusal by the live
program (an extension the deployed Token-2022 does not know yet, `Batch` on
Token-2022, `UnwrapLamports` on a program without discriminator 45) is
recorded as a finding, not hidden.

```text
py -3.12 scripts/test-token-lab-devnet.py \
  --program <deployed id> --payer <keypair> --hopper target/release/hopper.exe \
  --elf target/deploy/hopper_token_lab.so --out target/hopper/token-lab-devnet
```

Build with `cargo build-sbf` from this directory. Test a specific build with
`HOPPER_TOKEN_LAB_SBF` set to its absolute path without the `.so` suffix;
an explicitly missing fixture fails instead of skipping.

The hook probe tests parsing and address resolution. It does not invoke a
transfer-hook program or establish an account's owner/PDA provenance.

## Devnet, 2026-09-28, round ten: metadata and groups

Deployed fresh as `4MWp9iQM1qxLrz9sYj4jdf9BU4waEo58MEPs7R4j7m28` from a clean
worktree at `2ec0930`; 49 finalized transactions, every state check exact
(`audit/devnet-evidence-2026-09-28/token-lab-round10/`). The deployed
Token-2022 accepts the token-metadata and token-group interfaces. The mint
that carries its own metadata cost 17,783 CU to create and initialize, was
362 bytes, and held exactly the rent for that size. Setting a key cost
12,144 CU. Renaming the token, removing the key twice, giving up the
authority, and emitting cost 32,837 CU for the five CPIs. After every step
the `TokenMetadata` entry equalled the expected Borsh bytes and `Emit`
returned the stored entry. The group cost 15,425 CU and its member 16,630,
and the group counted one member afterwards.

## Devnet, 2026-09-28, round eight

Deployed fresh as `417akw6B2CcuTZFpePrZaSR5oyX3riBPtHmdePkjrAYJ` from a
clean worktree at `244b72c`; 42 finalized transactions, every state check
exact (`audit/devnet-evidence-2026-09-28/token-lab-round8/`). The findings
the live programs settled: both SPL Token and Token-2022 accept `Batch`
(the round trip of two `TransferChecked`s is one token CPI, 2,472 and 5,575
CU for the whole instruction); SPL Token accepts `UnwrapLamports`
(4,475 CU including the account's creation); every one of the eight
extensions selected by this lab is accepted alone, seven of them together
(29,294 CU for the mint with default account state, pausable, scaled UI
amount, group pointer, permissioned burn, metadata pointer, and group
member pointer), and interest bearing with scaled UI amount is the one pair
the program refuses, which `MintPlan` now refuses at plan time (439 CU,
before any CPI). The scaled mint displayed 1,234,567 as `2.469134`, then
`3.703701` after the multiplier update, and both round-tripped to the raw
amount.
