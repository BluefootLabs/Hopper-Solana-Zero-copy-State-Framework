# Cicada canonical route fixture

This compiled fixture models a small two-leg token swap without ever mutating
token account bytes itself. It invokes the supplied canonical SPL Token or
Token-2022 program for all token movement:

1. Move input from Cicada's source vault to a solver-owned input sink.
2. Move output from the solver's reserve to the intent destination.

Every token and mint account in one route must belong to the same supplied
token program. The first transfer is authorized by Cicada's vault PDA. Cicada signs the route
CPI and forwards that signer privilege. The second transfer is authorized by
the transaction-signed solver/liquidity authority. The route calls
Hopper's explicit-program token-interface `TransferChecked` path for both
legs, with signer and token-authority prechecks before the canonical processor
checks the complete instruction.

## Route data

The route accepts exactly 19 bytes:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 1 | command |
| 1 | 8 | input amount, little-endian |
| 9 | 8 | output amount, little-endian |
| 17 | 1 | input mint decimals |
| 18 | 1 | output mint decimals |

Commands are exported by the crate:

| Command | Behavior | Expected Cicada result |
| --- | --- | --- |
| `0xB0` | Both transfer legs | Settle when economic bounds pass |
| `0xB1` | Both legs with a test-supplied underpayment | `MinimumOutputNotMet` and full rollback |
| `0xB2` | Output leg only | `EmptySettlement` and full rollback |
| `0xB3` | Input leg only | `EmptySettlement` and full rollback |
| `0xB4` | Both legs, then canonical source `SetAuthority` | `SourceTokenPolicyChanged` and full rollback |
| `0xB5` | Both legs, then canonical `MintToChecked` plus balancing `BurnChecked` | `ProtectedAccountDelegation` before route CPI |

The hostile commands remain ownership-correct. They only request operations
that the supplied token authorities may legally authorize. The supply-neutral
command restores the output mint and reserve to the same bytes an honest swap
would leave, demonstrating why end-state hashing alone is insufficient. Cicada
must reject writable delegation of either committed mint before route CPI.

## Route accounts

| Index | Account | Route flags | Required property |
| ---: | --- | ---: | --- |
| 0 | source token vault | writable | Owned by the Cicada vault PDA |
| 1 | input mint | read-only | Canonical SPL Token or Token-2022 mint |
| 2 | solver input sink | writable | Must not be owned by the Cicada vault PDA |
| 3 | Cicada vault PDA | signer | Read-only signer forwarded by Cicada |
| 4 | solver output reserve | writable | Owned by account 7 |
| 5 | output mint | read-only | Canonical SPL Token or Token-2022 mint |
| 6 | intent destination | writable | The destination committed by the intent |
| 7 | solver/liquidity authority | signer | Transaction signer |
| 8 | token program | read-only | Matching canonical SPL Token or Token-2022 executable |

The corresponding Cicada route flag prefix is
`[1, 0, 1, 2, 1, 0, 1, 2, 0]`; every unused flag must remain zero.
The `0xB5` containment test marks account 5 writable, producing
`[1, 0, 1, 2, 1, 1, 1, 2, 0]`, and configures account 7 as its mint
authority. Honest swaps keep both mint accounts read-only.

## Required SVM proof

Integration must register Mollusk's vendored canonical SPL Token or Token-2022
ELF and load this fixture at its own program ID. A successful swap test should
assert two token-program invocations at CPI depth 3, followed by Cicada's
unused-input refund at depth 2. The source-policy hostile case should snapshot
the complete instruction account envelope and prove nested canonical writes
roll back. The supply-neutral mint/burn case must be rejected before the route
program is invoked at all.
