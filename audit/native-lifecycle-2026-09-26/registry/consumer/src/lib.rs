use hopper_native::{AccountView,Address,ProgramResult,RefMut};
pub fn resize<'a>(program_id: &Address, account: &AccountView<'a>, payer: &AccountView<'a>, system_program: &AccountView<'a>) -> ProgramResult {
    hopper_native::batch::ResizeWithPayer {account,payer,system_program,program_id,new_len:32}.invoke()
}
pub fn map(account: &AccountView) -> ProgramResult {
    let mut first = RefMut::filter_map(account.try_borrow_mut()?, |bytes| bytes.get_mut(..8))
        .map_err(|_| hopper_native::ProgramError::AccountDataTooSmall)?;
    first.fill(0); Ok(())
}
use hopper::prelude::*;
#[derive(Accounts)]
pub struct Authority<'info> { pub authority: Signer<'info> }
