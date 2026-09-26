//! A SOL treasury with an administrator, delegated operator, spending limits,
//! live-Clock cooldowns, and rent-preserving withdrawals.
//!
//! Wallet deposits invoke the System Program. Withdrawals debit the validated
//! program-owned treasury. The 169-byte account contains three separately
//! validated segments: core (56 bytes), permissions (57), and budget (56).
//! Budget periods are administrator-controlled revisions, not Solana epochs.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;
use hopper::systems::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// -- Layouts ---------------------------------------------------------
//
// Hopper's segmented architecture: one account, multiple logical regions.
// Each segment is a hopper_layout! struct that can be individually overlaid.

// Header + authority + cumulative deposits.
mod state;
pub use state::*;

// -- Errors ----------------------------------------------------------

hopper_error! {
    base = 7000;
    Unauthorized,
    TreasuryFrozen,
    BudgetExceeded,
    WithdrawalTooLarge,
    CooldownNotElapsed,
    EpochMismatch,
    ZeroAmount,
    InsufficientBalance,
    AliasedAccount
}

// -- Disc Registry ---------------------------------------------------

hopper_register_discs! {
    TreasuryCore,
    PermissionSegment,
    BudgetSegment,
}

// -- Entrypoint ------------------------------------------------------

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init_treasury,
        1 => process_deposit,
        2 => process_withdraw,
        3 => process_update_permissions,
        4 => process_rotate_epoch,
    }
}

// -- Init Treasury ---------------------------------------------------
//
// Creates the multi-segment account and initializes all three segments.
// Accounts: [0] payer (signer, writable), [1] treasury (writable), [2] system

fn process_init_treasury(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() != 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let treasury = &accounts[1];
    let system_program = &accounts[2];

    check_signer(payer)?;
    check_signer(treasury)?;
    check_writable(payer)?;
    check_writable(treasury)?;
    distinct(payer, treasury)?;
    validate_system(system_program)?;

    // Exact payload: budget, maximum single withdrawal, cooldown seconds.
    if data.len() != 24 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let epoch_budget = parse_amount(&data[..8])?;
    let max_withdrawal = parse_amount(&data[8..16])?;
    let cooldown = parse_amount(&data[16..24])?;

    // Create the account with full treasury size
    let rent = hopper::hopper_runtime::rent::minimum_balance_live(TREASURY_ACCOUNT_SIZE)?;
    hopper::hopper_system::CreateAccount {
        from: payer,
        to: treasury,
        lamports: rent,
        space: TREASURY_ACCOUNT_SIZE as u64,
        owner: program_id,
    }
    .invoke()?;

    let mut buf = treasury.try_borrow_mut()?;

    // Zero-init entire buffer
    zero_init(&mut buf);

    // Write core segment header + fields
    let core_slice = &mut buf[CORE_OFFSET..CORE_OFFSET + TreasuryCore::LEN];
    TreasuryCore::write_init_header(core_slice)?;
    let core = TreasuryCore::overlay_mut(core_slice)?;
    core.authority = TypedAddress::from_account(payer);
    core.total_deposited = WireU64::new(0);

    // Write permission segment
    let perm_slice = &mut buf[PERM_OFFSET..PERM_OFFSET + PermissionSegment::LEN];
    PermissionSegment::write_init_header(perm_slice)?;
    let perm = PermissionSegment::overlay_mut(perm_slice)?;
    perm.operator = TypedAddress::from_account(payer);
    perm.frozen = WireBool::new(false);
    perm.max_single_withdrawal = WireU64::new(max_withdrawal);

    // Write budget segment
    let budget_slice = &mut buf[BUDGET_OFFSET..BUDGET_OFFSET + BudgetSegment::LEN];
    BudgetSegment::write_init_header(budget_slice)?;
    let budget = BudgetSegment::overlay_mut(budget_slice)?;
    budget.epoch_budget = WireU64::new(epoch_budget);
    budget.epoch_spent = WireU64::new(0);
    budget.epoch_number = WireU64::new(0);
    budget.cooldown_seconds = WireU64::new(cooldown);
    budget.last_withdrawal_ts = WireU64::new(0);

    Ok(())
}

// -- Deposit ---------------------------------------------------------
//
// Accounts: [0] depositor (signer, writable), [1] treasury (writable), [2] system

fn process_deposit(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let [depositor, treasury, system_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    check_signer(depositor)?;
    check_writable(depositor)?;
    check_writable(treasury)?;
    check_owner(treasury, program_id)?;
    distinct(depositor, treasury)?;
    validate_system(system_program)?;
    let amount = parse_amount(data)?;
    hopper_require!(amount > 0, ZeroAmount);
    let new_total = {
        let buf = treasury.try_borrow()?;
        validate_treasury(&buf)?;
        TreasuryCore::overlay(&buf[..TreasuryCore::LEN])?
            .total_deposited
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?
    };
    // Release the state borrow before crossing the CPI boundary.
    hopper::system::Transfer {
        from: depositor,
        to: treasury,
        lamports: amount,
    }
    .invoke()?;
    let mut buf = treasury.try_borrow_mut()?;
    TreasuryCore::overlay_mut(&mut buf[..TreasuryCore::LEN])?.total_deposited =
        WireU64::new(new_total);
    Ok(())
}

// -- Withdraw --------------------------------------------------------
//
// Multi-segment validation: permissions + budget + balance.
// Accounts: [0] operator (signer), [1] treasury (writable), [2] destination (writable)

fn process_withdraw(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    if accounts.len() != 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let operator = &accounts[0];
    let treasury = &accounts[1];
    let destination = &accounts[2];

    check_signer(operator)?;
    check_writable(treasury)?;
    check_writable(destination)?;
    check_owner(treasury, program_id)?;

    // Parse amount
    if data.len() != 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);

    distinct(treasury, destination)?;
    let mut buf = treasury.try_borrow_mut()?;
    validate_treasury(&buf)?;

    // -- Phase 1: Resolve segments -----------------------------------

    let perm = PermissionSegment::overlay(&buf[PERM_OFFSET..PERM_OFFSET + PermissionSegment::LEN])?;

    // -- Phase 2: Validate -------------------------------------------

    // Permission check: operator must match
    if !perm.operator.eq_account(operator) {
        return Err(Unauthorized.into());
    }

    // Frozen check
    if perm.frozen.get() {
        return Err(TreasuryFrozen.into());
    }

    // Max single withdrawal
    if amount > perm.max_single_withdrawal.get() {
        return Err(WithdrawalTooLarge.into());
    }

    // Budget check (read budget segment)
    let budget = BudgetSegment::overlay(&buf[BUDGET_OFFSET..BUDGET_OFFSET + BudgetSegment::LEN])?;
    let new_spent = budget
        .epoch_spent
        .get()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if new_spent > budget.epoch_budget.get() {
        return Err(BudgetExceeded.into());
    }

    let now =
        u64::try_from(Clock::get()?.unix_timestamp).map_err(|_| ProgramError::InvalidArgument)?;
    if now == 0 {
        return Err(ProgramError::InvalidArgument);
    }
    if budget.last_withdrawal_ts.get() != 0 {
        let ready = budget
            .last_withdrawal_ts
            .get()
            .checked_add(budget.cooldown_seconds.get())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if now < ready {
            return Err(CooldownNotElapsed.into());
        }
    }
    let destination_balance = destination
        .lamports()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    // Balance check
    let balance = treasury.lamports();
    let rent = hopper::hopper_runtime::rent::minimum_balance_live(TREASURY_ACCOUNT_SIZE)?;
    let available = balance.saturating_sub(rent);
    if amount > available {
        return Err(InsufficientBalance.into());
    }

    // -- Phase 3: Execute (mutate) -----------------------------------

    // Update budget
    let budget_mut =
        BudgetSegment::overlay_mut(&mut buf[BUDGET_OFFSET..BUDGET_OFFSET + BudgetSegment::LEN])?;
    budget_mut.epoch_spent = WireU64::new(new_spent);
    budget_mut.last_withdrawal_ts = WireU64::new(now);
    drop(buf);

    // Transfer SOL
    let t_lamports = treasury.lamports();
    treasury.set_lamports(t_lamports - amount)?;
    destination.set_lamports(destination_balance)?;

    // -- Phase 4: Post-mutation invariant ----------------------------

    hopper_invariant! {
        "treasury_solvent" => {
            let remaining = treasury.lamports();
            // The live minimum read in the balance check above.
            let min_rent = rent;
            if remaining < min_rent {
                Err(ProgramError::InsufficientFunds)
            } else {
                Ok(())
            }
        }
    }
}

// -- Update Permissions ----------------------------------------------
//
// Admin-only: update operator, toggle freeze, adjust max withdrawal.
// Accounts: [0] admin (signer), [1] treasury (writable)

fn process_update_permissions(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() != 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let admin = &accounts[0];
    let treasury = &accounts[1];

    check_signer(admin)?;
    check_writable(treasury)?;
    check_owner(treasury, program_id)?;

    let mut buf = treasury.try_borrow_mut()?;
    validate_treasury(&buf)?;

    // Verify admin
    let core = TreasuryCore::overlay(&buf[..PERM_OFFSET])?;
    if !core.authority.eq_account(admin) {
        return Err(Unauthorized.into());
    }

    // Parse update: action(1B) + payload
    if data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }

    let perm_mut = PermissionSegment::overlay_mut(
        &mut buf[PERM_OFFSET..PERM_OFFSET + PermissionSegment::LEN],
    )?;

    match data[0] {
        // Set operator
        0 => {
            if data.len() != 33 {
                return Err(ProgramError::InvalidInstructionData);
            }
            perm_mut.operator = TypedAddress::from_slice(
                data[1..33]
                    .try_into()
                    .map_err(|_| ProgramError::InvalidInstructionData)?,
            );
        }
        // Toggle freeze
        1 => {
            if data.len() != 1 {
                return Err(ProgramError::InvalidInstructionData);
            }
            perm_mut.frozen = WireBool::new(!perm_mut.frozen.get());
        }
        // Set max withdrawal
        2 => {
            if data.len() != 9 {
                return Err(ProgramError::InvalidInstructionData);
            }
            let max = u64::from_le_bytes([
                data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
            ]);
            perm_mut.max_single_withdrawal = WireU64::new(max);
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    }

    Ok(())
}

// -- Rotate Epoch ----------------------------------------------------
//
// Resets epoch budget spent counter. Admin-only.
// Accounts: [0] admin (signer), [1] treasury (writable)

fn process_rotate_epoch(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() != 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let admin = &accounts[0];
    let treasury = &accounts[1];

    check_signer(admin)?;
    check_writable(treasury)?;
    check_owner(treasury, program_id)?;

    let mut buf = treasury.try_borrow_mut()?;
    validate_treasury(&buf)?;

    // Verify admin
    let core = TreasuryCore::overlay(&buf[..PERM_OFFSET])?;
    if !core.authority.eq_account(admin) {
        return Err(Unauthorized.into());
    }

    // Parse new epoch params: new_epoch_number(8B) + optional new_budget(8B)
    if data.len() != 8 && data.len() != 16 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let new_epoch = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);

    let budget_mut =
        BudgetSegment::overlay_mut(&mut buf[BUDGET_OFFSET..BUDGET_OFFSET + BudgetSegment::LEN])?;

    // Epoch must advance
    if new_epoch <= budget_mut.epoch_number.get() {
        return Err(EpochMismatch.into());
    }

    budget_mut.epoch_number = WireU64::new(new_epoch);
    budget_mut.epoch_spent = WireU64::new(0);

    // Optional: update budget
    if data.len() >= 16 {
        let new_budget = u64::from_le_bytes([
            data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
        ]);
        budget_mut.epoch_budget = WireU64::new(new_budget);
    }

    Ok(())
}

fn parse_amount(data: &[u8]) -> Result<u64, ProgramError> {
    Ok(u64::from_le_bytes(
        data.try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    ))
}
fn distinct(a: &AccountView, b: &AccountView) -> ProgramResult {
    if a.address() == b.address() {
        return Err(AliasedAccount.into());
    }
    Ok(())
}
fn validate_system(account: &AccountView) -> ProgramResult {
    if account.address() != &SYSTEM_PROGRAM_ID || !account.executable() {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}
fn validate_treasury(data: &[u8]) -> ProgramResult {
    if data.len() != TREASURY_ACCOUNT_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    TreasuryCore::validate_header(&data[..PERM_OFFSET])?;
    PermissionSegment::validate_header(&data[PERM_OFFSET..BUDGET_OFFSET])?;
    BudgetSegment::validate_header(&data[BUDGET_OFFSET..])?;
    Ok(())
}
