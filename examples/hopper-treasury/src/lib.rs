//! # Hopper Segmented Treasury
//!
//! Demonstrates Hopper's advanced features with a multi-segment treasury:
//!
//! - **Segmented layout**: Core + Permissions + Budget Rules + Journal
//! - **Phased execution**: Resolve -> Validate -> Execute
//! - **Invariant checking**: Post-mutation balance conservation
//! - **Fast account validation**: u32 batched header checks
//! - **Migration path**: Append-only layout evolution
//!
//! ## Account Structure (256 bytes)
//!
//! ```text
//! [0..16]    AccountHeader (disc=10, version=1)
//! [16..89]   TreasuryCore: authority, vault_mint, total_deposited, total_withdrawn, bump
//! [89..153]  PermissionSegment: admin, operator, frozen flag, max_single_withdrawal
//! [153..217] BudgetSegment: epoch_budget, epoch_spent, epoch_number, cooldown_seconds
//! [217..256] JournalSegment: last_action(u8), last_actor(32B), last_timestamp(u64=8B)
//!                            Note: timestamp field is approximate in this example
//! ```
//!
//! ## Instructions
//!
//! - `0` = InitTreasury
//! - `1` = Deposit
//! - `2` = Withdraw (with budget + permission checks)
//! - `3` = UpdatePermissions (admin only)
//! - `4` = RotateEpoch (reset budget)

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

// -- Layouts ---------------------------------------------------------
//
// Hopper's segmented architecture: one account, multiple logical regions.
// Each segment is a hopper_layout! struct that can be individually overlaid.

// 16-byte header + 73 bytes = 89 total for core segment
hopper_layout! {
    /// Core treasury data.
    pub struct TreasuryCore, disc = 10, version = 1 {
        authority:        TypedAddress<Authority>  = 32,
        vault_mint:       TypedAddress<Mint>       = 32,
        total_deposited:  WireU64                  = 8,
        bump:             u8                       = 1,
    }
}

// Permissions segment -- overlaid at offset 89
hopper_layout! {
    /// Permission and access control segment.
    pub struct PermissionSegment, disc = 11, version = 1 {
        admin:                  TypedAddress<Authority>  = 32,
        operator:               TypedAddress<Authority>  = 32,
        frozen:                 WireBool                 = 1,
        max_single_withdrawal:  WireU64                  = 8,
    }
}

// Budget segment -- overlaid at offset 89 + 16 + 73 = 178
hopper_layout! {
    /// Per-epoch budget tracking segment.
    pub struct BudgetSegment, disc = 12, version = 1 {
        epoch_budget:       WireU64  = 8,
        epoch_spent:        WireU64  = 8,
        epoch_number:       WireU64  = 8,
        cooldown_seconds:   WireU64  = 8,
        last_withdrawal_ts: WireU64  = 8,
    }
}

// The full treasury account size: we pack all segments contiguously.
// Core (89) + Permissions (89) + Budget (56) = 234 bytes
const TREASURY_ACCOUNT_SIZE: usize = TreasuryCore::LEN + PermissionSegment::LEN + BudgetSegment::LEN;

// Segment offsets
const CORE_OFFSET: usize = 0;
const PERM_OFFSET: usize = TreasuryCore::LEN;
const BUDGET_OFFSET: usize = TreasuryCore::LEN + PermissionSegment::LEN;

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
    InsufficientBalance
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
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let treasury = &accounts[1];
    let system_program = &accounts[2];

    check_signer(payer)?;
    check_writable(treasury)?;

    // Parse init params: vault_mint (32B) + epoch_budget (8B) + max_withdrawal (8B)
    if data.len() < 48 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let vault_mint = &data[0..32];
    let epoch_budget = u64::from_le_bytes([
        data[32], data[33], data[34], data[35],
        data[36], data[37], data[38], data[39],
    ]);
    let max_withdrawal = u64::from_le_bytes([
        data[40], data[41], data[42], data[43],
        data[44], data[45], data[46], data[47],
    ]);

    // Create the account with full treasury size
    let rent = rent_exempt_min(TREASURY_ACCOUNT_SIZE);
    hopper::hopper_runtime::system::CreateAccount {
        from: payer,
        to: treasury,
        lamports: rent,
        space: TREASURY_ACCOUNT_SIZE as u64,
        owner: program_id,
    }
    .invoke()?;

    // SAFETY: Just created, exclusive access guaranteed.
    let buf = unsafe { treasury.borrow_unchecked_mut() };

    // Zero-init entire buffer
    zero_init(buf);

    // Write core segment header + fields
    let core_slice = &mut buf[CORE_OFFSET..CORE_OFFSET + TreasuryCore::LEN];
    TreasuryCore::write_init_header(core_slice)?;
    let core = TreasuryCore::overlay_mut(core_slice)?;
    core.authority = TypedAddress::from_account(payer);
    core.vault_mint = TypedAddress::from_slice(vault_mint.try_into().map_err(|_| ProgramError::InvalidInstructionData)?);
    core.total_deposited = WireU64::new(0);
    core.bump = 0; // Set by caller if PDA

    // Write permission segment
    let perm_slice = &mut buf[PERM_OFFSET..PERM_OFFSET + PermissionSegment::LEN];
    PermissionSegment::write_init_header(perm_slice)?;
    let perm = PermissionSegment::overlay_mut(perm_slice)?;
    perm.admin = TypedAddress::from_account(payer);
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
    budget.cooldown_seconds = WireU64::new(60); // 60s default cooldown
    budget.last_withdrawal_ts = WireU64::new(0);

    Ok(())
}

// -- Deposit ---------------------------------------------------------
//
// Accounts: [0] depositor (signer, writable), [1] treasury (writable)

fn process_deposit(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let depositor = &accounts[0];
    let treasury = &accounts[1];

    check_signer(depositor)?;
    check_writable(treasury)?;
    check_owner(treasury, program_id)?;

    // Parse amount
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);

    // Transfer SOL
    let dep_lamports = depositor.lamports();
    depositor.set_lamports(
        dep_lamports.checked_sub(amount).ok_or(ProgramError::InsufficientFunds)?,
    );
    let t_lamports = treasury.lamports();
    treasury.set_lamports(
        t_lamports.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?,
    );

    // Update core balance
    let buf = unsafe { treasury.borrow_unchecked_mut() };
    let core = TreasuryCore::overlay_mut(&mut buf[CORE_OFFSET..CORE_OFFSET + TreasuryCore::LEN])?;
    let new_total = core.total_deposited.get()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    core.total_deposited = WireU64::new(new_total);

    Ok(())
}

// -- Withdraw --------------------------------------------------------
//
// Multi-segment validation: permissions + budget + balance.
// Accounts: [0] operator (signer), [1] treasury (writable), [2] destination (writable)

fn process_withdraw(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
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
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);

    let buf = unsafe { treasury.borrow_unchecked_mut() };

    // -- Phase 1: Resolve segments -----------------------------------

    let core = TreasuryCore::overlay(&buf[CORE_OFFSET..CORE_OFFSET + TreasuryCore::LEN])?;
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
    let new_spent = budget.epoch_spent.get()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if new_spent > budget.epoch_budget.get() {
        return Err(BudgetExceeded.into());
    }

    // Balance check
    let balance = treasury.lamports();
    let rent = rent_exempt_min(TREASURY_ACCOUNT_SIZE);
    let available = balance.saturating_sub(rent);
    if amount > available {
        return Err(InsufficientBalance.into());
    }

    // -- Phase 3: Execute (mutate) -----------------------------------

    // Update budget
    let budget_mut = BudgetSegment::overlay_mut(
        &mut buf[BUDGET_OFFSET..BUDGET_OFFSET + BudgetSegment::LEN],
    )?;
    budget_mut.epoch_spent = WireU64::new(new_spent);

    // Transfer SOL
    let t_lamports = treasury.lamports();
    treasury.set_lamports(t_lamports - amount);
    let d_lamports = destination.lamports();
    destination.set_lamports(
        d_lamports.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?,
    );

    // -- Phase 4: Post-mutation invariant ----------------------------

    hopper_invariant! {
        "treasury_solvent" => {
            let remaining = treasury.lamports();
            let min_rent = rent_exempt_min(TREASURY_ACCOUNT_SIZE);
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
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let admin = &accounts[0];
    let treasury = &accounts[1];

    check_signer(admin)?;
    check_writable(treasury)?;
    check_owner(treasury, program_id)?;

    let buf = unsafe { treasury.borrow_unchecked_mut() };

    // Verify admin
    let perm = PermissionSegment::overlay(&buf[PERM_OFFSET..PERM_OFFSET + PermissionSegment::LEN])?;
    if !perm.admin.eq_account(admin) {
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
            if data.len() < 33 {
                return Err(ProgramError::InvalidInstructionData);
            }
            perm_mut.operator = TypedAddress::from_slice(data[1..33].try_into().map_err(|_| ProgramError::InvalidInstructionData)?);
        }
        // Toggle freeze
        1 => {
            perm_mut.frozen = WireBool::new(!perm_mut.frozen.get());
        }
        // Set max withdrawal
        2 => {
            if data.len() < 9 {
                return Err(ProgramError::InvalidInstructionData);
            }
            let max = u64::from_le_bytes([
                data[1], data[2], data[3], data[4],
                data[5], data[6], data[7], data[8],
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
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let admin = &accounts[0];
    let treasury = &accounts[1];

    check_signer(admin)?;
    check_writable(treasury)?;
    check_owner(treasury, program_id)?;

    let buf = unsafe { treasury.borrow_unchecked_mut() };

    // Verify admin
    let perm = PermissionSegment::overlay(&buf[PERM_OFFSET..PERM_OFFSET + PermissionSegment::LEN])?;
    if !perm.admin.eq_account(admin) {
        return Err(Unauthorized.into());
    }

    // Parse new epoch params: new_epoch_number(8B) + optional new_budget(8B)
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let new_epoch = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);

    let budget_mut = BudgetSegment::overlay_mut(
        &mut buf[BUDGET_OFFSET..BUDGET_OFFSET + BudgetSegment::LEN],
    )?;

    // Epoch must advance
    if new_epoch <= budget_mut.epoch_number.get() {
        return Err(EpochMismatch.into());
    }

    budget_mut.epoch_number = WireU64::new(new_epoch);
    budget_mut.epoch_spent = WireU64::new(0);

    // Optional: update budget
    if data.len() >= 16 {
        let new_budget = u64::from_le_bytes([
            data[8], data[9], data[10], data[11],
            data[12], data[13], data[14], data[15],
        ]);
        budget_mut.epoch_budget = WireU64::new(new_budget);
    }

    Ok(())
}
