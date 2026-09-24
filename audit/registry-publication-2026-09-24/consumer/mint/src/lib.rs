//! Compiled fixture for mint-plan allocation, initialization and refusal tests.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::hopper_runtime::token_mint::{
    InitializeMint2, MintConfig, MintExtension as E, MintPlan, MintProgram,
};
use hopper::prelude::{AccountView, Address, ProgramError, ProgramResult};

#[derive(hopper::Accounts)]
pub struct ExtensionOnly<'info> {
    #[account(extensions::non_transferable)]
    pub mint: hopper::prelude::UncheckedAccount<'info>,
}

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
    hopper::program_entrypoint!(super::process_instruction, 4);
}

pub fn process_instruction<'info>(
    _program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    data: &'info [u8],
) -> ProgramResult {
    let [payer, mint, token, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let &[program, mask, operation, space_delta, bump] = data else {
        return Err(ProgramError::InvalidInstructionData);
    };
    let program = match program {
        0 => MintProgram::Legacy,
        1 => MintProgram::Token2022,
        _ => return Err(ProgramError::InvalidInstructionData),
    };
    if token.address() != program.address()
        || !token.executable()
        || system.address() != &hopper::hopper_runtime::system::SYSTEM_PROGRAM_ID
        || !system.executable()
    {
        return Err(ProgramError::IncorrectProgramId);
    }
    if mask > 63 {
        return Err(ProgramError::InvalidInstructionData);
    }
    if operation == 4 {
        let mut context = hopper::prelude::Context::new(_program_id, &accounts[1..2], data);
        ExtensionOnly::bind(&mut context)?;
        return Ok(());
    }
    let config = MintConfig {
        decimals: 9,
        mint_authority: payer.address(),
        freeze_authority: Some(payer.address()),
    };
    let all = [
        E::TransferFeeConfig {
            authority: Some(payer.address()),
            withdraw_authority: Some(payer.address()),
            basis_points: 250,
            maximum_fee: 1_000_000,
        },
        E::MintCloseAuthority(Some(payer.address())),
        E::NonTransferable,
        E::PermanentDelegate(payer.address()),
        E::TransferHook {
            authority: Some(payer.address()),
            program_id: Some(payer.address()),
        },
        E::MetadataPointer {
            authority: Some(payer.address()),
            metadata_address: Some(mint.address()),
        },
    ];
    let mut extensions = [E::NonTransferable; 6];
    let mut count = 0;
    for (i, extension) in all.iter().enumerate() {
        if mask & (1 << i) != 0 {
            extensions[count] = *extension;
            count += 1;
        }
    }
    let plan = MintPlan::new(program, config, &extensions[..count])?;
    let requested = plan
        .space()
        .checked_add_signed(space_delta as i8 as isize)
        .ok_or(ProgramError::InvalidArgument)?;
    plan.check_space(requested)?;
    match operation {
        0 => plan.create(payer, mint, &[]),
        1 => plan.initialize(mint),
        // Deliberately bypass plan preflight so tests also observe canonical
        // processor refusal of uninitialized or incorrectly sized extensions.
        2 => InitializeMint2 {
            mint,
            program,
            config,
        }
        .invoke(),
        3 => {
            use hopper::hopper_runtime::instruction::{Seed, Signer};
            let bump = [bump];
            let seeds = [
                Seed::from(b"mint".as_slice()),
                Seed::from(payer.address().as_array()),
                Seed::from(bump.as_slice()),
            ];
            plan.create(payer, mint, &[Signer::from(&seeds)])
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
