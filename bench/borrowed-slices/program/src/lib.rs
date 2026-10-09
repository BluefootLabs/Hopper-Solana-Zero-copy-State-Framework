//! Bounded batches borrowed from the Solana instruction buffer.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::*;

#[hopper::unit_enum]
pub enum Side {
    Bid = 1,
    Ask = 7,
}

#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Order {
    pub side: OptionByte<EnumByte<Side>>,
    pub amount: WireU64,
}

type Orders<'a> = BoundedSlice<'a, Order, 32>;
type Quantities = hopper::hopper_runtime::BoundedVec<u64, 32>;
type Label = hopper::hopper_runtime::BoundedString<8>;
type Nonce = u16;

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
    hopper::program_entrypoint!(super::process_instruction, 2);
}

pub fn process_instruction<'info>(
    program: &'info Address,
    accounts: &'info [AccountView<'info>],
    data: &'info [u8],
) -> ProgramResult {
    if matches!(data.first(), Some(1 | 2)) {
        return generated::process_instruction(&mut Context::new(program, accounts, data));
    }
    let (mode, input) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    if *mode != 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let (orders, rest) = Orders::parse_prefix(input)?;
    let nonce: [u8; 2] = rest
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    apply(program, accounts, data, orders, u16::from_le_bytes(nonce))
}

#[hopper::program(entrypoint = false, profile = "tiny")]
mod generated {
    use super::*;
    #[instruction(1)]
    pub fn batch(ctx: &mut Context<'_>, orders: Orders<'_>, nonce: u16) -> ProgramResult {
        apply(
            ctx.program_id,
            ctx.accounts(),
            ctx.instruction_data(),
            orders,
            nonce,
        )
    }

    #[instruction(2)]
    pub fn aliases(
        ctx: &mut Context<'_>,
        amounts: Quantities,
        label: Label,
        nonce: Nonce,
    ) -> ProgramResult {
        if label.as_str()? != "é" || nonce != 513 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let total = amounts.as_slice().iter().try_fold(0u64, |sum, amount| {
            sum.checked_add(*amount)
                .ok_or(ProgramError::ArithmeticOverflow)
        })?;
        apply_totals(ctx.program_id, ctx.accounts(), amounts.len(), total)
    }
}

fn apply<'info>(
    program: &'info Address,
    accounts: &'info [AccountView<'info>],
    input: &[u8],
    orders: Orders<'_>,
    nonce: u16,
) -> ProgramResult {
    if orders.as_slice().as_ptr().cast::<u8>() != input[3..].as_ptr() {
        return Err(ProgramError::Custom(6900));
    }
    if nonce != 513 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut total = 0u64;
    for order in orders.as_slice() {
        total = total
            .checked_add(order.amount.get())
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }
    apply_totals(program, accounts, orders.len(), total)
}

fn apply_totals(
    program: &Address,
    accounts: &[AccountView<'_>],
    item_count: usize,
    mut total: u64,
) -> ProgramResult {
    let [state, authority] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if state.read_owner() != *program {
        return Err(ProgramError::IllegalOwner);
    }
    if !state.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut data = state.try_borrow_mut()?;
    let data: &mut [u8; 16] = (&mut *data)
        .try_into()
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let count = u64::from_le_bytes(data[..8].try_into().unwrap())
        .checked_add(item_count as u64)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    total = u64::from_le_bytes(data[8..].try_into().unwrap())
        .checked_add(total)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    data[..8].copy_from_slice(&count.to_le_bytes());
    data[8..].copy_from_slice(&total.to_le_bytes());
    hopper::return_data::set_return_data(data);
    Ok(())
}
