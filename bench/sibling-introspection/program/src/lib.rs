#![cfg_attr(target_os = "solana", no_std)]
use hopper_native::instruction::{InstructionAccount, InstructionView};
use hopper_native::{introspect as native, AccountView, Address, ProgramError, ProgramResult};
use hopper_runtime::crypto as runtime;

#[cfg(target_os = "solana")]
hopper_native::program_entrypoint!(process_instruction, 2);
#[cfg(target_os = "solana")]
hopper_native::no_allocator!();
#[cfg(target_os = "solana")]
hopper_native::nostd_panic_handler!();

fn check(ok: bool, code: u32) -> ProgramResult {
    if ok {
        Ok(())
    } else {
        Err(ProgramError::Custom(code))
    }
}

fn call(program: &Address, accounts: &[AccountView], data: &[u8], empty: bool) -> ProgramResult {
    let [payer, target] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if empty {
        hopper_native::cpi::invoke::<1>(
            &InstructionView {
                program_id: program,
                accounts: &[],
                data,
            },
            &[target],
        )
    } else {
        let metas = [
            InstructionAccount::readonly_signer(payer.address()),
            InstructionAccount::readonly(target.address()),
        ];
        hopper_native::cpi::invoke::<2>(
            &InstructionView {
                program_id: program,
                accounts: &metas,
                data,
            },
            &[payer, target],
        )
    }
}

#[inline(never)]
fn native_owned(program: &Address) -> ProgramResult {
    let ix = native::get_processed_instruction(0).ok_or(ProgramError::Custom(101))?;
    check(
        ix.program_id == *program
            && ix.data_len == 2
            && ix.data[..2] == [0, 42]
            && ix.accounts_len == 2,
        111,
    )
}

#[inline(never)]
fn runtime_owned(program: &Address) -> ProgramResult {
    let ix = runtime::get_processed_instruction(0).ok_or(ProgramError::Custom(102))?;
    check(
        ix.program_id.to_bytes() == program.to_bytes()
            && ix.data_len == 2
            && ix.data[..2] == [0, 42]
            && ix.accounts_len == 2,
        112,
    )
}

#[inline(never)]
fn runtime_bounded(program: &Address) -> ProgramResult {
    let ix = runtime::get_processed_instruction_data::<16>(0).ok_or(ProgramError::Custom(103))?;
    check(
        ix.program_id.to_bytes() == program.to_bytes()
            && ix.data_len == 2
            && ix.data[..2] == [0, 42],
        113,
    )
}

#[inline(never)]
fn missing() -> ProgramResult {
    check(native::get_processed_instruction(0).is_none(), 106)?;
    check(runtime::get_processed_instruction(0).is_none(), 116)?;
    check(
        runtime::get_processed_instruction_data::<16>(0).is_none(),
        126,
    )?;
    #[cfg(not(feature = "baseline"))]
    check(
        native::get_processed_instruction_into(0, &mut [], &mut [])?.is_none(),
        136,
    )?;
    Ok(())
}

#[cfg(not(feature = "baseline"))]
#[inline(never)]
fn buffered(program: &Address, accounts: &[AccountView], tag: u8) -> ProgramResult {
    let mut data = [0xa5; 16];
    let mut metas =
        core::array::from_fn::<_, 3, _>(|_| native::ProcessedInstructionAccount::default());
    let ix = native::get_processed_instruction_into(0, &mut data, &mut metas)?
        .ok_or(ProgramError::Custom(104))?;
    check(
        ix.program_id == *program && ix.data == [0, 42] && ix.accounts.len() == 2,
        114,
    )?;
    check(
        ix.accounts[0].address == *accounts[0].address()
            && ix.accounts[0].is_signer
            && !ix.accounts[0].is_writable,
        124,
    )?;
    check(
        ix.accounts[1].address == *program
            && !ix.accounts[1].is_signer
            && !ix.accounts[1].is_writable,
        134,
    )?;
    check(
        data[2..] == [0xa5; 14] && metas[2] == native::ProcessedInstructionAccount::default(),
        144,
    )?;
    if tag == 10 {
        let previous = native::get_processed_instruction_into(1, &mut data, &mut metas)?
            .ok_or(ProgramError::Custom(110))?;
        check(previous.data == [0, 41], 120)?;
        check(
            native::get_processed_instruction_into(2, &mut data, &mut metas)?.is_none(),
            130,
        )?;
    }
    Ok(())
}

#[cfg(not(feature = "baseline"))]
#[inline(never)]
fn large(program: &Address) -> ProgramResult {
    check(native::get_processed_instruction(0).is_none(), 109)?;
    check(
        runtime::get_processed_instruction_data::<16>(0).is_none(),
        119,
    )?;
    let mut data = [0; 1300];
    let mut metas =
        core::array::from_fn::<_, 2, _>(|_| native::ProcessedInstructionAccount::default());
    let ix = runtime::get_processed_instruction_into(0, &mut data, &mut metas)
        .map_err(|_| ProgramError::Custom(129))?
        .ok_or(ProgramError::Custom(139))?;
    check(
        ix.program_id.to_bytes() == program.to_bytes()
            && ix.data.len() == 1300
            && ix.data[0] == 0
            && ix.data[1..].iter().all(|&b| b == 42)
            && ix.accounts.len() == 2,
        149,
    )
}

pub fn process_instruction(
    program: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let Some(&tag) = data.first() else {
        return Ok(());
    };
    match tag {
        0 => Ok(()),
        20 if data.len() == 2 => {
            let case = data[1];
            match case {
                6 => {}
                7 => call(program, accounts, &[], true)?,
                9 => {
                    let mut leaf = [42; 1300];
                    leaf[0] = 0;
                    call(program, accounts, &leaf, false)?;
                }
                10 => {
                    call(program, accounts, &[0, 41], false)?;
                    call(program, accounts, &[0, 42], false)?;
                }
                _ => call(program, accounts, &[0, 42], false)?,
            }
            call(program, accounts, &[case], false)
        }
        1 => native_owned(program),
        2 => runtime_owned(program),
        3 => runtime_bounded(program),
        6 => missing(),
        #[cfg(not(feature = "baseline"))]
        4 | 10 => buffered(program, accounts, tag),
        #[cfg(not(feature = "baseline"))]
        5 | 11 => {
            let mut data = [0; 2];
            let mut metas =
                core::array::from_fn::<_, 2, _>(|_| native::ProcessedInstructionAccount::default());
            let (bytes, records) = if tag == 5 {
                (&mut data[..1], &mut metas[..])
            } else {
                (&mut data[..], &mut metas[..1])
            };
            native::get_processed_instruction_into(0, bytes, records).map(|_| ())
        }
        #[cfg(not(feature = "baseline"))]
        7 => {
            let ix = native::get_processed_instruction_into(0, &mut [], &mut [])?
                .ok_or(ProgramError::Custom(107))?;
            check(
                ix.program_id == *program && ix.data.is_empty() && ix.accounts.is_empty(),
                117,
            )
        }
        #[cfg(not(feature = "baseline"))]
        8 => {
            call(program, accounts, &[0, 99], true)?;
            buffered(program, accounts, tag)
        }
        #[cfg(not(feature = "baseline"))]
        9 => large(program),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
