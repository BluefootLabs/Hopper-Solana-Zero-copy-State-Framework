//! Hopper-owned PDA (Program Derived Address) helpers.
//!
//! Provides Hopper-native PDA creation, discovery, and verification using
//! raw Solana syscalls. Only available on BPF targets (`target_os = "solana"`).

use crate::address::Address;
use crate::error::ProgramError;
use crate::AccountView;

#[cfg(target_os = "solana")]
extern "C" {
    fn sol_create_program_address(
        seeds: *const u8,
        seeds_len: u64,
        program_id: *const u8,
        address_out: *mut u8,
    ) -> u64;

    fn sol_try_find_program_address(
        seeds: *const u8,
        seeds_len: u64,
        program_id: *const u8,
        address_out: *mut u8,
        bump_out: *mut u8,
    ) -> u64;
}

/// Create a program-derived address from seeds and a program ID.
///
/// Returns `Err(InvalidSeeds)` if the derived address falls on the
/// ed25519 curve (not a valid PDA).
#[inline]
pub fn create_program_address(
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<Address, ProgramError> {
    #[cfg(target_os = "solana")]
    {
        let mut seed_buf: [u64; 32] = [0; 32]; // MAX_SEEDS * 2
        let num_seeds = seeds.len().min(16);
        let mut i = 0;
        while i < num_seeds {
            seed_buf[i * 2] = seeds[i].as_ptr() as u64;
            seed_buf[i * 2 + 1] = seeds[i].len() as u64;
            i += 1;
        }

        let mut result = Address::default();
        let rc = unsafe {
            sol_create_program_address(
                seed_buf.as_ptr() as *const u8,
                num_seeds as u64,
                program_id.as_ref().as_ptr(),
                result.as_mut().as_mut_ptr(),
            )
        };
        if rc == 0 {
            Ok(result)
        } else {
            Err(ProgramError::InvalidSeeds)
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id);
        Err(ProgramError::InvalidSeeds)
    }
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
        let mut seed_buf: [u64; 32] = [0; 32];
        let num_seeds = seeds.len().min(16);
        let mut i = 0;
        while i < num_seeds {
            seed_buf[i * 2] = seeds[i].as_ptr() as u64;
            seed_buf[i * 2 + 1] = seeds[i].len() as u64;
            i += 1;
        }

        let mut result = Address::default();
        let mut bump: u8 = 0;
        let rc = unsafe {
            sol_try_find_program_address(
                seed_buf.as_ptr() as *const u8,
                num_seeds as u64,
                program_id.as_ref().as_ptr(),
                result.as_mut().as_mut_ptr(),
                &mut bump as *mut u8,
            )
        };
        if rc == 0 {
            (result, bump)
        } else {
            (Address::default(), 0)
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id);
        (Address::default(), 0)
    }
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
