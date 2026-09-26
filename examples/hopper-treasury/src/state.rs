use hopper::prelude::*;
use hopper::systems::*;

hopper_layout! {
    /// Core treasury data.
    pub struct TreasuryCore, disc = 10, version = 2 {
        authority:        TypedAddress<Authority>  = 32,
        total_deposited:  WireU64                  = 8,
    }
}

// Permission segment follows core.
hopper_layout! {
    /// Permission and access control segment.
    pub struct PermissionSegment, disc = 11, version = 2 {
        operator:               TypedAddress<Authority>  = 32,
        frozen:                 WireBool                 = 1,
        max_single_withdrawal:  WireU64                  = 8,
    }
}

// Budget segment follows permissions.
hopper_layout! {
    /// Per-epoch budget tracking segment.
    pub struct BudgetSegment, disc = 12, version = 2 {
        epoch_budget:       WireU64  = 8,
        epoch_spent:        WireU64  = 8,
        epoch_number:       WireU64  = 8,
        cooldown_seconds:   WireU64  = 8,
        last_withdrawal_ts: WireU64  = 8,
    }
}

// The full treasury account size: we pack all segments contiguously.
// Core (56) + Permissions (57) + Budget (56) = 169 bytes
pub const TREASURY_ACCOUNT_SIZE: usize =
    TreasuryCore::LEN + PermissionSegment::LEN + BudgetSegment::LEN;

// Segment offsets
pub const CORE_OFFSET: usize = 0;
pub const PERM_OFFSET: usize = TreasuryCore::LEN;
pub const BUDGET_OFFSET: usize = TreasuryCore::LEN + PermissionSegment::LEN;
