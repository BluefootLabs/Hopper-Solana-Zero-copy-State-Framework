//! Hopper-owned PDA ergonomics on top of the active backend substrate.

use crate::address::Address;
use crate::error::ProgramError;
use crate::AccountView;

/// Create a program-derived address from seeds and a program ID.
///
/// Returns `Err(InvalidSeeds)` if the derived address falls on the
/// ed25519 curve (not a valid PDA).
#[inline]
pub fn create_program_address(
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<Address, ProgramError> {
    crate::compat::create_program_address(seeds, program_id)
}

/// Find a program-derived address and its bump seed.
///
/// Iterates bump seeds 255..=0 until a valid PDA is found.
#[inline]
pub fn find_program_address(
    seeds: &[&[u8]],
    program_id: &Address,
) -> (Address, u8) {
    #[cfg(target_os = "solana")]
    {
        crate::compat::find_program_address(seeds, program_id)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id);
        (Address::default(), 0)
    }
}

/// Hopper-facing alias for PDA derivation.
#[inline(always)]
pub fn derive(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
    find_program_address(seeds, program_id)
}

/// Verify that an account's address matches a PDA derived from the given seeds.
#[inline]
pub fn verify_pda(
    account: &AccountView,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<(), ProgramError> {
    let expected = create_program_address(seeds, program_id)?;
    if crate::address::address_eq(account.address(), &expected) {
        Ok(())
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}

/// Verify a PDA with an explicit bump seed appended to the seeds.
#[inline]
pub fn verify_pda_with_bump(
    account: &AccountView,
    seeds: &[&[u8]],
    bump: u8,
    program_id: &Address,
) -> Result<(), ProgramError> {
    let mut full_seeds: [&[u8]; 17] = [&[]; 17];
    let num = seeds.len().min(15);
    let mut i = 0;
    while i < num {
        full_seeds[i] = seeds[i];
        i += 1;
    }
    let bump_bytes = [bump];
    full_seeds[num] = &bump_bytes;

    let expected = create_program_address(&full_seeds[..num + 1], program_id)?;
    if crate::address::address_eq(account.address(), &expected) {
        Ok(())
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}

/// Verify that a raw address matches a PDA derived from the given seeds.
#[inline]
pub fn verify_pda_strict(
    expected: &Address,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<(), ProgramError> {
    let (derived, _) = find_program_address(seeds, program_id);
    if crate::address::address_eq(&derived, expected) {
        Ok(())
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}
