//! Executable probes of Hopper's safe runtime boundaries.
//!
//! This fixture accepts public test inputs, returns computed bytes, and owns
//! no application authority. Do not use its unrestricted state instruction
//! as a custody or application authorization pattern.
#![cfg_attr(target_os = "solana", no_std)]

use hopper_native::{AccountView, Address, LeU64, ProgramError, ProgramResult};
use hopper_runtime::{crypto, memory};

#[cfg(target_os = "solana")]
mod sbf {
    hopper_native::no_allocator!();
    hopper_native::nostd_panic_handler!();
    hopper_native::fast_entrypoint!(super::process_instruction, 3);
}

fn exact<const N: usize>(bytes: &[u8]) -> Result<&[u8; N], ProgramError> {
    bytes
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)
}

fn returned(bytes: &[u8]) -> ProgramResult {
    hopper_runtime::return_data::try_set_return_data(bytes).map_err(Into::into)
}

pub fn process_instruction<'a>(
    program: &'a Address,
    accounts: &'a [AccountView<'a>],
    data: &[u8],
) -> ProgramResult {
    let (tag, input) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match tag {
        0 => hashes(input),
        1 => memory_probe(input),
        2 => arithmetic(input),
        3 => sysvars(input),
        4 => {
            let input = exact::<3>(input)?;
            let len = u16::from_le_bytes([input[0], input[1]]) as usize;
            let buffer = [input[2]; 1025];
            returned(
                buffer
                    .get(..len)
                    .ok_or(ProgramError::InvalidInstructionData)?,
            )
        }
        5 => crypto_probe(input),
        6 => state_probe(program, accounts, input),
        7 => {
            exact::<0>(input)?;
            crypto::require_top_level()?;
            returned(&[
                crypto::get_stack_height() as u8,
                u8::from(crypto::is_top_level()),
                u8::from(crypto::is_cpi()),
            ])
        }
        8 => {
            let (mode, payload) = input
                .split_first()
                .ok_or(ProgramError::InvalidInstructionData)?;
            match mode {
                0 => hopper::receipts::emit_receipt(payload)?,
                1 => hopper::receipts::emit_tagged_receipt(29, payload)?,
                _ => return Err(ProgramError::InvalidInstructionData),
            }
            Ok(())
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

#[inline(never)]
fn hashes(input: &[u8]) -> ProgramResult {
    let [api, algorithm, count, payload @ ..] = input else {
        return Err(ProgramError::InvalidInstructionData);
    };
    let count = *count as usize;
    if count > 64 || (*api > 1) || (count == 0 && !payload.is_empty()) {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut parts: [&[u8]; 64] = [&[]; 64];
    if count == 1 {
        parts[0] = payload;
    } else if count > 1 {
        let split = usize::from(!payload.is_empty());
        parts[0] = &payload[..split];
        parts[count - 1] = &payload[split..];
    }
    let parts = &parts[..count];
    let digest = match (*api, *algorithm) {
        (0, 0) => hopper_native::hash::sha256(parts)?,
        (0, 1) => hopper_native::hash::keccak256(parts)?,
        #[cfg(feature = "blake3-syscall")]
        (0, 2) => hopper_native::hash::blake3(parts)?,
        (1, 0) => crypto::sha256(parts)?,
        (1, 1) => crypto::keccak256(parts)?,
        #[cfg(feature = "blake3-syscall")]
        (1, 2) => crypto::blake3(parts)?,
        #[cfg(feature = "sha512-syscall")]
        (_, 3) => return returned(&hopper_native::hash::sha512(parts)?),
        _ => return Err(ProgramError::InvalidInstructionData),
    };
    returned(&digest)
}

#[inline(never)]
fn memory_probe(input: &[u8]) -> ProgramResult {
    let [op, a, b, c, value, payload @ ..] = input else {
        return Err(ProgramError::InvalidInstructionData);
    };
    if payload.len() != 64 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let (a, b, c) = (*a as usize, *b as usize, *c as usize);
    let mut buffer = *exact::<64>(payload)?;
    match op {
        0 | 1 => {
            buffer.fill(*value);
            let dst = buffer
                .get_mut(..a)
                .ok_or(ProgramError::InvalidInstructionData)?;
            let src = payload
                .get(..b)
                .ok_or(ProgramError::InvalidInstructionData)?;
            if *op == 0 {
                hopper_native::mem::copy_bytes(dst, src)?;
            } else {
                memory::copy_bytes(dst, src)?;
            }
        }
        2 => memory::move_within(&mut buffer, a, b, c)?,
        3 => memory::fill_bytes(&mut buffer, *value),
        4 => memory::zero_bytes(&mut buffer),
        5 => hopper_native::mem::zero_fill(&mut buffer),
        6 => {
            let left = payload
                .get(..a)
                .ok_or(ProgramError::InvalidInstructionData)?;
            let right = payload
                .get(b..c)
                .ok_or(ProgramError::InvalidInstructionData)?;
            let order = match memory::compare_bytes(left, right) {
                core::cmp::Ordering::Less => 0,
                core::cmp::Ordering::Equal => 1,
                core::cmp::Ordering::Greater => 2,
            };
            return returned(&[
                order,
                u8::from(memory::bytes_eq(left, right)),
                u8::from(hopper_native::mem::bytes_eq(left, right)),
            ]);
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    }
    returned(&buffer)
}

fn arithmetic(input: &[u8]) -> ProgramResult {
    let input = exact::<16>(input)?;
    let a = u64::from_le_bytes(input[..8].try_into().unwrap());
    let b = u64::from_le_bytes(input[8..].try_into().unwrap());
    let wire = LeU64::new(a);
    let values = [
        wire.checked_add(b.into()).map(|v| v.get()),
        wire.checked_sub(b.into()).map(|v| v.get()),
        wire.checked_mul(b.into()).map(|v| v.get()),
        wire.checked_div(b.into()).map(|v| v.get()),
        hopper_native::arith::checked_mul_u64(a, b),
    ];
    let mut output = [0u8; 69];
    for (index, value) in values.into_iter().enumerate() {
        output[index * 9] = u8::from(value.is_some());
        output[index * 9 + 1..index * 9 + 9].copy_from_slice(&value.unwrap_or(0).to_le_bytes());
    }
    for (index, value) in [
        hopper_native::arith::saturating_mul_u64(a, b),
        wire.saturating_add(b.into()).get(),
        wire.saturating_sub(b.into()).get(),
    ]
    .into_iter()
    .enumerate()
    {
        output[45 + index * 8..53 + index * 8].copy_from_slice(&value.to_le_bytes());
    }
    returned(&output)
}

#[inline(never)]
fn sysvars(input: &[u8]) -> ProgramResult {
    use hopper_native::sysvar;
    if input.first() == Some(&2) {
        let input = exact::<50>(input)?;
        let schedule = sysvar::decode_epoch_schedule(input[1..34].try_into().unwrap());
        let slot = u64::from_le_bytes(input[34..42].try_into().unwrap());
        let epoch = u64::from_le_bytes(input[42..50].try_into().unwrap());
        let mut output = [0; 16];
        output[..8].copy_from_slice(&schedule.get_epoch(slot).to_le_bytes());
        output[8..].copy_from_slice(&schedule.get_first_slot_in_epoch(epoch).to_le_bytes());
        return returned(&output);
    }
    let mode = exact::<1>(input)?[0];
    if mode == 1 {
        let mut byte = [0];
        return sysvar::get_sysvar_into(&sysvar::CLOCK_ID, 40, &mut byte);
    }
    if mode != 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let clock = sysvar::get_clock()?;
    let rent = sysvar::get_rent()?;
    let schedule = sysvar::get_epoch_schedule()?;
    let mut output = [0u8; 113];
    sysvar::get_sysvar_into(&sysvar::CLOCK_ID, 0, &mut output[..40])?;
    let clock_words = [
        clock.slot,
        clock.epoch_start_timestamp as u64,
        clock.epoch,
        clock.leader_schedule_epoch,
        clock.unix_timestamp as u64,
    ];
    for (index, word) in clock_words.into_iter().enumerate() {
        if output[index * 8..index * 8 + 8] != word.to_le_bytes() {
            return Err(ProgramError::Custom(7100));
        }
    }
    let values = [
        rent.minimum_balance(0),
        rent.minimum_balance(16),
        rent.minimum_balance(128),
        schedule.slots_per_epoch,
        schedule.leader_schedule_slot_offset,
        schedule.first_normal_epoch,
        schedule.first_normal_slot,
        schedule.get_epoch(clock.slot),
        schedule.get_first_slot_in_epoch(clock.epoch),
    ];
    for (index, word) in values.into_iter().enumerate() {
        output[40 + index * 8..48 + index * 8].copy_from_slice(&word.to_le_bytes());
    }
    output[112] = u8::from(schedule.warmup);
    returned(&output)
}

#[inline(never)]
fn crypto_probe(input: &[u8]) -> ProgramResult {
    let (op, data) = input
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match op {
        0 | 3 => returned(&[u8::from(crypto::curve_validate_point(
            if *op == 0 { 0 } else { 1 },
            exact::<32>(data)?,
        )?)]),
        1 | 2 | 4 => {
            let pair = exact::<64>(data)?;
            let left = exact::<32>(&pair[..32])?;
            let right = exact::<32>(&pair[32..])?;
            let result = if *op == 2 {
                crypto::curve_group_mul(0, left, right)?
            } else {
                crypto::curve_group_add(if *op == 1 { 0 } else { 1 }, left, right)?
            };
            returned(&result)
        }
        5 => {
            let pair = exact::<64>(data)?;
            returned(&crypto::poseidon_bn254_x5(&[&pair[..32], &pair[32..]])?)
        }
        6 => returned(&crypto::alt_bn128_g1_addition_be(data)?),
        7 => returned(&crypto::alt_bn128_g1_multiplication_be(data)?),
        8 => returned(&crypto::alt_bn128_pairing_be(data)?),
        9 => returned(&crypto::alt_bn128_g1_compress_be(exact::<64>(data)?)?),
        10 => returned(&crypto::alt_bn128_g1_decompress_be(exact::<32>(data)?)?),
        #[cfg(feature = "big-mod-exp")]
        11 => {
            let triple = exact::<24>(data)?;
            let mut output = [0; 8];
            crypto::big_mod_exp(&triple[..8], &triple[8..16], &triple[16..], &mut output)?;
            returned(&output)
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

#[inline(never)]
fn state_probe<'a>(
    program: &Address,
    accounts: &'a [AccountView<'a>],
    input: &[u8],
) -> ProgramResult {
    use hopper_native::{
        capability::{OwnedView, SignerView, WritableView},
        lens, BalanceSnapshot, DataFingerprint,
    };
    let input = exact::<9>(input)?;
    let [state, signer, alias] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    SignerView::validate(signer.clone())?;
    OwnedView::validate(state.clone(), program)?;
    WritableView::validate(state.clone())?;
    if state.address() != alias.address() || state.data_len() != 16 {
        return Err(ProgramError::InvalidAccountData);
    }
    let balance = BalanceSnapshot::capture(state);
    let fingerprint = DataFingerprint::capture(state, 16)?;
    match input[0] {
        0 => {
            let value = u64::from_le_bytes(input[1..].try_into().unwrap());
            let prior = lens::read_field_pod::<LeU64>(state, 1)?;
            if alias.try_borrow_mut().is_ok() {
                return Err(ProgramError::Custom(7101));
            }
            drop(prior);
            let mut bytes = state.try_borrow_mut()?;
            bytes[1..9].copy_from_slice(&value.to_le_bytes());
            if alias.try_borrow().is_ok() {
                return Err(ProgramError::Custom(7102));
            }
            drop(bytes);
            if lens::read_field_pod::<LeU64>(alias, 1)?.get() != value {
                return Err(ProgramError::Custom(7103));
            }
        }
        1 => {
            let _guard = state.try_borrow()?;
            return alias.try_borrow_mut().map(|_| ());
        }
        2 => {
            let _ = lens::read_field_pod::<LeU64>(state, 9)?;
            return Err(ProgramError::Custom(7104));
        }
        3 => {
            fingerprint.verify_unchanged(state)?;
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    }
    balance.verify_unchanged(state)?;
    returned(&state.try_borrow()?)
}
