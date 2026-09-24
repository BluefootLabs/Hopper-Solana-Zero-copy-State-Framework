//! Compile the guide's complete examples and reject documentation/code drift.
#![cfg(feature = "proc-macros")]
#![allow(dead_code)]

mod example_0 {
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
}

mod example_1 {
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
}

mod example_2 {
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
}

#[test]
fn guide_matches_compiled_examples() {
    let guide = include_str!("../docs/TOKEN_2022_GUIDE.md").replace("\r\n", "\n");
    assert!(guide.contains(
        r####"use hopper::prelude::*;
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
}"####
    ));
    assert!(guide.contains(
        r####"use hopper::prelude::*;
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
}"####
    ));
    assert!(guide.contains(
        r####"use hopper::prelude::*;
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
}"####
    ));
}
