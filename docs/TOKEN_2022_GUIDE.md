# Token-2022: validate and initialize mints

Hopper provides zero-copy token readers, declarative extension constraints,
and fail-closed TLV policies. These are distinct from mint creation: reading
an extension never initializes it.

`MintPlan` and `InitializeMint2` below ship in Hopper 0.3.1. Use the matching
framework and CLI release; registry packages and compiled consumer checks
are recorded on the [release status page](https://hopperzero.dev/docs/release-status).

## Pin ownership and authority

Use the Token-2022 program explicitly when the instruction requires its
extension semantics. A token-program owner check alone does not authorize
the caller to operate on a mint.

```rust
use hopper::prelude::*;
use hopper::token_2022::TOKEN_2022_PROGRAM_ID;

#[derive(Accounts)]
pub struct ConfigureMint<'info> {
    #[account(
        mut,
        mint::authority = *ctx.account(1)?.address(),
        mint::token_program = TOKEN_2022_PROGRAM_ID,
    )]
    pub mint: UncheckedAccount<'info>,
    pub authority: Signer<'info>,
}
```

`mint::token_program` and `token::token_program` choose the required owner
for the corresponding mint or token constraints. Without an override they
use legacy SPL Token. In 0.3.1, every extension constraint also
requires Token-2022 ownership before reading its bytes, even when a separate
mint/token owner constraint is omitted. The published 0.3.0 release requires
that explicit owner constraint; retain it in code that must support 0.3.0.
Raw TLV reader functions do not establish ownership on their own.

## Create a mint with an exact extension plan

This helper creates a signer mint with a close authority and metadata
pointer. The pointer identifies an account; it does not create or populate
metadata in that account.

```rust
use hopper::prelude::*;
use hopper::token_2022::{MintConfig, MintExtension, MintPlan, MintProgram};

pub fn create_mint<'info>(
    payer: &Signer<'info>,
    mint: &Signer<'info>,
    authority: &Address,
) -> ProgramResult {
    let extensions = [
        MintExtension::MintCloseAuthority(Some(authority)),
        MintExtension::MetadataPointer {
            authority: Some(authority),
            metadata_address: Some(mint.address()),
        },
    ];
    let plan = MintPlan::new(
        MintProgram::Token2022,
        MintConfig {
            decimals: 9,
            mint_authority: authority,
            freeze_authority: None,
        },
        &extensions,
    )?;
    // Include System and Token-2022 program accounts in the instruction.
    // Both payer and mint must be writable and sign this creation.
    plan.create(payer.as_account(), mint.as_account(), &[])
}
```

For a PDA mint, pass the executing program's `Signer` seeds to `create`.
A signature authorizes the creation; it does not prove that an application
selected a canonical PDA. Validate the address policy separately.

The plan supports these six fixed-size mint extensions:

| Variant | Initialization configuration |
| --- | --- |
| `TransferFeeConfig` | Configuration and withdrawal authorities, basis points, maximum fee |
| `MintCloseAuthority` | Optional close authority |
| `NonTransferable` | Non-transferability marker |
| `PermanentDelegate` | Delegate address |
| `TransferHook` | Optional authority and hook program |
| `MetadataPointer` | Optional authority and metadata account |

`plan.space()` includes the base state, Token-2022 padding and TLV headers.
`plan.check_space(requested)` rejects both undersized and oversized allocations.
The token processor requires the size to match the extensions actually
initialized; arbitrary spare space is not accepted by this plan.

`MintPlan::new` rejects duplicate extensions, fees above 10,000 basis points,
and zero addresses in optional fields where zero encodes absence. A legacy
`MintProgram::Legacy` plan accepts an empty extension list and uses 82 bytes.

`create` uses the live Rent sysvar, funds only the shortfall on a prefunded
System-owned empty mint, initializes the listed extensions, and initializes
the base mint last. It uses `CreateAccountAllowPrefund`, so the target cluster
must support that System instruction. `initialize` accepts an already
allocated, correctly owned, rent-exempt, entirely zeroed account of exactly
the planned size. Neither operation creates token accounts or mints supply.

Propagate errors from these multi-CPI operations so the enclosing instruction
rolls back earlier CPIs. Catching an error and returning success can retain
partial work. The low-level `InitializeMint2` builder is also available when
you manage allocation and extension initialization yourself.

The plan covers the thirteen fixed-size mint extensions: transfer fee, mint
close authority, non-transferable, permanent delegate, transfer hook,
metadata pointer, default account state, interest bearing, scaled UI amount,
pausable, group pointer, group member pointer, and permissioned burn. Each
variant's allocation and bytes are checked against the canonical
`spl-token-2022-interface` constructors. Variable-length token metadata,
confidential extensions, and automatic extension inference are outside this
API.

## Every Token-2022 instruction

The shared instruction set (transfers, mints, burns, approvals, close,
freeze, authority changes, account and multisig initialization, the
data-size and UI-amount queries, excess-lamport withdrawal) lives in
`hopper::token`. Every builder there targets SPL Token from `invoke()` and
Token-2022 from `invoke_on`; `invoke_for_owner` reads the first account's
owner, picks the program, and refuses anything that is not one of the two:

```rust
use hopper::token::{GetAccountDataSize, InitializeAccount3, InitializeImmutableOwner, TokenProgram};

let program = TokenProgram::owning(mint)?;
let size = GetAccountDataSize { mint, extension_types: &[7] }.query(program)?;
// allocate `size` bytes owned by `program.address()`, then:
InitializeImmutableOwner { account }.invoke_on(program, &[], &[])?;
InitializeAccount3 { account, mint, owner }.invoke_for_owner(&[], &[])?;
```

The Token-2022-only instructions live in
`hopper::token_2022::extension_instructions`: `CreateNativeMint`,
`InitializeNonTransferableMint`, `Reallocate`, and every extension family's
initializers, updates, and toggles (transfer fee, default account state,
memo transfer, interest bearing, CPI guard, permanent delegate, transfer
hook, metadata pointer, group pointer, group member pointer, scaled UI
amount, pausable, permissioned burn, mint close authority). Each has
`invoke`, `invoke_signed`, and, where an authority is involved,
`invoke_multisig` and `invoke_signed_multisig`:

```rust
use hopper::token_2022::extension_instructions::{Pause, Resume, UpdateScaledUiAmountMultiplier};

Pause { mint, authority }.invoke()?;
UpdateScaledUiAmountMultiplier { mint, authority, multiplier: 3.0, effective_timestamp: 0 }.invoke()?;
Resume { mint, authority }.invoke()?;
```

`TokenBatch` collects any of these builders and sends them as one `Batch`
CPI; SPL Token (p-token) accepts it, and the token-lab devnet runner records
whether the deployed Token-2022 does.

## Metadata and groups on the mint

Token-2022 implements the token-metadata and token-group interfaces, so a
mint can be its own metadata account and its own group. The builders live
in `hopper::token_2022::metadata_instructions`. They send to Token-2022
from `invoke()` and to any other program that implements the interface
from `invoke_on_program`.

```rust
use hopper::token_2022::metadata_instructions::{
    InitializeTokenMetadata, MetadataField, UpdateMetadataField,
};

// The mint was created with a metadata pointer that names itself.
InitializeTokenMetadata {
    metadata: mint,
    update_authority: authority,
    mint,
    mint_authority: authority,
    name: "Hopper",
    symbol: "HOP",
    uri: "https://example.com/hop.json",
}
.invoke()?;

UpdateMetadataField {
    metadata: mint,
    update_authority: authority,
    field: MetadataField::Key("tier"),
    value: "gold",
}
.invoke()?;
```

Two things to know before you ship this:

- Token-2022 grows the mint to hold the metadata and does not pay for it.
  Transfer the rent for the new size to the mint first, or the instruction
  fails. The token lab's `fund_growth` is a few lines and does exactly
  that.
- The instruction is encoded on the stack and capped at 512 bytes. A
  longer name, symbol, URI, or value is refused with `InvalidArgument`
  before the CPI, never truncated.

`RemoveMetadataKey`, `UpdateMetadataAuthority`, and `EmitTokenMetadata`
(the serialized metadata comes back as return data) cover the rest of the
metadata interface. `InitializeTokenGroup`, `UpdateTokenGroupMaxSize`,
`UpdateTokenGroupAuthority`, and `InitializeTokenGroupMember` cover groups.

## Confidential transfers

`hopper::token_2022::confidential_instructions` has a builder for each of
the fifteen confidential-transfer instructions. Hopper does not encrypt,
decrypt, or prove anything on chain: the ciphertexts, the ElGamal keys, and
the proofs are made off chain with the account's keys, and your program
carries them to Token-2022 as bytes.

Each proof an instruction needs is a `ProofLocation`: another instruction
in the same transaction (an offset from the Token-2022 instruction, read
through the Instructions sysvar) or a context-state account that the ZK
ElGamal proof program verified earlier.

```rust
use hopper::token_2022::confidential_instructions::{ConfidentialWithdraw, ProofLocation};

ConfidentialWithdraw {
    account,
    mint,
    authority,
    amount,
    decimals,
    new_decryptable_available_balance: &new_balance,
    equality_proof: ProofLocation::ContextStateAccount(equality_context),
    range_proof: ProofLocation::ContextStateAccount(range_context),
    instructions_sysvar: None,
}
.invoke()?;
```

The account order is the part people get wrong by hand, so the builder
owns it: the instruction's accounts, the Instructions sysvar once if any
proof is by offset, the context-state accounts in proof order, the
authority, then the multisig signers. A proof by offset with no sysvar
account is refused with `NotEnoughAccountKeys` before the CPI. Every
builder is compared with the `spl-token-2022-interface` constructor for
all 32 combinations of proof locations, with a single authority and with
a multisig.

### Run against the program mainnet runs

`examples/hopper-confidential-lab` puts each builder behind an instruction
and drives the whole flow through them under Mollusk, against the
Token-2022 that mainnet-beta runs (`program@v11.0.0`, dumped on 2026-09-29
and pinned by hash) and the ZK ElGamal proof program, with proofs made by
`solana-zk-sdk` and `spl-token-confidential-transfer-proof-generation`:

| Step through Hopper's builder | Proofs | CU, lab instruction |
|---|---|---:|
| Create a confidential mint (`InitializeMint`, then `InitializeMint2`) | | 6,592 |
| Configure, proof in a context-state account | pubkey validity | 11,343 |
| Configure, proof by instruction offset | pubkey validity | 14,294 |
| Configure from an ElGamal registry (Token-2022 grows the account) | | 14,144 |
| Approve, update the mint | | 2,870, 2,438 |
| Deposit | | 11,304 |
| Apply the pending balance | | 9,330 |
| Withdraw | equality, range u64 | 7,418 |
| Transfer | equality, validity, range u128 | 16,746 |
| Transfer on a fee mint | five | 46,703 |
| A credit toggle (any of the four) | | 2,360 |
| Empty, proof by instruction offset | zero ciphertext | 9,280 |

The proof program's own cost is separate: 2,600 CU for a pubkey validity
proof, 6,400 for equality, 16,400 for three-handle validity, 111,000 for a
u64 range proof, 200,000 for u128, and 368,000 for u256. The tests decrypt
what landed: the recipient reads the transferred amount, the auditor reads
it from the ciphertexts the builder carried, and the withdraw authority
reads the withheld fee. A replayed withdraw proof is refused (`Balance
mismatch`), and so is a registry that belongs to someone else.

Two things the lab found that are easy to trip on:

- On a mint with a confidential transfer fee, size new accounts for
  `ConfidentialTransferFeeAmount` yourself. `GetAccountDataSize` makes
  room for the transfer-fee amount and the confidential state, and
  `ConfigureAccount` then fails with `InvalidAccountData`.
- The Token-2022 that Mollusk 0.15 bundles is v7.0.0, whose ciphertext
  operations are compiled out: it answers `Deposit` with
  `InvalidInstructionData`. Test confidential flows against a current dump.

No public cluster can run this flow today. The ZK ElGamal proof program is
disabled on mainnet-beta, testnet, and devnet, and Token-2022 only moves a
confidential balance against a proof that program verified. The builders
are ready for the day it is enabled.

## Extension constraints

Mint-side constraints include close authority, permanent delegate, transfer
hook authority/program, metadata pointer authority/address, default account
state, interest-bearing rate authority, transfer-fee authorities, and presence
checks for non-transferability, confidential transfer, and scaled UI amounts.
For example, add these to a Token-2022 mint field:

```rust
use hopper::prelude::*;
use hopper::token_2022::TOKEN_2022_PROGRAM_ID;

#[derive(Accounts)]
#[instruction(close_authority: Address)]
pub struct InspectMint<'info> {
    #[account(
        mint::token_program = TOKEN_2022_PROGRAM_ID,
        extensions::mint_close_authority::authority = close_authority,
        extensions::non_transferable,
    )]
    pub mint: UncheckedAccount<'info>,
}
```

Token-account constraints include `extensions::immutable_owner`,
`extensions::cpi_guard`, and `extensions::confidential_transfer::account`.
Presence does not mean that the program supports an extension's behavior.
`default_account_state::state` compares a raw byte: 1 is Initialized and 2 is
Frozen. A present extension with a matching field is not an allowlist of every
other extension on the account.

## Apply a policy before interpreting TLV bytes

For custody or settlement, start with an explicit allowlist of extension
semantics the application supports:

```rust
use hopper::token_2022::validate_extension_allowlist;

// `tlv` is the TLV region of an owner-checked account.
// An empty allowlist rejects every extension.
validate_extension_allowlist(tlv, &[])?;
```

The allowlist rejects unknown IDs, duplicate entries, truncation, and entries
outside the list. The required/forbidden `ExtensionPolicy` helper validates
TLV structure too, but does not reject every unlisted known extension.
Use the strict mint screening helpers when checking a complete finalized
mint envelope; a raw TLV policy alone does not prove owner, mint identity,
initialization, authority, or every extension's payload semantics.

The current constants cover official discriminators through PermissionedBurn
(28). A raw presence reader can inspect an unknown ID; that does not grant
permission to accept its behavior. Never treat `find_extension(...) == None`
as a complete safety decision on unvalidated input. Check payload lengths
before slicing or interpreting bytes.

## Resolve transfer-hook extra accounts

`extra_account_metas_pda`, `ExtraAccountMetaList`, and `HookAccountBuf<N>`
resolve the hook's declared extra accounts without heap allocation. Use
`.address()` on the resolver's account input for its address. The list supports literal
addresses, this-program PDAs, external-program PDAs, and literal,
instruction-data, or account-key seeds. Account-data seeds return
`HookError::UnsupportedSeed` and require explicit handling.

The resolver computes account addresses and privileges. The caller must
validate the metadata-list PDA and its owner, supply the resolved accounts,
and choose a capacity that accommodates the list. It does not fetch accounts
or run the transfer hook.

## Working programs and executable checks

- `examples/hopper-token-2022-vault` binds an existing mint and authority to a
  reward vault, creates the vault ATA, mints rewards, and sweeps tokens. It does
  not impose non-transferability or a mint-close-authority constraint.
- `examples/hopper-token-2022-transfer-hook` demonstrates hook integration.
- `bench/mint-plan/program` exercises mint creation and initialization against
  canonical token processors, including PDA and prefunded mint creation.
- `crates/hopper-spl/hopper-token-2022/tests/mint_plan.rs` compares allocation
  sizes and emitted bytes with the canonical SPL interface;
  `crates/hopper-runtime/src/token_differential_tests.rs` does the same for
  every builder's bytes and account metas.
- `examples/hopper-token-lab` runs one instruction per builder family against
  SPL Token and Token-2022 on devnet through
  `scripts/test-token-lab-devnet.py`, with byte-level checks of every
  touched account.

Mint initialization does not replace the token readers and authority checks
needed by later instructions. Validate each operation's own contract.
