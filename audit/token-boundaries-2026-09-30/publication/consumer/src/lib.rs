use hopper::prelude::*;
use hopper_runtime::token::{TransferChecked,TokenBatch};
use hopper_token_2022::hook::{ExtraAccountMetaList,HookError};
#[derive(Accounts)]
pub struct Authority<'info> { pub authority: Signer<'info> }
pub fn selected_bump(seeds:&[&[u8]],program:&Address)->Result<(Address,u8),ProgramError> {
    hopper_runtime::pda::try_find_program_address(seeds,program)
}
pub fn inspect_hook(bytes:&[u8])->Result<usize,HookError> {
    Ok(ExtraAccountMetaList::unpack(bytes)?.len())
}
pub fn transfer<'a>(source:&'a AccountView<'a>,mint:&'a AccountView<'a>,to:&'a AccountView<'a>,authority:&'a AccountView<'a>)->ProgramResult {
    let mut batch=TokenBatch::<64,8>::new();
    batch.push(&TransferChecked{from:source,mint,to,authority,amount:10,decimals:6})?;
    batch.invoke()
}
pub fn payout<'a>(source:&'a AccountView<'a>,destination:&'a AccountView<'a>,mint:&'a AccountView<'a>,authority:&'a AccountView<'a>,token:&'a AccountView<'a>,configured:&Address)->ProgramResult {
    let receipt=hopper_solana::transfer::TokenTransferSnapshot::capture(source,destination,configured,100,99)?;
    hopper_solana::interface::interface_transfer_checked_with_program(source,mint,destination,authority,token,100,6)?;
    receipt.verify()?;Ok(())
}
#[test]
fn malformed_hook_returns_a_typed_error() {
    assert!(matches!(ExtraAccountMetaList::unpack(&[0;16]),Err(HookError::InvalidDiscriminator)));
}
