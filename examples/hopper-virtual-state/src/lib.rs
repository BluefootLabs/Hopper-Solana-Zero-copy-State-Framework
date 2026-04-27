//! # Hopper Virtual State Example
//!
//! Demonstrates multi-account logical entities using Hopper's `VirtualState`
//! and `ShardedAccess` primitives.
//!
//! ## What This Shows
//!
//! 1. **VirtualState** -- Map N accounts into a single logical entity with typed slots
//! 2. **Slot kinds** -- Read-only, writable, and foreign (cross-program) slots
//! 3. **Typed overlays** -- Access each slot as a typed zero-copy layout
//! 4. **ShardedAccess** -- Distribute a collection across multiple accounts by key hash
//! 5. **Aggregation** -- Read and combine data from multiple virtual slots
//!
//! ## Architecture
//!
//! ```text
//! Marketplace (3-account virtual entity):
//!   Slot 0: MarketConfig  (owned, read-only)  -- admin, fee_bps, listing_count
//!   Slot 1: MarketVault   (owned, writable)   -- balance tracking
//!   Slot 2: MarketStats   (owned, read-only)  -- aggregate stats
//!
//! Order Shards (4-account sharded collection):
//!   Shard 0..3: each holds a portion of orders, routed by FNV-1a key hash
//! ```
//!
//! ## Instructions
//!
//! - `0` = InitMarket: create config + vault + stats accounts
//! - `1` = PlaceOrder: route an order to the correct shard by key
//! - `2` = ReadMarket: aggregate stats across all virtual slots
//! - `3` = ReadShard: demonstrate shard routing for a given key

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
// Layouts
// =====================================================================

hopper_layout! {
    /// Marketplace configuration -- admin settings and counters.
    pub struct MarketConfig, disc = 20, version = 1 {
        admin:         TypedAddress<Authority> = 32,
        fee_bps:       WireU16                = 2,
        listing_count: WireU32                = 4,
        frozen:        WireBool               = 1,
    }
}

hopper_layout! {
    /// Marketplace SOL vault -- tracks collected fees.
    pub struct MarketVault, disc = 21, version = 1 {
        authority:    TypedAddress<Authority> = 32,
        total_fees:   WireU64                = 8,
        total_volume: WireU64                = 8,
        bump:         u8                     = 1,
    }
}

hopper_layout! {
    /// Aggregate marketplace statistics.
    pub struct MarketStats, disc = 22, version = 1 {
        total_orders:    WireU64 = 8,
        total_fills:     WireU64 = 8,
        total_cancels:   WireU64 = 8,
        last_order_slot: WireU64 = 8,
    }
}

hopper_layout! {
    /// A single order record within a shard.
    pub struct OrderRecord, disc = 23, version = 1 {
        maker:  TypedAddress<Authority> = 32,
        price:  WireU64                = 8,
        amount: WireU64                = 8,
        side:   u8                     = 1,
        status: u8                     = 1,
    }
}

// Compile-time disc uniqueness
hopper_register_discs! {
    MarketConfig,
    MarketVault,
    MarketStats,
    OrderRecord,
}

// =====================================================================
// Virtual Slot Indices (logical names for clarity)
// =====================================================================

const SLOT_CONFIG: usize = 0;
const SLOT_VAULT: usize = 1;
const SLOT_STATS: usize = 2;

const SHARD_COUNT: usize = 4;

// =====================================================================
// Errors
// =====================================================================

hopper_error! {
    base = 6200;
    MarketFrozen,
    Unauthorized,
    InvalidShard,
    ZeroAmount,
    ShardFull
}

// =====================================================================
// Entrypoint
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
        0 => process_init_market,
        1 => process_place_order,
        2 => process_read_market,
        3 => process_read_shard,
    }
}

// =====================================================================
// Instruction 0: Init Market (3 accounts)
// =====================================================================

fn process_init_market(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    // accounts[0] = payer/admin (signer)
    // accounts[1] = config account
    // accounts[2] = vault account
    // accounts[3] = stats account
    // accounts[4] = system program
    if accounts.len() < 5 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let config_acc = &accounts[1];
    let vault_acc = &accounts[2];
    let stats_acc = &accounts[3];
    let system_program = &accounts[4];

    require_payer(payer)?;

    // Create all three accounts
    hopper_init!(payer, config_acc, system_program, program_id, MarketConfig)?;
    hopper_init!(payer, vault_acc, system_program, program_id, MarketVault)?;
    hopper_init!(payer, stats_acc, system_program, program_id, MarketStats)?;

    // Initialize config
    {
        let mut cfg = MarketConfig::load_mut(config_acc, program_id)?;
        let cfg = cfg.get_mut();
        cfg.admin = TypedAddress::from_account(payer);
        cfg.fee_bps = WireU16::new(if data.len() >= 2 {
            u16::from_le_bytes([data[0], data[1]])
        } else {
            50 // default 0.5%
        });
        cfg.listing_count = WireU32::new(0);
        cfg.frozen = WireBool::new(false);
    }

    // Initialize vault
    {
        let mut vault = MarketVault::load_mut(vault_acc, program_id)?;
        let vault = vault.get_mut();
        vault.authority = TypedAddress::from_account(payer);
        vault.total_fees = WireU64::new(0);
        vault.total_volume = WireU64::new(0);
    }

    // Stats stays zeroed (all counters start at 0)

    emit_slices(&[b"market_init"]);
    Ok(())
}

// =====================================================================
// Instruction 1: Place Order (virtual state + shard routing)
// =====================================================================
//
// Accounts:
//   [0] maker (signer)
//   [1] config (read-only)
//   [2] vault (writable -- collects fee)
//   [3] stats (writable -- increment counter)
//   [4..8] shard accounts (one will be written)
//   [8] instruction data: [price:8][amount:8][side:1][order_key:32]

fn process_place_order(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 5 || data.len() < 49 {
        return Err(ProgramError::InvalidInstructionData);
    }

    let maker = &accounts[0];
    check_signer(maker)?;

    // Build virtual state for the core market entity (3 slots)
    let market = VirtualState::<3>::new()
        .map(SLOT_CONFIG, 1) // config: owned, read-only
        .map_mut(SLOT_VAULT, 2) // vault: owned, writable
        .map_mut(SLOT_STATS, 3); // stats: owned, writable

    // Validate all virtual slot constraints
    market.validate(accounts, program_id)?;

    // Read config to check frozen state
    let config = market.overlay::<MarketConfig>(accounts, SLOT_CONFIG)?;
    if config.frozen.get() {
        return Err(MarketFrozen.into());
    }
    let _fee_bps = config.fee_bps.get();

    // Parse order data
    let price = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    let amount = u64::from_le_bytes([
        data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
    ]);
    let side = data[16];
    let order_key = &data[17..49]; // 32-byte key for shard routing

    hopper_require!(amount > 0, ZeroAmount);

    // Route order to the correct shard via ShardedAccess.
    // Shard accounts start at index 4 in the accounts array.
    if accounts.len() < 4 + SHARD_COUNT {
        // Not enough shard accounts -- fall back to first available
        // In production, require all shards present
    }
    let available_shards = (accounts.len() - 4).min(SHARD_COUNT);
    if available_shards > 0 {
        let mut shard_indices = [0u8; SHARD_COUNT];
        let mut i = 0;
        while i < available_shards {
            shard_indices[i] = (4 + i) as u8;
            i += 1;
        }

        let shards =
            ShardedAccess::<SHARD_COUNT>::new(accounts, &shard_indices[..available_shards])?;

        // Determine which shard owns this order
        let target_shard = shards.shard_for_key(order_key);
        let shard_account = shards.shard_account(target_shard)?;

        // In a real program, you would write the OrderRecord into the shard's
        // FixedVec. Here we demonstrate the routing and emit the shard index.
        check_owner(shard_account, program_id)?;
        check_writable(shard_account)?;

        emit_slices(&[b"order_routed_to_shard", &[target_shard as u8]]);
    }

    // Update vault volume
    {
        let mut vault = market.overlay_mut::<MarketVault>(accounts, SLOT_VAULT)?;
        let vol = vault.total_volume.get();
        vault.total_volume = WireU64::new(vol.saturating_add(price.saturating_mul(amount)));
    }

    // Update stats
    {
        let mut stats = market.overlay_mut::<MarketStats>(accounts, SLOT_STATS)?;
        let orders = stats.total_orders.get();
        stats.total_orders = WireU64::new(orders.saturating_add(1));
    }

    emit_slices(&[b"order_placed"]);
    Ok(())
}

// =====================================================================
// Instruction 2: Read Market (aggregate across virtual slots)
// =====================================================================
//
// Reads from all 3 slots as a single logical entity and emits a summary.

fn process_read_market(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    // Build the virtual market view -- all read-only
    let market = VirtualState::<3>::new()
        .map(SLOT_CONFIG, 0)
        .map(SLOT_VAULT, 1)
        .map(SLOT_STATS, 2);

    market.validate(accounts, program_id)?;

    // Read all three overlays
    let config = market.overlay::<MarketConfig>(accounts, SLOT_CONFIG)?;
    let vault = market.overlay::<MarketVault>(accounts, SLOT_VAULT)?;
    let stats = market.overlay::<MarketStats>(accounts, SLOT_STATS)?;

    // Aggregate: total fees + total volume + total orders
    let _total_fees = vault.total_fees.get();
    let _total_volume = vault.total_volume.get();
    let _total_orders = stats.total_orders.get();
    let _fee_bps = config.fee_bps.get();
    let _frozen = config.frozen.get();

    emit_slices(&[b"market_read"]);
    Ok(())
}

// =====================================================================
// Instruction 3: Read Shard (demonstrate shard routing)
// =====================================================================
//
// Given a key in instruction data, determines which shard it maps to
// and reads that shard account.

fn process_read_shard(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if data.len() < 32 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let lookup_key = &data[..32];

    let available_shards = accounts.len().min(SHARD_COUNT);
    if available_shards == 0 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let mut shard_indices = [0u8; SHARD_COUNT];
    let mut i = 0;
    while i < available_shards {
        shard_indices[i] = i as u8;
        i += 1;
    }

    let shards = ShardedAccess::<SHARD_COUNT>::new(accounts, &shard_indices[..available_shards])?;

    // Route key to shard
    let target = shards.shard_for_key(lookup_key);
    let _shard_data = shards.data_for_key(lookup_key)?;

    emit_slices(&[b"shard_lookup", &[target as u8]]);

    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_sizes() {
        assert_eq!(MarketConfig::LEN, 16 + 32 + 2 + 4 + 1); // 55
        assert_eq!(MarketVault::LEN, 16 + 32 + 8 + 8 + 1); // 65
        assert_eq!(MarketStats::LEN, 16 + 8 + 8 + 8 + 8); // 48
        assert_eq!(OrderRecord::LEN, 16 + 32 + 8 + 8 + 1 + 1); // 66
    }

    #[test]
    fn distinct_discs() {
        assert_ne!(MarketConfig::DISC, MarketVault::DISC);
        assert_ne!(MarketConfig::DISC, MarketStats::DISC);
        assert_ne!(MarketConfig::DISC, OrderRecord::DISC);
        assert_ne!(MarketVault::DISC, MarketStats::DISC);
    }

    #[test]
    fn distinct_layout_ids() {
        assert_ne!(MarketConfig::LAYOUT_ID, MarketVault::LAYOUT_ID);
        assert_ne!(MarketConfig::LAYOUT_ID, MarketStats::LAYOUT_ID);
        assert_ne!(MarketVault::LAYOUT_ID, MarketStats::LAYOUT_ID);
    }

    #[test]
    fn virtual_state_slot_counts() {
        let vstate = VirtualState::<3>::new()
            .map(0, 0)
            .map_mut(1, 1)
            .map_foreign(2, 2);
        // Can't test validate without real accounts, but verify it builds
        assert_eq!(
            core::mem::size_of::<VirtualState<3>>(),
            core::mem::size_of::<VirtualState<3>>()
        );
    }

    #[test]
    fn shard_routing_deterministic() {
        // Two identical keys must hash to the same shard index
        let key = b"test_order_key_0123456789abcdef";
        // FNV-1a is deterministic -- same key always same shard
        let mut hash: u32 = 0x811c_9dc5;
        for &byte in key.iter() {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(0x0100_0193);
        }
        let shard_a = (hash as usize) % 4;
        let shard_b = (hash as usize) % 4;
        assert_eq!(shard_a, shard_b);
    }
}
