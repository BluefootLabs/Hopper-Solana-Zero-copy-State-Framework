//! End-to-end compile smoke test for Metaplex context constraints.
//!
//! Verifies that `#[hopper::context]` accepts Anchor-style
//! `metadata::*` and `master_edition::*` account keywords, threads
//! instruction arguments into the generated validators/helpers, and
//! type-checks the generated Metaplex CPI builder lowering.

#![cfg(all(feature = "proc-macros", feature = "metaplex"))]

use hopper::prelude::*;

#[hopper::context]
#[instruction(
    name: &'static str,
    symbol: &'static str,
    uri: &'static str,
    sfbp: u16,
    max_supply: u64,
)]
pub struct MintNft {
    #[account(signer, mut)]
    pub authority: AccountView,

    #[account(mut)]
    pub mint: AccountView,

    #[account(
        metadata::mint = mint,
        metadata::mint_authority = authority,
        metadata::payer = authority,
        metadata::update_authority = authority,
        metadata::system_program = system_program,
        metadata::name = name,
        metadata::symbol = symbol,
        metadata::uri = uri,
        metadata::seller_fee_basis_points = sfbp,
        metadata::is_mutable = true,
    )]
    pub metadata: AccountView,

    #[account(
        master_edition::mint = mint,
        master_edition::metadata = metadata,
        master_edition::update_authority = authority,
        master_edition::mint_authority = authority,
        master_edition::payer = authority,
        master_edition::token_program = token_program,
        master_edition::system_program = system_program,
        master_edition::max_supply = max_supply,
    )]
    pub master_edition: AccountView,

    pub token_program: AccountView,
    pub system_program: AccountView,
}

#[test]
fn metaplex_data_v2_context_validation_accepts_limits() {
    hopper::hopper_metaplex::DataV2::simple(
        "01234567890123456789012345678901",
        "SYMBOL1234",
        "https://example.com/nft.json",
        500,
    )
    .validate_for_context()
    .unwrap();
}

#[test]
fn master_edition_max_supply_accepts_u64_and_option() {
    let finite: Option<u64> =
        hopper::hopper_metaplex::IntoMasterEditionMaxSupply::into_master_edition_max_supply(0u64);
    let unlimited: Option<u64> =
        hopper::hopper_metaplex::IntoMasterEditionMaxSupply::into_master_edition_max_supply(
            None::<u64>,
        );

    assert_eq!(finite, Some(0));
    assert_eq!(unlimited, None);
}
