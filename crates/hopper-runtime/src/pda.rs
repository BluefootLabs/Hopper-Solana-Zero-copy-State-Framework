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
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    {
        hopper_native::pda::verify_pda(
            account.as_backend(),
            seeds,
            crate::compat::as_backend_address(program_id),
        )
        .map_err(ProgramError::from)
    }

    #[cfg(not(all(target_os = "solana", feature = "hopper-native-backend")))]
    {
        let expected = create_program_address(seeds, program_id)?;
        if crate::address::address_eq(account.address(), &expected) {
            Ok(())
        } else {
            Err(ProgramError::InvalidSeeds)
        }
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
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    {
        hopper_native::pda::verify_pda_with_bump(
            account.as_backend(),
            seeds,
            bump,
            crate::compat::as_backend_address(program_id),
        )
        .map_err(ProgramError::from)
    }

    #[cfg(not(all(target_os = "solana", feature = "hopper-native-backend")))]
    {
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
}

/// Verify that an account matches a PDA derived from the given seeds.
///
/// **Verify-only approach**: iterates bumps 255→0 using `sol_sha256` only —
/// no `sol_curve_validate_point` needed because we compare each hash directly
/// against the known PDA address. This saves ~159 CU per attempt compared to
/// the standard `find_program_address` approach (sha256+curve_validate).
///
/// Average cost: ~200 CU for bump=255, ~400 CU for bump=254, etc.
/// Standard find_program_address: ~544 CU per attempt.
///
/// Returns the bump seed on success.
#[inline]
pub fn find_and_verify_pda(
    account: &AccountView,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<u8, ProgramError> {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    {
        let backend_pid = crate::compat::as_backend_address(program_id);
        let expected_addr = account.as_backend().address();

        // Build the sha256 input slice array once.
        // Layout: [seed_0, seed_1, ..., seed_n, bump_byte, program_id, PDA_MARKER]
        let n = seeds.len().min(16);
        let mut slices = core::mem::MaybeUninit::<[&[u8]; 19]>::uninit();
        let sptr = slices.as_mut_ptr() as *mut &[u8];
        let mut i = 0;
        while i < n {
            unsafe { sptr.add(i).write(seeds[i]) };
            i += 1;
        }
        // Bump byte slot — the fat pointer is written once; we mutate the
        // backing byte in-place each iteration so sha256 reads the updated
        // value through the same pointer.
        let mut bump_byte = [255u8];
        unsafe {
            sptr.add(n).write(&bump_byte as &[u8]);
            sptr.add(n + 1).write(backend_pid.as_ref());
            sptr.add(n + 2).write(hopper_native::address::PDA_MARKER.as_slice());
        }
        let input = unsafe { core::slice::from_raw_parts(sptr as *const &[u8], n + 3) };

        // Verify-only loop: sha256 per attempt, no curve_validate.
        let mut bump: u16 = 256;
        while bump > 0 {
            bump -= 1;
            bump_byte[0] = bump as u8;

            let mut hash = core::mem::MaybeUninit::<[u8; 32]>::uninit();
            unsafe {
                hopper_native::syscalls::sol_sha256(
                    input as *const _ as *const u8,
                    input.len() as u64,
                    hash.as_mut_ptr() as *mut u8,
                );
            }
            let derived =
                unsafe { &*(hash.as_ptr() as *const hopper_native::address::Address) };
            if hopper_native::address::address_eq(derived, expected_addr) {
                return Ok(bump as u8);
            }
        }

        return Err(ProgramError::InvalidSeeds);
    }

    #[cfg(not(all(target_os = "solana", feature = "hopper-native-backend")))]
    {
        let (expected, bump) = find_program_address(seeds, program_id);
        if crate::address::address_eq(account.address(), &expected) {
            Ok(bump)
        } else {
            Err(ProgramError::InvalidSeeds)
        }
    }
}

/// Verify that a raw address matches a PDA derived from the given seeds.
///
/// Uses the same verify-only sha256 loop as `find_and_verify_pda`.
#[inline]
pub fn verify_pda_strict(
    expected: &Address,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<(), ProgramError> {
    #[cfg(all(target_os = "solana", feature = "hopper-native-backend"))]
    {
        let backend_pid = crate::compat::as_backend_address(program_id);
        let backend_expected =
            unsafe { &*(expected as *const Address as *const hopper_native::address::Address) };

        let n = seeds.len().min(16);
        let mut slices = core::mem::MaybeUninit::<[&[u8]; 19]>::uninit();
        let sptr = slices.as_mut_ptr() as *mut &[u8];
        let mut i = 0;
        while i < n {
            unsafe { sptr.add(i).write(seeds[i]) };
            i += 1;
        }
        let mut bump_byte = [255u8];
        unsafe {
            sptr.add(n).write(&bump_byte as &[u8]);
            sptr.add(n + 1).write(backend_pid.as_ref());
            sptr.add(n + 2).write(hopper_native::address::PDA_MARKER.as_slice());
        }
        let input = unsafe { core::slice::from_raw_parts(sptr as *const &[u8], n + 3) };

        let mut bump: u16 = 256;
        while bump > 0 {
            bump -= 1;
            bump_byte[0] = bump as u8;

            let mut hash = core::mem::MaybeUninit::<[u8; 32]>::uninit();
            unsafe {
                hopper_native::syscalls::sol_sha256(
                    input as *const _ as *const u8,
                    input.len() as u64,
                    hash.as_mut_ptr() as *mut u8,
                );
            }
            let derived =
                unsafe { &*(hash.as_ptr() as *const hopper_native::address::Address) };
            if hopper_native::address::address_eq(derived, backend_expected) {
                return Ok(());
            }
        }

        return Err(ProgramError::InvalidSeeds);
    }

    #[cfg(not(all(target_os = "solana", feature = "hopper-native-backend")))]
    {
        let (derived, _) = find_program_address(seeds, program_id);
        if crate::address::address_eq(&derived, expected) {
            Ok(())
        } else {
            Err(ProgramError::InvalidSeeds)
        }
    }
}
