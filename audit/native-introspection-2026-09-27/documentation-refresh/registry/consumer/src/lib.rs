use hopper_native::{Address as NativeAddress, ProgramError as NativeError};
use hopper_native::introspect::{get_processed_instruction_into, ProcessedInstructionAccount};
pub fn prior(expected: &NativeAddress) -> Result<(), NativeError> {
    let mut data=[0;128];
    let mut accounts: [ProcessedInstructionAccount;4]=core::array::from_fn(|_| ProcessedInstructionAccount::default());
    let previous=get_processed_instruction_into(0,&mut data,&mut accounts)?.ok_or(NativeError::InvalidArgument)?;
    if previous.program_id != *expected { return Err(NativeError::IncorrectProgramId); }
    Ok(())
}
pub fn runtime_prior() -> hopper_runtime::ProgramResult {
    let mut data=[0;128];
    let mut accounts: [hopper_runtime::crypto::ProcessedInstructionAccount;4]=core::array::from_fn(|_| Default::default());
    if let Some(ix)=hopper_runtime::crypto::get_processed_instruction_into(0,&mut data,&mut accounts)? {
        if let Some(meta)=ix.accounts.first() { let _: hopper_runtime::Address=meta.address.clone().into(); }
    }
    Ok(())
}
use hopper_runtime::{AccountView, Address, ProgramResult};
use hopper_solana::{interface::interface_transfer_checked_with_program,transfer::TokenTransferSnapshot};
pub fn payout<'a>(source: &'a AccountView<'a>, destination: &'a AccountView<'a>, mint: &'a AccountView<'a>, authority: &'a AccountView<'a>, token: &'a AccountView<'a>, configured: &Address) -> ProgramResult {
    let snapshot=TokenTransferSnapshot::capture(source,destination,configured,100,99)?;
    interface_transfer_checked_with_program(source,mint,destination,authority,token,100,6)?;
    snapshot.verify()?; Ok(())
}
use hopper::prelude::*;
#[derive(Accounts)]
pub struct Authority<'info> { pub authority: Signer<'info> }
