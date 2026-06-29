use hopper::prelude::*;

// Stand-in program ids; in real code these are
// `hopper::token::TOKEN_PROGRAM_ID` and
// `hopper::token_2022::TOKEN_2022_PROGRAM_ID`. Using local consts keeps this
// trybuild case independent of the optional token feature crates.
const TOKEN_ID: Address = Address::new([1u8; 32]);
const TOKEN_2022_ID: Address = Address::new([2u8; 32]);

// `owner_any = [..]` accepts an account owned by *either* program -- the
// Token / Token-2022 interface-account first-touch. The macro must parse the
// array and emit a `check_owned_by_any` guard over the listed ids.
#[derive(Accounts)]
pub struct TransferTokens<'info> {
    pub authority: Signer<'info>,

    #[account(owner_any = [TOKEN_ID, TOKEN_2022_ID])]
    pub token_account: UncheckedAccount<'info>,
}

fn main() {}
