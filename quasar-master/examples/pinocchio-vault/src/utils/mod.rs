pub mod pda;

use pinocchio::AccountView;

pub struct Context<'info> {
    pub accounts: &'info [AccountView],
    pub data: &'info [u8],
}
