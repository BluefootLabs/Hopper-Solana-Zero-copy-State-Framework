//! Hopper-owned PDA ergonomics on top of the native runtime boundary.

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
    crate::native_boundary::create_program_address(seeds, program_id)
}

/// Find a program-derived address and its bump seed.
///
/// Iterates bump seeds 255..=0 until a valid PDA is found.
///
/// # Panics
///
/// Panics if no viable bump exists (matching upstream
/// `Pubkey::find_program_address`), and on non-SVM hosts, where the sha256
/// syscall is unavailable. The earlier host fallback silently returned
/// `(Address::default(), 0)`, the all-zero System Program address; which
/// made host tests "pass" derivation while comparing against a meaningless
/// key. Host tests should exercise PDA paths through the SVM harness.
#[inline]
pub fn find_program_address(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
    #[cfg(target_os = "solana")]
    {
        crate::native_boundary::find_program_address(seeds, program_id)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id);
        panic!(
            "hopper: find_program_address requires the SVM sha256 syscall; \
             run PDA paths under the SVM harness (target_os = \"solana\")"
        );
    }
}

/// Hopper-facing alias for PDA derivation.
#[inline(always)]
pub fn derive(seeds: &[&[u8]], program_id: &Address) -> (Address, u8) {
    find_program_address(seeds, program_id)
}

/// A program-derived address evaluated at compile time.
///
/// For seeds that are all literals (a `b"config"` singleton, a
/// `b"vault"` + declared-id pair), the address is a constant of the
/// program, so there is nothing to hash on chain: declare it once and
/// check the account with `#[account(address = CONFIG)]`, a 32-byte
/// compare instead of a `sol_sha256` (about 150 CU) or a
/// `create_program_address` syscall (1,500 CU) on every instruction that
/// touches the account. [`crate::const_pda!`] is the same call with the
/// seed list spelled inline.
///
/// `bump` must be the canonical bump `find_program_address` returns (the
/// one every client derives); the hash is not curve-checked here, exactly
/// like [`verify_pda_address`], and the soundness argument is the same:
/// an owner- and layout-validated account at a hash output is a PDA, and
/// no key-holder can produce a program-owned account at one. For a raw or
/// system-owned account keep the curve-checked runtime derivation.
///
/// ```ignore
/// hopper::declare_id!("F4Um7PWsnZfN7y8WFzu1aPYJwqGduJTa4zuCGY9EUqMy");
/// pub const CONFIG: Address = hopper::const_pda!(ID, [b"config"], 254);
///
/// #[derive(Accounts)]
/// pub struct Touch<'info> {
///     #[account(mut, address = CONFIG)]
///     pub config: Account<'info, Config>,
/// }
/// ```
pub const fn const_program_address(program_id: &Address, seeds: &[&[u8]], bump: u8) -> Address {
    let backend = hopper_native::address::Address::new_from_array(*program_id.as_array());
    Address::new_from_array(
        hopper_native::pda::program_address_const(seeds, bump, &backend).to_bytes(),
    )
}

/// Verify that `expected` is the address the PDA hash of `seeds` (bump
/// included) yields under `program_id`: one `sol_sha256` (about 150 CU),
/// no `create_program_address` syscall (1,500 CU) and no curve check.
///
/// Sound wherever the address is already bound to something only a PDA
/// can be: an account this program owns and whose layout validated (no
/// private key can sign a program-owned account into existence at a hash
/// output), or an account about to be created by a CPI signed with these
/// seeds (the runtime's own signer check rejects an on-curve address). For
/// an address with no such binding, an unchecked or system account, use
/// [`verify_pda_address_checked`], which keeps the curve rejection.
#[inline]
pub fn verify_pda_address(
    seeds: &[&[u8]],
    program_id: &Address,
    expected: &Address,
) -> Result<(), ProgramError> {
    #[cfg(target_os = "solana")]
    {
        hopper_native::pda::verify_program_address(
            seeds,
            crate::native_boundary::as_backend_address(program_id),
            crate::native_boundary::as_backend_address(expected),
        )
        .map_err(ProgramError::from)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id, expected);
        Err(ProgramError::InvalidSeeds)
    }
}

/// [`verify_pda_address`] kept out of line.
///
/// `#[derive(Accounts)]` calls this on the branch of a CPI-proven `init`
/// field that the creation CPI cannot prove (a signer, or an account that
/// already holds data). That branch is cold, so the seed staging and the
/// hash compare, about 700 bytes inlined, are linked once for the program
/// instead of once per such field.
#[cold]
#[inline(never)]
pub fn verify_pda_address_cold(
    seeds: &[&[u8]],
    program_id: &Address,
    expected: &Address,
) -> Result<(), ProgramError> {
    verify_pda_address(seeds, program_id, expected)
}

/// [`verify_pda_address`] with the full `create_program_address` syscall,
/// so an address whose hash lands on the ed25519 curve is refused.
#[inline]
pub fn verify_pda_address_checked(
    seeds: &[&[u8]],
    program_id: &Address,
    expected: &Address,
) -> Result<(), ProgramError> {
    let derived = create_program_address(seeds, program_id)?;
    if crate::address::address_eq(&derived, expected) {
        Ok(())
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}

/// Find the bump under which `seeds` hash to `expected`, searching from
/// 255 down with one `sol_sha256` per candidate and no curve check (about
/// 150 CU per candidate instead of about 310). Returns `InvalidSeeds` when
/// no bump matches. Same soundness condition as [`verify_pda_address`]:
/// use it only when `expected` is bound to a program-owned or about-to-be
/// created account; otherwise [`find_canonical_bump_checked`].
#[inline]
pub fn find_bump_for_address(
    seeds: &[&[u8]],
    program_id: &Address,
    expected: &Address,
) -> Result<u8, ProgramError> {
    #[cfg(target_os = "solana")]
    {
        hopper_native::pda::find_bump_for_address(
            seeds,
            crate::native_boundary::as_backend_address(program_id),
            crate::native_boundary::as_backend_address(expected),
        )
        .map_err(ProgramError::from)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (seeds, program_id, expected);
        Err(ProgramError::InvalidSeeds)
    }
}

/// The canonical bump for `seeds`, found with the curve check on every
/// candidate, provided the canonical address equals `expected`.
#[inline]
pub fn find_canonical_bump_checked(
    seeds: &[&[u8]],
    program_id: &Address,
    expected: &Address,
) -> Result<u8, ProgramError> {
    let (derived, bump) = find_program_address(seeds, program_id);
    if crate::address::address_eq(&derived, expected) {
        Ok(bump)
    } else {
        Err(ProgramError::InvalidSeeds)
    }
}

/// Verify that an account's address matches a PDA derived from the given seeds.
#[inline]
pub fn verify_pda(
    account: &AccountView<'_>,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<(), ProgramError> {
    #[cfg(target_os = "solana")]
    {
        hopper_native::pda::verify_pda(
            account.as_backend(),
            seeds,
            crate::native_boundary::as_backend_address(program_id),
        )
        .map_err(ProgramError::from)
    }

    #[cfg(not(target_os = "solana"))]
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
    account: &AccountView<'_>,
    seeds: &[&[u8]],
    bump: u8,
    program_id: &Address,
) -> Result<(), ProgramError> {
    #[cfg(target_os = "solana")]
    {
        hopper_native::pda::verify_pda_with_bump(
            account.as_backend(),
            seeds,
            bump,
            crate::native_boundary::as_backend_address(program_id),
        )
        .map_err(ProgramError::from)
    }

    #[cfg(not(target_os = "solana"))]
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
// ---------------------------------------------------------------------
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
    account: &AccountView<'_>,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<u8, ProgramError> {
    #[cfg(target_os = "solana")]
    {
        let expected_addr = account.as_backend().address();
        let backend_expected =
            // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            unsafe { &*(expected_addr as *const hopper_native::address::Address) };
        verify_pda_sha256_loop(backend_expected, seeds, program_id)
    }

    #[cfg(not(target_os = "solana"))]
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
    #[cfg(target_os = "solana")]
    {
        let backend_expected =
            // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            unsafe { &*(expected as *const Address as *const hopper_native::address::Address) };
        verify_pda_sha256_loop(backend_expected, seeds, program_id).map(|_| ())
    }

    #[cfg(not(target_os = "solana"))]
    {
        let (derived, _) = find_program_address(seeds, program_id);
        if crate::address::address_eq(&derived, expected) {
            Ok(())
        } else {
            Err(ProgramError::InvalidSeeds)
        }
    }
}

/// Shared sha256-only PDA verify loop used by both `find_and_verify_pda`
/// and `verify_pda_strict`.
///
// ---------------------------------------------------------------------
/// Returns the matching bump on success.
///
/// `#[inline(always)]` is deliberate and MEASURED, do not "fix" the
/// duplication: outlining this (`inline(never)`) was tried on 2026-07-09
/// and saved only 88 bytes of release `.text` while costing **+44..+73
/// CU on every benched vault row** (Authorize 420→464, Counter 518→591,
/// Deposit 1653→1697, Withdraw 494→541), the call boundary defeats
/// LLVM's per-call-site specialization of the seed-list build and bump
/// loop, and the syscall does NOT dominate at that point. Size-per-CU,
/// the inlined copies win decisively.
#[cfg(target_os = "solana")]
#[inline(always)]
fn verify_pda_sha256_loop(
    expected: &hopper_native::address::Address,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<u8, ProgramError> {
    let backend_pid = crate::native_boundary::as_backend_address(program_id);
    let n = seeds.len().min(16);
    let mut slices = core::mem::MaybeUninit::<[&[u8]; 19]>::uninit();
    let sptr = slices.as_mut_ptr() as *mut &[u8];
    let mut i = 0;
    while i < n {
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { sptr.add(i).write(*seeds.get_unchecked(i)) };
        i += 1;
    }
    let mut bump_byte = [255u8];
    // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    unsafe {
        sptr.add(n).write(&bump_byte as &[u8]);
        sptr.add(n + 1).write(backend_pid.as_ref());
        sptr.add(n + 2)
            .write(hopper_native::address::PDA_MARKER.as_slice());
    }
    // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
    let input = unsafe { core::slice::from_raw_parts(sptr as *const &[u8], n + 3) };

    let mut bump: u16 = 256;
    while bump > 0 {
        bump -= 1;
        bump_byte[0] = bump as u8;

        let mut hash = core::mem::MaybeUninit::<[u8; 32]>::uninit();
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            hopper_native::syscalls::sol_sha256(
                input as *const _ as *const u8,
                input.len() as u64,
                hash.as_mut_ptr() as *mut u8,
            );
        }
        let derived =
            // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
            unsafe { &*(hash.as_ptr() as *const hopper_native::address::Address) };
        if hopper_native::address::address_eq(derived, expected) {
            return Ok(bump as u8);
        }
    }

    Err(ProgramError::InvalidSeeds)
}

/// Verify a PDA using the bump stored in account data (cheapest path).
///
/// Reads the bump byte at `bump_offset` in account data, appends it to seeds,
/// then hashes with SHA-256 and compares to the account address. ~200 CU total.
#[inline]
pub fn verify_pda_from_stored_bump(
    account: &AccountView<'_>,
    seeds: &[&[u8]],
    bump_offset: usize,
    program_id: &Address,
) -> Result<(), ProgramError> {
    #[cfg(target_os = "solana")]
    {
        hopper_native::verify_pda_from_stored_bump(
            account.as_backend(),
            seeds,
            bump_offset,
            crate::native_boundary::as_backend_address(program_id),
        )
        .map_err(ProgramError::from)
    }

    #[cfg(not(target_os = "solana"))]
    {
        // Off-chain fallback: read bump, append to seeds, derive + compare.
        let data = account.try_borrow()?;
        if bump_offset >= data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let bump = data[bump_offset];
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The devnet lane of 2026-09-21 (`audit/devnet-evidence-2026-09-21/counter/`)
    /// created these PDAs on chain under program `F4Um7PWs…`; the const
    /// derivation must land on the same addresses, and on the address pina's
    /// counter program derives for the same payer.
    #[test]
    fn const_program_address_matches_devnet_created_pdas() {
        const PROGRAM: Address = crate::address!("F4Um7PWsnZfN7y8WFzu1aPYJwqGduJTa4zuCGY9EUqMy");
        const PAYER: Address = crate::address!("4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn");
        const AUTHORITY_C: Address =
            crate::address!("7Qj28pSptq3YEdppwTmxDEP4jLsS1o67D1ZfKQJB9SE2");
        const PINA_COUNTER: Address =
            crate::address!("GJQcuWrT2f3f4KNuJcXhhwUa1ZQTYbxzzJ1hotzKu8hS");

        const PDA_A: Address = crate::const_pda!(PROGRAM, [b"counter", PAYER.as_array()], 252);
        const PDA_C: Address =
            crate::const_pda!(PROGRAM, [b"counter", AUTHORITY_C.as_array()], 254);
        const PDA_PINA: Address =
            const_program_address(&PINA_COUNTER, &[b"counter", PAYER.as_array()], 253);

        assert_eq!(
            PDA_A,
            crate::address!("Cn3JBYNBEctRDGuotxM7c3Fz3QgCZgRXkKV1G7h1qZKn")
        );
        assert_eq!(
            PDA_C,
            crate::address!("6vh34eBGs3gvwdaJ3fgXDLQMtNfqUwSJYrHqZ3FgCwYP")
        );
        assert_eq!(
            PDA_PINA,
            crate::address!("CW1z5aL4hTAFFubWKVKw1ANkdYurNEAiWbqxKsDCaERH")
        );
        // A different bump is a different address, never a silent match.
        assert_ne!(
            const_program_address(&PROGRAM, &[b"counter", PAYER.as_array()], 251),
            PDA_A
        );
    }
}
