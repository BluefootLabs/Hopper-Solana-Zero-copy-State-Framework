use hopper::prelude::*;

const A: Address = Address::new([1u8; 32]);
const B: Address = Address::new([2u8; 32]);

// `owner = expr` and `owner_any = [..]` are mutually exclusive: a field has one
// owner rule, not two.
#[derive(Accounts)]
pub struct Bad<'info> {
    pub authority: Signer<'info>,

    #[account(owner = A, owner_any = [A, B])]
    pub token_account: UncheckedAccount<'info>,
}

fn main() {}
