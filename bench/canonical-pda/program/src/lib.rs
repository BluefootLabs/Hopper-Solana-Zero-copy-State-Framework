//! Compare a canonical runtime PDA search with a macro-expanded constant.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::{AccountView, Address, Context, ProgramError, ProgramResult};

hopper::declare_id!("8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH");
pub const CONFIG: (Address, u8) = hopper::canonical_pda!(
    "8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH",
    [b"config", b"v1"]
);

#[derive(hopper::Accounts)]
pub struct Inferred<'info> {
    #[account(seeds = [b"config", b"v1"], bump)]
    pub config: hopper::prelude::UncheckedAccount<'info>,
}

mod config;
use config::Config;

#[derive(hopper::Accounts)]
pub struct TypedInferred<'info> {
    #[account(seeds = [b"config", b"v1"], bump)]
    pub config: hopper::prelude::Account<'info, Config>,
}

fn config_seeds() -> [&'static [u8]; 2] {
    [b"config", b"v1"]
}

#[derive(hopper::Accounts)]
pub struct TypedSeeds<'info> {
    #[account(seeds_fn = config_seeds())]
    pub config: hopper::prelude::Account<'info, Config>,
}

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
    hopper::program_entrypoint!(super::process_instruction, 1);
}

pub fn process_instruction<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    data: &'info [u8],
) -> ProgramResult {
    if *program_id != ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    let &[mode, supplied_bump] = data else {
        return Err(ProgramError::InvalidInstructionData);
    };
    let account = accounts.first().ok_or(ProgramError::NotEnoughAccountKeys)?;
    let canonical_bump = match mode {
        0 => hopper::pda::find_canonical_bump_checked(
            &[b"config", b"v1"],
            program_id,
            account.address(),
        )?,
        1 => {
            if *account.address() != CONFIG.0 {
                return Err(ProgramError::InvalidSeeds);
            }
            CONFIG.1
        }
        2 | 3 => {
            let mut context = Context::new(program_id, accounts, data);
            // Mode 3 explicitly repeats validation as a comparison. Mode 2
            // exercises the generated bind and its retained validated bump.
            if mode == 3 {
                Inferred::validate(&context)?;
            }
            let bound = Inferred::bind(&mut context)?;
            bound.bumps().config
        }
        4 => {
            let context = Context::new(program_id, accounts, data);
            Inferred::validate_config::<0>(&context)?;
            CONFIG.1
        }
        5 => {
            let mut context = Context::new(program_id, accounts, data);
            TypedInferred::bind(&mut context)?.bumps().config
        }
        6 => {
            let mut context = Context::new(program_id, accounts, data);
            TypedSeeds::bind(&mut context)?.bumps().config
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    };
    if supplied_bump != canonical_bump {
        return Err(ProgramError::InvalidSeeds);
    }
    Ok(())
}
