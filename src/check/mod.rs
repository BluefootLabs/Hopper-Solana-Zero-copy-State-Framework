//! Multi-tier validation system.
//!
//! Hopper supports five validation levels:
//!
//! 1. **Account-local**: owner, signer, writable, size, discriminator, layout_id
//! 2. **Cross-account**: `vault.mint == mint.address()`, authority matches
//! 3. **State-transition**: status enum transitions, balance bounds
//! 4. **CPI composition**: post-CPI invariants, no-CPI guards
//! 5. **Post-mutation**: balance conservation, solvency invariants (via `PostMutationValidator`)
//!
//! Validation can be composed with named groups (`ValidationGroup`), instruction-specific
//! rule packs (`TransitionRulePack`), and multi-group bundles (`ValidationBundle`).

pub mod fast;
pub mod graph;
pub mod guards;
pub mod modifier;
pub mod trust;

use hopper_runtime::{address::address_eq, error::ProgramError, AccountView, Address, ProgramResult};

// --- Tier 1: Account-Local -------------------------------------------

/// Check that an account is a signer.
#[inline(always)]
pub fn check_signer(account: &AccountView) -> ProgramResult {
    if !account.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    Ok(())
}

/// Check that an account is writable.
#[inline(always)]
pub fn check_writable(account: &AccountView) -> ProgramResult {
    if !account.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that an account is owned by the expected program.
#[inline(always)]
pub fn check_owner(account: &AccountView, expected: &Address) -> ProgramResult {
    if !account.owned_by(expected) {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

/// Check that an account is executable (a program).
#[inline(always)]
pub fn check_executable(account: &AccountView) -> ProgramResult {
    if !account.executable() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check minimum data size.
#[inline(always)]
pub fn check_size(data: &[u8], min_len: usize) -> ProgramResult {
    if data.len() < min_len {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(())
}

/// Check that the discriminator byte matches.
#[inline(always)]
pub fn check_discriminator(data: &[u8], expected: u8) -> ProgramResult {
    if data.is_empty() || data[0] != expected {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check uninitialized: account data is empty.
#[inline(always)]
pub fn check_uninitialized(account: &AccountView) -> ProgramResult {
    if !account.is_data_empty() {
        return Err(ProgramError::AccountAlreadyInitialized);
    }
    Ok(())
}

/// Check that an account has not been closed (no close sentinel).
#[inline(always)]
pub fn check_not_closed(data: &[u8]) -> ProgramResult {
    if !data.is_empty() && data[0] == crate::account::CLOSE_SENTINEL {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Rent-exempt minimum lamports for a given data size.
#[inline(always)]
pub fn rent_exempt_min(data_len: usize) -> u64 {
    ((128 + data_len) as u64) * 6960
}

/// Check that an account is rent exempt.
#[inline(always)]
pub fn check_rent_exempt(account: &AccountView) -> ProgramResult {
    let lamports = account.lamports();
    let data = account.try_borrow()?;
    let min = rent_exempt_min(data.len());
    if lamports < min {
        return Err(ProgramError::InsufficientFunds);
    }
    Ok(())
}

/// Check that the account has at least `min` lamports.
#[inline(always)]
pub fn check_lamports_gte(account: &AccountView, min: u64) -> ProgramResult {
    let lamports = account.lamports();
    if lamports < min {
        return Err(ProgramError::InsufficientFunds);
    }
    Ok(())
}

// --- Tier 2: Cross-Account ------------------------------------------

/// Check that two account addresses are equal.
#[inline(always)]
pub fn check_keys_eq(a: &AccountView, b: &AccountView) -> ProgramResult {
    if !address_eq(a.address(), b.address()) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Fast 32-byte key equality check using 4x u64 comparisons.
///
/// Short-circuits on the first differing 8-byte chunk, saving cycles
/// vs byte-by-byte comparison for addresses that differ early.
/// hopper-native-inspired optimization.
#[inline(always)]
pub fn keys_eq_fast(a: &[u8; 32], b: &[u8; 32]) -> bool {
    // SAFETY: [u8; 32] is always valid for read_unaligned as u64.
    // We compare 4 x u64 chunks with short-circuit evaluation.
    unsafe {
        let a_ptr = a.as_ptr() as *const u64;
        let b_ptr = b.as_ptr() as *const u64;
        core::ptr::read_unaligned(a_ptr) == core::ptr::read_unaligned(b_ptr)
            && core::ptr::read_unaligned(a_ptr.add(1)) == core::ptr::read_unaligned(b_ptr.add(1))
            && core::ptr::read_unaligned(a_ptr.add(2)) == core::ptr::read_unaligned(b_ptr.add(2))
            && core::ptr::read_unaligned(a_ptr.add(3)) == core::ptr::read_unaligned(b_ptr.add(3))
    }
}

/// Check if a 32-byte address is all zeros (the default/system address).
///
/// Uses an OR-fold: OR all 4 u64 chunks together, then check if the result is zero.
/// This avoids 32 individual byte comparisons.
#[inline(always)]
pub fn is_zero_address(addr: &[u8; 32]) -> bool {
    // SAFETY: [u8; 32] is always valid for read_unaligned as u64.
    unsafe {
        let ptr = addr.as_ptr() as *const u64;
        let combined = core::ptr::read_unaligned(ptr)
            | core::ptr::read_unaligned(ptr.add(1))
            | core::ptr::read_unaligned(ptr.add(2))
            | core::ptr::read_unaligned(ptr.add(3));
        combined == 0
    }
}

/// Check `has_one`: a stored address in account data matches another account's address.
///
/// `stored` is the 32-byte address stored in account data at a given offset.
/// This is the Anchor-style `has_one` equivalent.
#[inline(always)]
pub fn check_has_one(
    stored: &[u8; 32],
    account: &AccountView,
) -> ProgramResult {
    // SAFETY: Address is [u8; 32]. Reinterpret as reference.
    let addr: &[u8; 32] = unsafe { &*(account.address() as *const Address as *const [u8; 32]) };
    if !keys_eq_fast(stored, addr) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that two accounts are unique (different addresses).
#[inline(always)]
pub fn check_accounts_unique(a: &AccountView, b: &AccountView) -> ProgramResult {
    if address_eq(a.address(), b.address()) {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Check that three accounts are all unique.
#[inline(always)]
pub fn check_accounts_unique_3(
    a: &AccountView,
    b: &AccountView,
    c: &AccountView,
) -> ProgramResult {
    if address_eq(a.address(), b.address())
        || address_eq(a.address(), c.address())
        || address_eq(b.address(), c.address())
    {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Check an account's address matches an expected value.
#[inline(always)]
pub fn check_address(account: &AccountView, expected: &Address) -> ProgramResult {
    if !address_eq(account.address(), expected) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check instruction data meets minimum length.
#[inline(always)]
pub fn check_instruction_data_min(data: &[u8], min: usize) -> ProgramResult {
    if data.len() < min {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(())
}

// --- Tier 3: Combined Checks ----------------------------------------

/// Combined account check: owner + discriminator + minimum size.
///
/// This is the most common validation pattern. One function call instead of three.
#[inline(always)]
pub fn check_account(
    account: &AccountView,
    program_id: &Address,
    disc: u8,
    min_size: usize,
) -> ProgramResult {
    check_owner(account, program_id)?;
    let data = account.try_borrow()?;
    check_size(&data, min_size)?;
    check_discriminator(&data, disc)?;
    Ok(())
}

/// System program check.
#[inline(always)]
pub fn check_system_program(account: &AccountView) -> ProgramResult {
    // System program ID: 11111111111111111111111111111111
    const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);
    if *account.address() != SYSTEM_PROGRAM {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

// --- PDA Helpers ----------------------------------------------------

/// Verify a PDA with bump, using the cheap `create_program_address` path.
///
/// This costs ~200 CU vs ~1500 CU for `find_program_address`.
/// Always use this when you have the bump stored.
#[inline(always)]
pub fn verify_pda(
    account: &AccountView,
    seeds: &[&[u8]],
    bump: u8,
    program_id: &Address,
) -> ProgramResult {
    hopper_runtime::pda::verify_pda_with_bump(account, seeds, bump, program_id)
}

/// Find a PDA and verify it matches the account, returning the bump.
///
/// On Hopper Native this uses the fast PDA path (`sol_sha256` +
/// `sol_curve_validate_point`), which is roughly ~544 CU for a first-try bump.
/// Prefer `verify_pda` when bump is stored.
#[inline(always)]
pub fn find_and_verify_pda(
    account: &AccountView,
    seeds: &[&[u8]],
    program_id: &Address,
) -> Result<u8, ProgramError> {
    hopper_runtime::pda::find_and_verify_pda(account, seeds, program_id)
}

// --- BUMP_OFFSET PDA Optimization ----------------------------------
//
// Inspired by Quasar's BUMP_OFFSET pattern. When a layout stores its PDA
// bump in a known field, we can read it directly from account data and
// call `create_program_address` (~200 CU) instead of `find_program_address`
// (~544 CU). Saves ~344 CU per PDA validation.

/// Verify a PDA by reading the bump from account data at a known offset.
///
/// This is the **BUMP_OFFSET optimization**: when a layout stores its PDA bump
/// byte at a compile-time-known offset, we read it directly from account data
/// and use `create_program_address` (~200 CU) instead of `find_program_address`
/// (~544 CU). Saves ~344 CU per PDA check.
///
/// # Arguments
/// - `account` -- The account to verify
/// - `seeds` -- PDA seeds (without bump)
/// - `bump_offset` -- Byte offset of the bump field in account data
/// - `program_id` -- The owning program
#[inline(always)]
pub fn verify_pda_cached(
    account: &AccountView,
    seeds: &[&[u8]],
    bump_offset: usize,
    program_id: &Address,
) -> ProgramResult {
    #[cfg(target_os = "solana")]
    {
        let data = account.try_borrow()?;
        if bump_offset >= data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let bump = data[bump_offset];
        let bump_seed = [bump];
        let mut all_seeds: [&[u8]; 17] = [&[]; 17];
        let seed_count = seeds.len();
        if seed_count > 16 {
            return Err(ProgramError::InvalidSeeds);
        }
        let mut i = 0;
        while i < seed_count {
            all_seeds[i] = seeds[i];
            i += 1;
        }
        all_seeds[seed_count] = &bump_seed;

        let derived = Address::create_program_address(
            &all_seeds[..seed_count + 1],
            program_id,
        )?;

        if !address_eq(account.address(), &derived) {
            return Err(ProgramError::InvalidSeeds);
        }
        Ok(())
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (account, seeds, bump_offset, program_id);
        Err(ProgramError::InvalidSeeds)
    }
}

// --- Multi-Owner Foreign Load --------------------------------------
//
// Inspired by Quasar's Interface<T> + ProgramInterface pattern.
// Allows loading foreign accounts that could be owned by any of several
// programs (e.g., Token program OR Token-2022).

/// Check that an account is owned by one of the given program IDs.
///
/// Used for multi-program interfaces (e.g., Token vs Token-2022).
/// Returns the index of the matching owner, or error.
#[inline]
pub fn check_owner_multi(
    account: &AccountView,
    owners: &[&Address],
) -> Result<usize, ProgramError> {
    // SAFETY: Reading the owner field is safe in the context of
    // account validation; no conflicting borrows exist yet.
    let acct_owner = unsafe { account.owner() };
    for (i, expected) in owners.iter().enumerate() {
        if acct_owner == *expected {
            return Ok(i);
        }
    }
    Err(ProgramError::IncorrectProgramId)
}

// --- Instruction Introspection Guards ------------------------------
//
// Ported and improved from Jiminy's instruction sysvar analysis.
// Detects CPI re-entrancy, flash loans, and sandwich attacks.

/// Instructions sysvar address (Sysvar1nstructions1111111111111111111111111).
#[allow(dead_code)]
const INSTRUCTIONS_SYSVAR: Address = {
    // Sysvar1nstructions1111111111111111111111111
    // This is the base58-decoded address
    let mut addr = [0u8; 32];
    addr[0] = 0x06;
    addr[1] = 0xa7;
    addr[2] = 0xd5;
    addr[3] = 0x17;
    addr[4] = 0x18;
    addr[5] = 0x7b;
    addr[6] = 0xd1;
    addr[7] = 0x66;
    addr[8] = 0x35;
    addr[9] = 0xda;
    addr[10] = 0xd4;
    addr[11] = 0x04;
    addr[12] = 0x55;
    addr[13] = 0xfb;
    addr[14] = 0x04;
    addr[15] = 0x6e;
    addr[16] = 0x12;
    addr[17] = 0x46;
    addr[18] = 0x00;
    addr[19] = 0x00;
    addr[20] = 0x00;
    addr[21] = 0x00;
    addr[22] = 0x00;
    addr[23] = 0x00;
    addr[24] = 0x00;
    addr[25] = 0x00;
    addr[26] = 0x00;
    addr[27] = 0x00;
    addr[28] = 0x00;
    addr[29] = 0x00;
    addr[30] = 0x00;
    addr[31] = 0x00;
    Address::new_from_array(addr)
};

/// Read the number of instructions in the current transaction.
///
/// The Instructions sysvar stores `num_instructions` as the first u16 LE
/// at offset 0 in the serialized data.
#[inline(always)]
pub fn instruction_count(sysvar_data: &[u8]) -> Result<u16, ProgramError> {
    if sysvar_data.len() < 2 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u16::from_le_bytes([sysvar_data[0], sysvar_data[1]]))
}

/// Read the current instruction index from the Instructions sysvar.
///
/// The Instructions sysvar stores the current instruction index as the
/// last u16 LE at offset `data.len() - 2`.
#[inline(always)]
pub fn current_instruction_index(sysvar_data: &[u8]) -> Result<u16, ProgramError> {
    let len = sysvar_data.len();
    if len < 2 {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(u16::from_le_bytes([sysvar_data[len - 2], sysvar_data[len - 1]]))
}

/// Read the program_id of instruction at the given index.
///
/// Instructions sysvar layout:
/// ```text
/// [u16 num_instructions]
/// [u16 offset_0] [u16 offset_1] ... [u16 offset_{n-1}]  <-- offset table
/// [serialized instruction 0]
/// [serialized instruction 1]
/// ...
/// [u16 current_instruction_index]  <-- last 2 bytes
/// ```
///
/// Per-instruction layout at `sysvar_data[offset]`:
/// ```text
/// [u16 num_accounts]
/// [u8 flags + [u8; 32] pubkey] * num_accounts  (33 bytes each)
/// [u8; 32] program_id
/// [u16 data_len]
/// [u8; data_len] data
/// ```
#[inline]
pub fn read_program_id_at(
    sysvar_data: &[u8],
    index: u16,
) -> Result<[u8; 32], ProgramError> {
    let num_ix = instruction_count(sysvar_data)?;
    if index >= num_ix {
        return Err(ProgramError::InvalidArgument);
    }
    // Offset table starts at byte 2, with one u16 per instruction.
    let offset_entry = 2 + (index as usize) * 2;
    if offset_entry + 2 > sysvar_data.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    let ix_offset = u16::from_le_bytes([
        sysvar_data[offset_entry],
        sysvar_data[offset_entry + 1],
    ]) as usize;

    // At ix_offset: [num_accounts: u16 LE][accounts...]
    // Each account meta is 33 bytes (1 byte flags + 32 byte pubkey).
    // After accounts: [program_id: 32 bytes][data_len: u16][data...]
    if ix_offset + 2 > sysvar_data.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    let num_accounts = u16::from_le_bytes([
        sysvar_data[ix_offset],
        sysvar_data[ix_offset + 1],
    ]) as usize;
    // Skip accounts: each is 1 + 32 = 33 bytes
    let program_id_offset = ix_offset + 2 + num_accounts * 33;
    if program_id_offset + 32 > sysvar_data.len() {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut pid = [0u8; 32];
    pid.copy_from_slice(&sysvar_data[program_id_offset..program_id_offset + 32]);
    Ok(pid)
}

/// Require that the current instruction is top-level (not a CPI).
///
/// Checks: current instruction's program_id matches `our_program`.
/// If called via CPI, the current instruction would have a different
/// program_id, so this fails.
#[inline]
pub fn require_top_level(
    sysvar_data: &[u8],
    our_program: &Address,
) -> ProgramResult {
    let current_idx = current_instruction_index(sysvar_data)?;
    let pid = read_program_id_at(sysvar_data, current_idx)?;
    if pid != *our_program.as_array() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Detect flash-loan bracket: same program called before AND after current.
///
/// Returns `Err` if the pattern is detected (the program appears both
/// before and after the current instruction index).
#[inline]
pub fn detect_flash_loan_bracket(
    sysvar_data: &[u8],
    our_program: &Address,
) -> ProgramResult {
    let current_idx = current_instruction_index(sysvar_data)?;
    let num_ix = instruction_count(sysvar_data)?;

    let mut before = false;
    let mut after = false;

    let mut i: u16 = 0;
    while i < num_ix {
        if i == current_idx {
            i += 1;
            continue;
        }
        if let Ok(pid) = read_program_id_at(sysvar_data, i) {
            if pid == *our_program.as_array() {
                if i < current_idx {
                    before = true;
                } else {
                    after = true;
                }
            }
        }
        i += 1;
    }

    if before && after {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Ensure our program is not invoked after the current instruction.
///
/// Prevents post-execution re-entrancy patterns.
#[inline]
pub fn check_no_subsequent_invocation(
    sysvar_data: &[u8],
    our_program: &Address,
) -> ProgramResult {
    let current_idx = current_instruction_index(sysvar_data)?;
    let num_ix = instruction_count(sysvar_data)?;

    let mut i = current_idx + 1;
    while i < num_ix {
        if let Ok(pid) = read_program_id_at(sysvar_data, i) {
            if pid == *our_program.as_array() {
                return Err(ProgramError::InvalidAccountData);
            }
        }
        i += 1;
    }
    Ok(())
}
