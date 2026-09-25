# Hopper byte allowance

A small on-chain quota program demonstrating typed validation, selected-cell
write policies, deny-all lamport policy, generated cell accessors, and revision
checks. It uses no off-chain policy service. Units are application quotas, not
tokens or SOL. The 272-byte account contains four cells; Solana still locks the
whole account for each writable transaction.

The program builds as `cdylib` only so the workspace's fat-LTO profile applies
to its deployed ELF. The verifier includes the source as a host module for
layout checks; it does not require a second library output.

| Opcode | Accounts in order | Arguments after the one-byte opcode |
|---|---|---|
| 0 initialize | authority signer+writable, new book signer+writable, System Program | four delegate addresses, u64 initial limit |
| 1 consume | delegate signer, book writable | u16 slot, u64 expected revision, u64 amount |
| 2 set_limit | authority signer, book writable | u16 slot, u64 expected revision, u64 limit |

Integers are little-endian. Initialization allocates exactly 272 bytes through
System CPI and refuses an existing book. Each successful consume changes
only selected `spent` and `revisions` cells. A limit update changes only
selected `limits` and `revisions` cells. Both update instructions reject stale
revisions and arithmetic overflow. A lowered limit cannot fall below spent
units. There is no reset, close, or rent-recovery instruction.

The bounded entrypoint may ignore surplus account metas after the declared
accounts; `sealed` does not require an exact account count. Instruction payload
lengths are exact. The compiled and devnet suites verify that supplying an extra
account leaves its state unchanged and produces only the expected cell writes.

Read the [on-chain byte-policy guide](../../docs/ONCHAIN_BYTE_POLICIES.md) for
the API, enforcement boundaries, and build/test commands. Compiled verification
is in `bench/framework-comparison/verifier/tests/byte_allowance_sbf.rs`; public
devnet verification is in `scripts/test-byte-allowance-devnet.py`.

The [September 25 release evidence](../../audit/onchain-byte-policies-2026-09-25/README.md)
records three compiled suites on both v0 and v3 and 40 finalized devnet
transactions. The deployed v0 ELF is 24,920 bytes; ordinary consume and limit
updates measured 889 and 831 CU respectively. A fresh registry-only build
reproduces that ELF exactly and passes all three compiled suites. These figures
describe this fixture and toolchain, not a universal accessor cost.
