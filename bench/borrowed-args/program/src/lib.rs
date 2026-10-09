//! Checked borrowed instruction prefixes, exercised inside the Solana VM.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::*;

#[hopper::unit_enum]
pub enum Mode {
    Open = 1,
    Closed = 7,
}

type OptionalMode = OptionByte<EnumByte<Mode>>;

#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Modes {
    pub values: [OptionalMode; 2],
}

#[hopper::args(tail)]
#[repr(C)]
pub struct Update {
    pub modes: Modes,
    pub amount: WireU64,
}

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
    hopper::program_entrypoint!(super::process_instruction, 2);
}

pub fn process_instruction<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    data: &'info [u8],
) -> ProgramResult {
    if matches!(data.first(), Some(2 | 3)) {
        return generated::process_instruction(&mut Context::new(program_id, accounts, data));
    }
    let (mode, payload) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    let (args, tail) = match mode {
        0 => (Update::parse_exact_checked(payload)?, &[][..]),
        1 => Update::parse_with_tail_checked(payload)?,
        _ => return Err(ProgramError::InvalidInstructionData),
    };
    apply_update(program_id, accounts, data, args, tail)
}

#[hopper::program(entrypoint = false, profile = "tiny")]
mod generated {
    use super::*;

    #[instruction(2)]
    pub fn exact(ctx: &mut Context<'_>, args: &Update) -> ProgramResult {
        apply_update(
            ctx.program_id,
            ctx.accounts(),
            ctx.instruction_data(),
            args,
            &[],
        )
    }

    #[instruction(3)]
    pub fn tail(ctx: &mut Context<'_>, args: &Update, tail: &[u8]) -> ProgramResult {
        apply_update(
            ctx.program_id,
            ctx.accounts(),
            ctx.instruction_data(),
            args,
            tail,
        )
    }
}

fn apply_update<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    data: &[u8],
    args: &Update,
    tail: &[u8],
) -> ProgramResult {
    let payload = &data[1..];
    // Verify the prefix and suffix still refer to the original instruction.
    if !core::ptr::eq(args as *const Update, payload.as_ptr().cast())
        || (matches!(data[0], 1 | 3)
            && !core::ptr::eq(tail.as_ptr(), payload[Update::PACKED_SIZE..].as_ptr()))
    {
        return Err(ProgramError::Custom(6900));
    }
    // The typed parser checks the prefix. The application owns tail semantics.
    if tail.len() > 32 || core::str::from_utf8(tail).is_err() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [state, authority] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if state.read_owner() != *program_id {
        return Err(ProgramError::IllegalOwner);
    }
    if !state.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut bytes = state.try_borrow_mut()?;
    let bytes: &mut [u8; 16] = (&mut *bytes)
        .try_into()
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let count = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let total = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    let count = count
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let total = total
        .checked_add(args.amount.get())
        .ok_or(ProgramError::ArithmeticOverflow)?;
    bytes[..8].copy_from_slice(&count.to_le_bytes());
    bytes[8..].copy_from_slice(&total.to_le_bytes());
    hopper::return_data::set_return_data(bytes);
    Ok(())
}
