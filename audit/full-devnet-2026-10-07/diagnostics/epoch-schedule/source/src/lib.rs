#![no_std]
#![allow(unexpected_cfgs)]
use hopper_native::{Address, AccountView, ProgramResult, ProgramError, sysvar};
hopper_native::no_allocator!();
hopper_native::nostd_panic_handler!();
hopper_native::program_entrypoint!(process, 0);
fn process(_: &Address, _: &[AccountView], data: &[u8]) -> ProgramResult {
    if !data.is_empty() { return Err(ProgramError::InvalidInstructionData); }
    let clock = sysvar::get_clock()?;
    let account = sysvar::get_epoch_schedule()?;
    let mut runtime = core::mem::MaybeUninit::<sysvar::EpochSchedule>::uninit();
    // SAFETY: EpochSchedule has the asserted Solana C ABI. The syscall writes
    // the complete valid struct on success, before assume_init is reached.
    let rc = unsafe { hopper_native::syscalls::sol_get_epoch_schedule_sysvar(runtime.as_mut_ptr().cast()) };
    if rc != 0 { return Err(ProgramError::UnsupportedSysvar); }
    // SAFETY: The successful syscall initialized every field above.
    let runtime = unsafe { runtime.assume_init() };
    let values = [clock.slot, clock.epoch, account.slots_per_epoch, account.leader_schedule_slot_offset, account.first_normal_epoch, account.first_normal_slot,
        runtime.slots_per_epoch, runtime.leader_schedule_slot_offset, runtime.first_normal_epoch, runtime.first_normal_slot];
    let mut output=[0u8;82];
    for (i,v) in values.iter().enumerate() { output[i*8..i*8+8].copy_from_slice(&v.to_le_bytes()); }
    output[80]=u8::from(account.warmup);output[81]=u8::from(runtime.warmup);
    hopper_native::cpi::set_return_data(&output);
    Ok(())
}
