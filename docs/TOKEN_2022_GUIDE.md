# Token-2022: validate and initialize mints

Hopper provides zero-copy token readers, declarative extension constraints,
and fail-closed TLV policies. These are distinct from mint creation: reading
an extension never initializes it.

`MintPlan` and `InitializeMint2` below are workspace additions after the
published 0.3.0 release. Use the matching workspace until the next registry
release is verified.

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
use legacy SPL Token. In this workspace, every extension constraint also
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

Variable-length token metadata, confidential extensions, and automatic
extension inference are outside this API. Initialize unsupported extensions
through their own reviewed instruction builders, then validate the result.

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
  sizes and emitted bytes with the canonical SPL interface.

Mint initialization does not replace the token readers and authority checks
needed by later instructions. Validate each operation's own contract.
