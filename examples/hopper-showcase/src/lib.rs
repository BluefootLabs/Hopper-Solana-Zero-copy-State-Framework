//! # Hopper Showcase -- The Canonical Architecture Path
//!
//! This example demonstrates **the default Hopper way** -- a complete program
//! showing every layer of the framework working together as one coherent system.
//!
//! ## What This Demonstrates
//!
//! 1. **Layout definition** -- `hopper_layout!` with typed addresses and wire types
//! 2. **Tiered account loading** -- T1 (own), T2 (foreign), T4 (unchecked)
//! 3. **Phased execution** -- Resolve → Validate → Execute typestate pipeline
//! 4. **Composable validation** -- ValidationGraph with named rule groups
//! 5. **Policy-aware capabilities** -- Declare what you do, auto-trigger guards
//! 6. **State receipts** -- Before/after fingerprints, segment tracking, policy flags
//! 7. **Invariant checking** -- Post-mutation correctness verification
//! 8. **Segment roles** -- Typed semantic roles for multi-segment accounts
//! 9. **Events** -- Structured event emission with receipts
//! 10. **Error codes** -- Sequential error variants via `hopper_error!`
//! 11. **PDA verification** -- BUMP_OFFSET cached PDA validation
//! 12. **Instruction dispatch** -- `hopper_dispatch!` macro
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                    Pool Account                      │
//! │  [Header 16B]                                        │
//! │  [PoolState: authority, mint, balance, ... 89B]      │
//! │  [PoolConfig: fee_bps, max_deposit, ... 57B]         │
//! └─────────────────────────────────────────────────────┘
//!
//! Instructions:
//!   0 = InitPool     (creates account, writes all segments)
//!   1 = Deposit      (mutates balance, emits receipt)
//!   2 = Withdraw     (policy-gated, invariant-checked)
//!   3 = UpdateConfig (admin-only, config segment mutation)
//! ```

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

// =====================================================================
// Step 1: Define Layouts
// =====================================================================
//
// `hopper_layout!` generates:
//   - #[repr(C)] struct with alignment-1 wire types
//   - Compile-time LAYOUT_ID (SHA-256 fingerprint)
//   - Tiered load functions: load(), load_cross_program(), load_unverified()
//   - SIZE, LEN, DISC, VERSION constants
//   - load()/load_mut() for canonical whole-layout access; overlay() for raw projection
//   - BUMP_OFFSET for PDA verification

hopper_layout! {
    /// Pool state -- primary data segment.
    pub struct PoolState, disc = 1, version = 1 {
        authority:     TypedAddress<Authority>  = 32,
        mint:          TypedAddress<Mint>       = 32,
        total_deposit: WireU64                  = 8,
        total_withdrawn: WireU64               = 8,
        deposit_count: WireU32                  = 4,
        bump:          u8                       = 1,
    }
}

hopper_layout! {
    /// Pool configuration -- admin-controlled parameters.
    pub struct PoolConfig, disc = 2, version = 1 {
        admin:         TypedAddress<Authority>  = 32,
        fee_bps:       WireU16                  = 2,
        max_deposit:   WireU64                  = 8,
        frozen:        WireBool                 = 1,
    }
}

/// Full pool account size: both segments packed contiguously.
const POOL_SIZE: usize = PoolState::LEN + PoolConfig::LEN;
const STATE_OFFSET: usize = 0;
const CONFIG_OFFSET: usize = PoolState::LEN;

// =====================================================================
// Step 2: Error Codes
// =====================================================================

hopper_error! {
    base = 6000;
    PoolFrozen,
    UnauthorizedAdmin,
    DepositExceedsMax,
    InsufficientPoolBalance,
    ZeroAmount,
    BalanceInvariantViolation
}

// =====================================================================
// Step 3: Disc Registry (compile-time uniqueness check)
// =====================================================================

hopper_register_discs! {
    PoolState,
    PoolConfig,
}

// =====================================================================
// Step 4: Define Policy -- capability-requirement binding
// =====================================================================
//
// Hopper ships named policy packs for common instruction patterns.
// Use them directly (as shown here) or build custom ones for your protocol.
//
// TREASURY_WRITE_POLICY/CAPS -- for mutations that touch balances/vault
// AUTHORITY_CHANGE_POLICY/CAPS -- for admin/permission changes
//
// When an instruction declares capabilities, the policy resolves which
// validation requirements must be met. This is evaluated at const time.

/// Deposit and withdraw both touch treasury balances. Use the built-in pack.
const DEPOSIT_CAPS: CapabilitySet = TREASURY_WRITE_CAPS;
const WITHDRAW_CAPS: CapabilitySet = TREASURY_WRITE_CAPS;

/// UpdateConfig modifies authority-controlled data. Use the built-in pack.
const CONFIG_CAPS: CapabilitySet = AUTHORITY_CHANGE_CAPS;

// =====================================================================
// Step 5: Entrypoint + Dispatch
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
        0 => process_init_pool,
        1 => process_deposit,
        2 => process_withdraw,
        3 => process_update_config,
    }
}

// =====================================================================
// Step 6: InitPool -- account creation with typed layout
// =====================================================================
//
// Accounts: [0] payer (signer, writable), [1] pool (writable), [2] system
// Data: mint(32B)

fn process_init_pool(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let pool = &accounts[1];
    let system = &accounts[2];

    check_signer(payer)?;
    check_writable(pool)?;

    if data.len() < 32 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mint_bytes: &[u8; 32] = data[0..32]
        .try_into()
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    // Create account
    let rent = rent_exempt_min(POOL_SIZE);
    hopper::hopper_system::CreateAccount {
        from: payer,
        to: pool,
        lamports: rent,
        space: POOL_SIZE as u64,
        owner: program_id,
    }
    .invoke()?;

    let mut buf = pool.try_borrow_mut()?;
    zero_init(&mut buf);

    // Init PoolState segment
    let state_slice = &mut buf[STATE_OFFSET..STATE_OFFSET + PoolState::LEN];
    PoolState::write_init_header(state_slice)?;
    let state = PoolState::overlay_mut(state_slice)?;
    state.authority = TypedAddress::from_account(payer);
    state.mint = TypedAddress::from_slice(mint_bytes);
    state.total_deposit = WireU64::new(0);
    state.total_withdrawn = WireU64::new(0);
    state.deposit_count = WireU32::new(0);

    // Init PoolConfig segment
    let config_slice = &mut buf[CONFIG_OFFSET..CONFIG_OFFSET + PoolConfig::LEN];
    PoolConfig::write_init_header(config_slice)?;
    let config = PoolConfig::overlay_mut(config_slice)?;
    config.admin = TypedAddress::from_account(payer);
    config.fee_bps = WireU16::new(100); // 1% default fee
    config.max_deposit = WireU64::new(u64::MAX); // no limit
    config.frozen = WireBool::new(false);

    Ok(())
}

// =====================================================================
// Step 7: Deposit -- mutation with receipt + invariants
// =====================================================================
//
// Accounts: [0] depositor (signer), [1] pool (writable)
// Data: amount(8B)
//
// This shows the full Hopper pattern:
//   1. Resolve policy requirements
//   2. Validate accounts
//   3. Snapshot state (for receipt)
//   4. Mutate
//   5. Run invariants
//   6. Emit receipt

fn process_deposit(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let depositor = &accounts[0];
    let pool = &accounts[1];

    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);

    // -- Resolve policy requirements from the named pack --
    let _reqs = TREASURY_WRITE_POLICY.resolve(&DEPOSIT_CAPS);
    // reqs.has(PolicyRequirement::Authority)          → true
    // reqs.has(PolicyRequirement::InvariantCheck)      → true
    // reqs.has(PolicyRequirement::LamportConservation) → true
    // reqs.has(PolicyRequirement::StateSnapshot)       → true

    // -- Validate --
    check_signer(depositor)?;
    check_owner(pool, program_id)?;
    check_writable(pool)?;

    let mut buf = pool.try_borrow_mut()?;

    // Read config to check frozen status and max deposit
    let config = PoolConfig::overlay(&buf[CONFIG_OFFSET..CONFIG_OFFSET + PoolConfig::LEN])?;
    if config.frozen.get() {
        return Err(PoolFrozen.into());
    }
    if amount == 0 {
        return Err(ZeroAmount.into());
    }
    if amount > config.max_deposit.get() {
        return Err(DepositExceedsMax.into());
    }

    // -- Snapshot state (for receipt) --
    let mut receipt = StateReceipt::<256>::begin(&PoolState::LAYOUT_ID, &buf);

    // -- Mutate --
    let state = PoolState::overlay_mut(&mut buf[STATE_OFFSET..STATE_OFFSET + PoolState::LEN])?;
    let new_total = checked_add(state.total_deposit.get(), amount)?;
    state.total_deposit = WireU64::new(new_total);
    let new_count = checked_add(state.deposit_count.get() as u64, 1)?;
    state.deposit_count = WireU32::new(new_count as u32);

    // -- Invariant check --
    let mut invariants = InvariantSet::new();
    invariants.check(
        state.total_deposit.get() >= state.total_withdrawn.get(),
        BalanceInvariantViolation::CODE,
    );
    let inv_passed = invariants.all_passed();
    let inv_count = invariants.checked_count();
    invariants.finalize()?;

    // -- Commit receipt with segment tracking --
    let segments: &[(usize, usize)] = &[
        (STATE_OFFSET, PoolState::LEN),
        (CONFIG_OFFSET, PoolConfig::LEN),
    ];
    receipt.commit_with_segments(&buf, segments);
    receipt.set_invariants(inv_passed, inv_count);
    receipt.set_policy_flags(DEPOSIT_CAPS.bits());

    // -- Emit receipt as event --
    let receipt_bytes = receipt.to_bytes();
    emit_slices(&[&receipt_bytes]);

    Ok(())
}

// =====================================================================
// Step 8: Withdraw -- full policy-gated path
// =====================================================================
//
// Accounts: [0] authority (signer), [1] pool (writable)
// Data: amount(8B)

fn process_withdraw(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let authority = &accounts[0];
    let pool = &accounts[1];

    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);

    // -- Resolve policy from the named pack --
    let reqs = TREASURY_WRITE_POLICY.resolve(&WITHDRAW_CAPS);

    // -- Validate --
    check_signer(authority)?;
    check_owner(pool, program_id)?;
    check_writable(pool)?;

    let mut buf = pool.try_borrow_mut()?;
    let state_ref = PoolState::overlay(&buf[STATE_OFFSET..STATE_OFFSET + PoolState::LEN])?;

    // Authority check (required by policy)
    check_has_one(state_ref.authority.as_bytes(), authority)?;

    let config = PoolConfig::overlay(&buf[CONFIG_OFFSET..CONFIG_OFFSET + PoolConfig::LEN])?;
    if config.frozen.get() {
        return Err(PoolFrozen.into());
    }
    if amount == 0 {
        return Err(ZeroAmount.into());
    }

    let current_balance = state_ref
        .total_deposit
        .get()
        .checked_sub(state_ref.total_withdrawn.get())
        .ok_or(ProgramError::from(InsufficientPoolBalance))?;

    if amount > current_balance {
        return Err(InsufficientPoolBalance.into());
    }

    // -- Snapshot + Mutate --
    let mut receipt = StateReceipt::<256>::begin(&PoolState::LAYOUT_ID, &buf);

    let state = PoolState::overlay_mut(&mut buf[STATE_OFFSET..STATE_OFFSET + PoolState::LEN])?;
    let new_withdrawn = checked_add(state.total_withdrawn.get(), amount)?;
    state.total_withdrawn = WireU64::new(new_withdrawn);

    // -- Invariant check (required by policy) --
    let mut invariants = InvariantSet::new();
    invariants.check(
        state.total_deposit.get() >= state.total_withdrawn.get(),
        BalanceInvariantViolation::CODE,
    );
    let inv_passed = invariants.all_passed();
    let inv_count = invariants.checked_count();
    invariants.finalize()?;

    // -- Receipt with segment tracking --
    let segments: &[(usize, usize)] = &[
        (STATE_OFFSET, PoolState::LEN),
        (CONFIG_OFFSET, PoolConfig::LEN),
    ];
    receipt.commit_with_segments(&buf, segments);
    receipt.set_invariants(inv_passed, inv_count);
    receipt.set_policy_flags(WITHDRAW_CAPS.bits());
    let receipt_bytes = receipt.to_bytes();
    emit_slices(&[&receipt_bytes]);

    Ok(())
}

// =====================================================================
// Step 9: UpdateConfig -- admin-gated config mutation
// =====================================================================
//
// Accounts: [0] admin (signer), [1] pool (writable)
// Data: new_fee_bps(2B) + new_max_deposit(8B) + frozen(1B)

fn process_update_config(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let admin = &accounts[0];
    let pool = &accounts[1];

    if data.len() < 11 {
        return Err(ProgramError::InvalidInstructionData);
    }

    check_signer(admin)?;
    check_owner(pool, program_id)?;
    check_writable(pool)?;

    let mut buf = pool.try_borrow_mut()?;

    // Verify admin authority
    let config_ref = PoolConfig::overlay(&buf[CONFIG_OFFSET..CONFIG_OFFSET + PoolConfig::LEN])?;
    check_has_one(config_ref.admin.as_bytes(), admin)?;

    // Parse params
    let new_fee_bps = u16::from_le_bytes([data[0], data[1]]);
    let new_max_deposit = u64::from_le_bytes([
        data[2], data[3], data[4], data[5], data[6], data[7], data[8], data[9],
    ]);
    let frozen = data[10] != 0;

    // -- Snapshot config (for receipt) --
    let mut receipt = StateReceipt::<256>::begin(&PoolConfig::LAYOUT_ID, &buf);

    // Mutate config
    let config = PoolConfig::overlay_mut(&mut buf[CONFIG_OFFSET..CONFIG_OFFSET + PoolConfig::LEN])?;
    config.fee_bps = WireU16::new(new_fee_bps);
    config.max_deposit = WireU64::new(new_max_deposit);
    config.frozen = WireBool::new(frozen);

    // -- Receipt --
    let segments: &[(usize, usize)] = &[
        (STATE_OFFSET, PoolState::LEN),
        (CONFIG_OFFSET, PoolConfig::LEN),
    ];
    receipt.commit_with_segments(&buf, segments);
    receipt.set_policy_flags(CONFIG_CAPS.bits());
    let receipt_bytes = receipt.to_bytes();
    emit_slices(&[&receipt_bytes]);

    Ok(())
}
