use {
    crate::{
        constants::{SPL_TOKEN_BYTES, SPL_TOKEN_ID, TOKEN_2022_ID},
        instructions::TokenCpi,
        state::{MintAccountState, TokenAccountState},
    },
    quasar_lang::{prelude::*, traits::Id},
};

/// Token account view — validates owner is SPL Token program.
///
/// Use as `Account<Token>` for single-program token accounts,
/// or `InterfaceAccount<Token>` to accept both SPL Token and Token-2022.
///
/// Also implements `Id`, so `Program<Token>` serves as the program account
/// type.
#[repr(transparent)]
pub struct Token {
    __view: AccountView,
}
impl_program_account!(Token, SPL_TOKEN_ID, TokenAccountState);

impl Id for Token {
    const ID: Address = Address::new_from_array(SPL_TOKEN_BYTES);
}

/// Mint account view — validates owner is SPL Token program.
///
/// Use as `Account<Mint>` for single-program mints,
/// or `InterfaceAccount<Mint>` to accept both SPL Token and Token-2022.
#[repr(transparent)]
pub struct Mint {
    __view: AccountView,
}
impl_program_account!(Mint, SPL_TOKEN_ID, MintAccountState);

/// Valid owner programs for token interface accounts (SPL Token + Token-2022).
static SPL_TOKEN_OWNERS: [Address; 2] = [SPL_TOKEN_ID, TOKEN_2022_ID];

impl quasar_lang::traits::Owners for Token {
    #[inline(always)]
    fn owners() -> &'static [Address] {
        &SPL_TOKEN_OWNERS
    }
}

impl quasar_lang::traits::Owners for Mint {
    #[inline(always)]
    fn owners() -> &'static [Address] {
        &SPL_TOKEN_OWNERS
    }
}

impl TokenCpi for Program<Token> {}
