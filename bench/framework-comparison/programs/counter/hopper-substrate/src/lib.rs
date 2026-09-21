//! Hopper substrate half of the pina counter comparison.
//!
//! Byte-for-byte the contract of pina's hand-written pinocchio fixture
//! (`benchmarks/framework-comparison/programs/counter/pinocchio`):
//!
//! - PDA seeded by `b"counter" + authority`, bump supplied in the
//!   `initialize` instruction data (`initializeTakesBump: true`);
//! - a 10-byte account `[disc = 1][bump][count u64 LE]`;
//! - `initialize` = accounts `[authority(sw), counter(w), system]`, one
//!   plain `CreateAccount` CPI signed with the PDA seeds, then the state
//!   write;
//! - `increment` = accounts `[authority(s), counter(w)]`, re-derives the
//!   PDA from the stored bump (`create_program_address`, the ~1.4k CU
//!   that Quasar skips) and adds one with a checked add.
//!
//! The account layout is a Hopper compact state: `#[hopper::state(compact,
//! disc = 1)]` puts the body straight after a one-byte discriminator, no
//! 16-byte header, which is exactly pina's `BUMP_THEN_COUNT` layout. The
//! `#[bump]` marker publishes the bump offset for `bump = stored` contexts;
//! this fixture reads it by hand to stay on the raw path.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::pda::create_program_address;
use hopper::prelude::WireU64;
use hopper::prelude::{AccountView, Address, ProgramError, ProgramResult};
use hopper::substrate::sysvar::Rent;
use hopper::substrate::{Seed, Signer};
use hopper::system::CreateAccount;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// `initialize` is the widest instruction at three accounts.
#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction, 3);

/// `[disc:1][bump:1][count:8]`, the pina `BUMP_THEN_COUNT` layout.
pub const COUNTER_SPACE: u64 = 10;
pub const SEED_COUNTER: &[u8] = b"counter";
pub const ACCOUNT_DISCRIMINATOR: u8 = 1;
const SYSTEM_PROGRAM: Address = Address::new_from_array([0u8; 32]);

/// Compact counter body. Byte 0 of the account is the discriminator the
/// macro writes; `bump` lands at offset 1 and `count` at offset 2.
#[derive(Clone, Copy, Debug, Default)]
#[hopper::state(compact, disc = 1)]
#[repr(C)]
pub struct Counter {
    #[bump]
    pub bump: u8,
    pub count: WireU64,
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let (&discriminator, rest) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match discriminator {
        0 => initialize(program_id, accounts, rest),
        1 => increment(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn initialize(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let [authority, counter, system_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let &bump = data.first().ok_or(ProgramError::InvalidInstructionData)?;

    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !counter.is_data_empty() {
        return Err(ProgramError::AccountAlreadyInitialized);
    }
    if system_program.address() != &SYSTEM_PROGRAM {
        return Err(ProgramError::InvalidArgument);
    }

    let bump_seed = [bump];
    let seeds = [
        Seed::from(SEED_COUNTER),
        Seed::from(authority.address().as_array()),
        Seed::from(&bump_seed[..]),
    ];
    CreateAccount {
        from: authority,
        to: counter,
        lamports: Rent::get()?.minimum_balance(COUNTER_SPACE as usize),
        space: COUNTER_SPACE,
        owner: program_id,
    }
    .invoke_signed(&[Signer::from(&seeds)])?;

    // `init_compact` stamps the discriminator and zeroes the body; the
    // typed view then stores the bump the caller proved.
    counter.init_compact::<Counter>()?;
    let mut state = counter.load_compact_mut::<Counter>()?;
    state.bump = bump;
    state.count = WireU64::ZERO;

    hopper::substrate::log::log("Counter initialized");
    Ok(())
}

fn increment(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [authority, counter] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !counter.owned_by(program_id) {
        return Err(ProgramError::IncorrectProgramId);
    }

    // `load_compact_mut` validates the exact length and the discriminator
    // byte, the same two checks the pinocchio fixture writes by hand.
    let mut state = counter.load_compact_mut::<Counter>()?;

    let derived = create_program_address(
        &[SEED_COUNTER, authority.address().as_array(), &[state.bump]],
        program_id,
    )
    .map_err(|_| ProgramError::InvalidSeeds)?;
    if counter.address() != &derived {
        return Err(ProgramError::InvalidSeeds);
    }

    state.count.checked_add_assign(1)?;

    hopper::substrate::log::log("Counter incremented");
    Ok(())
}
