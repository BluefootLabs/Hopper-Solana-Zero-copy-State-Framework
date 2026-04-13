//! # Hopper Migration Example
//!
//! Demonstrates layout version evolution with the Hopper framework.
//!
//! ## What This Shows
//!
//! 1. **V1 layout** -- Original vault with authority + balance
//! 2. **V2 layout** -- Extended vault with authority + balance + bump + last_deposit
//! 3. **Append-only migration** -- Evolve V1 accounts to V2 in place
//! 4. **Migration planner** -- Use `hopper-schema` to generate migration steps
//! 5. **Dual-version loading** -- Accept both V1 and V2 accounts during rollout
//!
//! ## Account Evolution
//!
//! ```text
//! V1 (56 bytes):
//!   [Header: 16B] [authority: 32B] [balance: 8B]
//!
//! V2 (65 bytes):
//!   [Header: 16B] [authority: 32B] [balance: 8B] [bump: 1B] [last_deposit: 8B]
//! ```
//!
//! ## Instructions
//!
//! - `0` = InitV1: create a V1 vault
//! - `1` = MigrateV1ToV2: upgrade a V1 account to V2 (append-only)
//! - `2` = DepositV2: deposit into a V2 vault (shows post-migration usage)
//! - `3` = ReadEither: load either V1 or V2 vault (dual-version pattern)

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;
use hopper::hopper_assert_compatible;
use hopper::hopper_core::account::read_layout_id;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

// =====================================================================
// Step 1: Define Both Layout Versions
// =====================================================================

hopper_layout! {
    /// Vault V1 -- the original layout.
    pub struct VaultV1, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
    }
}

hopper_layout! {
    /// Vault V2 -- extended layout with bump and last_deposit.
    ///
    /// Append-compatible: V1 fields unchanged, new fields appended.
    pub struct VaultV2, disc = 1, version = 2 {
        authority:    TypedAddress<Authority> = 32,
        balance:      WireU64                = 8,
        bump:         u8                     = 1,
        last_deposit: WireU64                = 8,
    }
}

// =====================================================================
// Step 2: Schema Manifests (for migration planner)
// =====================================================================

hopper_manifest! {
    VAULT_V1_MANIFEST = VaultV1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
    }
}

hopper_manifest! {
    VAULT_V2_MANIFEST = VaultV2 {
        authority:    TypedAddress<Authority> = 32,
        balance:      WireU64                = 8,
        bump:         u8                     = 1,
        last_deposit: WireU64                = 8,
    }
}

// =====================================================================
// Step 3: Compile-Time Compatibility Assertion
// =====================================================================

// Verifies at compile time: same disc, V2 larger, distinct layout IDs.
hopper_assert_compatible!(VaultV1, VaultV2, append);

// =====================================================================
// Step 4: Error Codes
// =====================================================================

hopper_error! {
    base = 6100;
    AlreadyMigrated,
    VersionMismatch,
    Unauthorized,
    ZeroAmount,
    InsufficientBalance
}

// =====================================================================
// Step 5: Entrypoint
// =====================================================================

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init_v1,
        1 => process_migrate_v1_to_v2,
        2 => process_deposit_v2,
        3 => process_read_either,
    }
}

// =====================================================================
// Instruction 0: Initialize a V1 Vault
// =====================================================================

fn process_init_v1(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let vault_account = &accounts[1];
    let system_program = &accounts[2];

    require_payer(payer)?;
    check_writable(vault_account)?;

    hopper_init!(payer, vault_account, system_program, program_id, VaultV1)?;

    let mut vault = VaultV1::load_mut(vault_account, program_id)?;
    let vault = vault.get_mut();
    vault.authority = TypedAddress::from_account(payer);
    vault.balance = WireU64::new(0);

    Ok(())
}

// =====================================================================
// Instruction 1: Migrate V1 -> V2 (append-only)
// =====================================================================
//
// This uses `migrate_append` from hopper-core which:
//   1. Validates old layout_id
//   2. Reallocs to new size
//   3. Updates header (version + layout_id)
//   4. Zeroes the appended region
//
// After migration, the caller fills in the new fields.

fn process_migrate_v1_to_v2(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let authority = &accounts[0];
    let vault_account = &accounts[1];

    check_signer(authority)?;

    // Use the schema planner to verify the migration is valid.
    // This is a compile-time computed plan but we verify at runtime.
    let plan = hopper_schema::MigrationPlan::<16>::generate(
        &VAULT_V1_MANIFEST,
        &VAULT_V2_MANIFEST,
    );

    // The plan should be AppendOnly for this version pair
    if plan.policy == hopper_schema::MigrationPolicy::Incompatible {
        return Err(ProgramError::InvalidAccountData);
    }

    // Check the vault is still V1 (not already migrated)
    {
        let account_data = vault_account.try_borrow()?;
        let current_layout = read_layout_id(&account_data)?;
        if current_layout == VaultV2::LAYOUT_ID {
            return Err(AlreadyMigrated.into());
        }
        if current_layout != VaultV1::LAYOUT_ID {
            return Err(VersionMismatch.into());
        }
    }

    // Verify authority before migrating
    {
        let v1 = VaultV1::load(vault_account, program_id)?;
        if !v1.get().authority.eq_account(authority) {
            return Err(Unauthorized.into());
        }
    }

    // Execute the append migration.
    // migrate_append validates ownership, writable, and old layout_id,
    // then reallocs, updates header, and zeroes the new region.
    migrate_append(
        vault_account,
        authority,       // payer for realloc rent
        program_id,
        &VaultV1::LAYOUT_ID,
        VaultV2::VERSION,
        &VaultV2::LAYOUT_ID,
        VaultV2::DISC,
        VaultV2::LEN,
    )?;

    // Fill in new fields.
    // After migrate_append, the header is V2 but new fields are zeroed.
    let mut vault = VaultV2::load_mut(vault_account, program_id)?;
    let vault = vault.get_mut();

    // Parse bump from instruction data if provided (byte 0)
    if !data.is_empty() {
        vault.bump = data[0];
    }
    // last_deposit stays zero (never deposited in V2 yet)

    emit_slices(&[b"vault_migrated_v1_to_v2"]);

    Ok(())
}

// =====================================================================
// Instruction 2: Deposit into V2 Vault
// =====================================================================

fn process_deposit_v2(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 || data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let depositor = &accounts[0];
    let vault_account = &accounts[1];

    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]);
    hopper_require!(amount > 0, ZeroAmount);

    require_payer(depositor)?;
    check_owner(vault_account, program_id)?;
    check_writable(vault_account)?;

    // Load as V2 -- this fails if account hasn't been migrated yet
    let mut vault = VaultV2::load_mut(vault_account, program_id)?;
    let v = vault.get_mut();

    // Transfer SOL: depositor -> vault
    let dep_lamports = depositor.lamports();
    depositor.set_lamports(
        dep_lamports.checked_sub(amount)
            .ok_or(ProgramError::InsufficientFunds)?,
    );
    let vault_lamports = vault_account.lamports();
    vault_account.set_lamports(
        vault_lamports.checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );

    // Update balance and last_deposit
    let new_balance = v.balance.get()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    v.balance = WireU64::new(new_balance);
    v.last_deposit = WireU64::new(amount);

    emit_slices(&[b"deposit_v2"]);

    Ok(())
}

// =====================================================================
// Instruction 3: Read Either Version (Dual-Version Pattern)
// =====================================================================
//
// During a migration rollout, some accounts are V1, some are V2.
// This shows how to handle both versions in a single instruction.

fn process_read_either(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.is_empty() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let vault_account = &accounts[0];
    check_owner(vault_account, program_id)?;

    let account_data = vault_account.try_borrow()?;
    let layout_id = read_layout_id(&account_data)?;

    if layout_id == VaultV2::LAYOUT_ID {
        // V2 path
        let vault = VaultV2::overlay(&account_data)?;
        let _balance = vault.balance.get();
        let _last = vault.last_deposit.get();
        emit_slices(&[b"read_v2"]);
    } else if layout_id == VaultV1::LAYOUT_ID {
        // V1 path -- still works, just no new fields
        let vault = VaultV1::overlay(&account_data)?;
        let _balance = vault.balance.get();
        emit_slices(&[b"read_v1"]);
    } else {
        return Err(ProgramError::InvalidAccountData);
    }

    Ok(())
}

// =====================================================================
// Tests: Migration Plan Verification
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_schema::{MigrationPlan, MigrationPolicy, MigrationAction};

    #[test]
    fn v1_layout_constants() {
        assert_eq!(VaultV1::DISC, 1);
        assert_eq!(VaultV1::VERSION, 1);
        assert_eq!(VaultV1::LEN, 16 + 32 + 8); // 56
    }

    #[test]
    fn v2_layout_constants() {
        assert_eq!(VaultV2::DISC, 1);
        assert_eq!(VaultV2::VERSION, 2);
        assert_eq!(VaultV2::LEN, 16 + 32 + 8 + 1 + 8); // 65
    }

    #[test]
    fn distinct_layout_ids() {
        assert_ne!(VaultV1::LAYOUT_ID, VaultV2::LAYOUT_ID);
    }

    #[test]
    fn migration_plan_is_append_only() {
        let plan = MigrationPlan::<16>::generate(&VAULT_V1_MANIFEST, &VAULT_V2_MANIFEST);
        assert_eq!(plan.policy, MigrationPolicy::AppendOnly);
        assert_eq!(plan.old_size, 56);
        assert_eq!(plan.new_size, 65);
    }

    #[test]
    fn migration_plan_steps() {
        let plan = MigrationPlan::<16>::generate(&VAULT_V1_MANIFEST, &VAULT_V2_MANIFEST);

        // Should have: CopyPrefix + Realloc + ZeroInit(bump) + ZeroInit(last_deposit) + UpdateHeader
        assert!(plan.step_count >= 4);

        // First step: CopyPrefix for the shared fields
        assert_eq!(plan.steps[0].action, MigrationAction::CopyPrefix);
        assert!(plan.copy_bytes > 0);

        // Should have a Realloc step
        let has_realloc = (0..plan.step_count)
            .any(|i| plan.steps[i].action == MigrationAction::Realloc);
        assert!(has_realloc);

        // Should have ZeroInit for new fields
        let zero_count = (0..plan.step_count)
            .filter(|&i| plan.steps[i].action == MigrationAction::ZeroInit)
            .count();
        assert!(zero_count >= 1); // bump and/or last_deposit

        // Last step: UpdateHeader
        assert_eq!(plan.steps[plan.step_count - 1].action, MigrationAction::UpdateHeader);
    }

    #[test]
    fn no_op_same_version() {
        let plan = MigrationPlan::<16>::generate(&VAULT_V1_MANIFEST, &VAULT_V1_MANIFEST);
        assert_eq!(plan.policy, MigrationPolicy::NoOp);
        assert_eq!(plan.step_count, 0);
    }

    #[test]
    fn append_compatible_check() {
        assert!(hopper_schema::is_append_compatible(&VAULT_V1_MANIFEST, &VAULT_V2_MANIFEST));
    }

    #[test]
    fn manifest_field_counts() {
        assert_eq!(VAULT_V1_MANIFEST.field_count, 2);
        assert_eq!(VAULT_V2_MANIFEST.field_count, 4);
    }

    #[test]
    fn field_compat_report() {
        let report = hopper_schema::compare_fields::<16>(&VAULT_V1_MANIFEST, &VAULT_V2_MANIFEST);
        assert!(report.is_append_safe);

        // 2 identical + 2 added
        let identical = report.count_status(hopper_schema::FieldCompat::Identical);
        let added = report.count_status(hopper_schema::FieldCompat::Added);
        assert_eq!(identical, 2);
        assert_eq!(added, 2);
    }
}
