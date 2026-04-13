//! PDA (Program Derived Address) helpers.
//!
//! Direct syscall-based PDA creation and derivation. No external dependencies.

use crate::account_view::AccountView;
use crate::address::{Address, MAX_SEEDS};
use crate::error::ProgramError;

#[cfg(target_os = "solana")]
const CURVE25519_EDWARDS: u64 = 0;
#[cfg(target_os = "solana")]
const PDA_MARKER_BYTES: &[u8; 21] = crate::address::PDA_MARKER;

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
        // Build the seeds array in the format expected by the syscall:
        // each seed is a (ptr, len) pair packed as two u64 values.
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
            crate::syscalls::sol_create_program_address(
                seed_buf.as_ptr() as *const u8,
                num_seeds as u64,
                program_id.as_array().as_ptr(),
                result.0.as_mut_ptr(),
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
        based_try_find_program_address(seeds, program_id).unwrap_or((Address::default(), 0))
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id);
        (Address::default(), 0)
    }
}

/// Verify that an expected address matches the PDA hash for the provided seeds.
///
/// The seeds slice must already include the bump byte.
#[inline]
pub fn verify_program_address(
    seeds: &[&[u8]],
    program_id: &Address,
    expected: &Address,
) -> Result<(), ProgramError> {
    if seeds.len() > MAX_SEEDS + 1 {
        return Err(ProgramError::InvalidSeeds);
    }

    #[cfg(target_os = "solana")]
    {
        let n = seeds.len();
        let mut slices = core::mem::MaybeUninit::<[&[u8]; MAX_SEEDS + 3]>::uninit();
        let slice_ptr = slices.as_mut_ptr() as *mut &[u8];

        let mut i = 0;
        while i < n {
            unsafe { slice_ptr.add(i).write(seeds[i]) };
            i += 1;
        }
        unsafe {
            slice_ptr.add(n).write(program_id.as_ref());
            slice_ptr.add(n + 1).write(PDA_MARKER_BYTES.as_slice());
        }

        let input = unsafe { core::slice::from_raw_parts(slice_ptr, n + 2) };
        let mut hash = core::mem::MaybeUninit::<[u8; 32]>::uninit();

        unsafe {
            crate::syscalls::sol_sha256(
                input as *const _ as *const u8,
                input.len() as u64,
                hash.as_mut_ptr() as *mut u8,
            );
        }

        let derived = unsafe { &*(hash.as_ptr() as *const Address) };
        if derived == expected {
            Ok(())
        } else {
            Err(ProgramError::InvalidSeeds)
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id, expected);
        Err(ProgramError::InvalidSeeds)
    }
}

/// Find a valid PDA by hashing seeds directly and checking curve validity.
///
/// This avoids the `sol_try_find_program_address` syscall and substantially
/// reduces the per-attempt CU cost on SBF.
#[inline]
pub fn based_try_find_program_address(
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<(Address, u8), ProgramError> {
    if seeds.len() > MAX_SEEDS {
        return Err(ProgramError::InvalidSeeds);
    }

    #[cfg(target_os = "solana")]
    {
        let n = seeds.len();
        let mut slices = core::mem::MaybeUninit::<[&[u8]; MAX_SEEDS + 3]>::uninit();
        let slice_ptr = slices.as_mut_ptr() as *mut &[u8];

        let mut i = 0;
        while i < n {
            unsafe { slice_ptr.add(i).write(seeds[i]) };
            i += 1;
        }
        unsafe {
            slice_ptr.add(n + 1).write(program_id.as_ref());
            slice_ptr.add(n + 2).write(PDA_MARKER_BYTES.as_slice());
        }

        let mut bump_seed = [u8::MAX];
        let bump_ptr = bump_seed.as_mut_ptr();
        unsafe { slice_ptr.add(n).write(core::slice::from_raw_parts(bump_ptr, 1)) };

        let input = unsafe { core::slice::from_raw_parts(slice_ptr, n + 3) };
        let mut hash = core::mem::MaybeUninit::<[u8; 32]>::uninit();
        let mut bump: u64 = u8::MAX as u64;

        loop {
            unsafe { bump_ptr.write(bump as u8) };

            unsafe {
                crate::syscalls::sol_sha256(
                    input as *const _ as *const u8,
                    input.len() as u64,
                    hash.as_mut_ptr() as *mut u8,
                );
            }

            let on_curve = unsafe {
                crate::syscalls::sol_curve_validate_point(
                    CURVE25519_EDWARDS,
                    hash.as_ptr() as *const u8,
                    core::ptr::null_mut(),
                )
            };

            if on_curve != 0 {
                return Ok((Address::new_from_array(unsafe { hash.assume_init() }), bump as u8));
            }

            if bump == 0 {
                break;
            }
            bump -= 1;
        }

        Err(ProgramError::InvalidSeeds)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id);
        Err(ProgramError::InvalidSeeds)
    }
}

/// Verify that an account's address matches a PDA derived from the given seeds.
///
/// Returns `Ok(())` if the account address matches the derived PDA,
/// or `Err(InvalidSeeds)` if it does not.
#[inline]
pub fn verify_pda(
    account: &AccountView,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<(), ProgramError> {
    let expected = create_program_address(seeds, program_id)?;
    if account.address() == &expected {
        Ok(())
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}

/// Verify a PDA with an explicit bump seed appended to the seeds.
///
/// Appends `&[bump]` to the end of the seed list before deriving.
#[inline]
pub fn verify_pda_with_bump(
    account: &AccountView,
    seeds: &[&[u8]],
    bump: u8,
    program_id: &Address,
) -> Result<(), ProgramError> {
    // Build a seed list with the bump appended.
    // We use a stack-allocated array since MAX_SEEDS is 16.
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
    if account.address() == &expected {
        Ok(())
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}

/// Verify that an address matches a PDA derived from the given seeds.
///
/// Unlike `verify_pda` which takes an `AccountView`, this accepts a raw
/// `Address` reference directly. Useful when validating addresses outside
/// of the account parsing flow (e.g. instruction data, cross-program reads).
///
/// Returns `Ok(())` if the address matches the derived PDA,
/// or `Err(InvalidSeeds)` if it does not.
#[inline]
pub fn verify_pda_strict(
    expected: &Address,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<(), ProgramError> {
    let (derived, _) = find_program_address(seeds, program_id);
    if &derived == expected {
        Ok(())
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}
