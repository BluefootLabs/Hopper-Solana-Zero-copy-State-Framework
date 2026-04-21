//! Segment roles -- typed semantic classification for account segments.
//!
//! Each segment in a Hopper segmented account can be assigned a role that
//! conveys its purpose to tooling, migration planners, and runtime guards.
//!
//! ## Roles
//!
//! | Role      | Description                              | Writability | Migration |
//! |-----------|------------------------------------------|-------------|-----------|
//! | Core      | Primary fixed-layout state               | Read/Write  | Must copy |
//! | Extension | Optional fields appended in later version| Read/Write  | Append-safe|
//! | Journal   | Append-only audit trail (`Journal<T>`)   | Append-only | Clearable |
//! | Index     | Lookup index (SortedVec, SlotMap)         | Read/Write  | Rebuildable|
//! | Cache     | Derived/computed values, can be rebuilt   | Read/Write  | Droppable |
//! | Audit     | Immutable audit log, locked after init    | Read-only   | Must copy |
//! | Shard     | Part of a sharded collection              | Read/Write  | Redistributable|
//!
//! ## Wire Encoding
//!
//! The role is encoded in the **upper 4 bits** of the segment entry `flags` field
//! (bits 12-15 of the u16 flags). The lower 12 bits remain available for
//! per-segment flags (`LOCKED`, `FROZEN`, `DYNAMIC`, etc.).
//!
//! ```text
//! flags [u16 LE]:
//!   bits 0-2:   LOCKED | FROZEN | DYNAMIC (existing)
//!   bits 3-11:  reserved for future per-segment flags
//!   bits 12-15: SegmentRole (0-7)
//! ```

/// Segment role classification.
///
/// Encoded as a 4-bit value (0-15). Currently 8 roles defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SegmentRole {
    /// Primary fixed-layout state. Must be preserved across migrations.
    Core = 0,
    /// Optional extension fields added in later versions.
    Extension = 1,
    /// Append-only audit trail (`Journal<T>`). Can be cleared on migration.
    Journal = 2,
    /// Lookup index (`SortedVec`, `SlotMap`). Rebuildable from core data.
    Index = 3,
    /// Derived/computed cache. Safe to drop and rebuild.
    Cache = 4,
    /// Immutable audit log. Locked after initialization.
    Audit = 5,
    /// Part of a sharded collection. May be redistributed on rebalance.
    Shard = 6,
    /// Unclassified segment (legacy compatibility).
    Unclassified = 7,
}

impl SegmentRole {
    /// Decode role from segment flags (upper 4 bits).
    #[inline(always)]
    pub const fn from_flags(flags: u16) -> Self {
        match (flags >> 12) & 0xF {
            0 => Self::Core,
            1 => Self::Extension,
            2 => Self::Journal,
            3 => Self::Index,
            4 => Self::Cache,
            5 => Self::Audit,
            6 => Self::Shard,
            _ => Self::Unclassified,
        }
    }

    /// Encode role into segment flags (preserving lower 12 bits).
    #[inline(always)]
    pub const fn into_flags(self, existing_flags: u16) -> u16 {
        (existing_flags & 0x0FFF) | ((self as u16) << 12)
    }

    /// Whether this segment must be preserved during migration.
    #[inline(always)]
    pub const fn must_preserve(&self) -> bool {
        matches!(*self, Self::Core | Self::Audit)
    }

    /// Whether this segment can safely be cleared on migration.
    #[inline(always)]
    pub const fn clearable_on_migration(&self) -> bool {
        matches!(*self, Self::Journal | Self::Cache)
    }

    /// Whether this segment can be rebuilt from other data.
    #[inline(always)]
    pub const fn rebuildable(&self) -> bool {
        matches!(*self, Self::Index | Self::Cache)
    }

    /// Whether this segment should be append-only at runtime.
    #[inline(always)]
    pub const fn is_append_only(&self) -> bool {
        matches!(*self, Self::Journal | Self::Audit)
    }

    /// Whether writes to this segment should be rejected after init.
    #[inline(always)]
    pub const fn is_immutable_after_init(&self) -> bool {
        matches!(*self, Self::Audit)
    }

    /// Whether this segment's data must be copied during migration.
    ///
    /// Core and Audit segments contain irreplaceable state that cannot
    /// be rebuilt or cleared, their bytes must survive migration intact.
    #[inline(always)]
    pub const fn requires_migration_copy(&self) -> bool {
        matches!(*self, Self::Core | Self::Audit)
    }

    /// Whether this segment can be safely dropped (zeroed) without data loss.
    ///
    /// Cache segments hold derived/computed values that can be rebuilt
    /// from other on-chain state. Dropping them is always safe.
    #[inline(always)]
    pub const fn is_safe_to_drop(&self) -> bool {
        matches!(*self, Self::Cache)
    }

    /// Whether mutations to this segment should generate a receipt entry.
    ///
    /// Core, Extension, and Shard mutations are always receipt-worthy.
    /// Journal and Audit appends are also receipt-worthy.
    /// Cache and Index rebuilds typically are not.
    #[inline(always)]
    pub const fn should_emit_receipt(&self) -> bool {
        matches!(*self, Self::Core | Self::Extension | Self::Journal | Self::Audit | Self::Shard)
    }

    /// Whether this segment is relevant to operator dashboards and Manager output.
    ///
    /// Core, Audit, and Journal segments carry meaningful business state.
    /// Cache and Index are derived and typically hidden from operators.
    #[inline(always)]
    pub const fn is_operator_relevant(&self) -> bool {
        matches!(*self, Self::Core | Self::Extension | Self::Journal | Self::Audit | Self::Shard)
    }

    /// Whether this segment potentially holds financial state.
    ///
    /// Core and Extension segments may contain balance/treasury fields.
    /// Other segments (Journal, Cache, Index) typically hold derived data.
    #[inline(always)]
    pub const fn may_hold_financial_state(&self) -> bool {
        matches!(*self, Self::Core | Self::Extension)
    }

    /// Human-readable role name (for schema export and tooling).
    #[inline(always)]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Extension => "extension",
            Self::Journal => "journal",
            Self::Index => "index",
            Self::Cache => "cache",
            Self::Audit => "audit",
            Self::Shard => "shard",
            Self::Unclassified => "unclassified",
        }
    }
}

/// Segment role flags -- convenience constants for `SegmentEntry::new()`.
pub const SEG_ROLE_CORE: u16 = (SegmentRole::Core as u16) << 12;
pub const SEG_ROLE_EXTENSION: u16 = (SegmentRole::Extension as u16) << 12;
pub const SEG_ROLE_JOURNAL: u16 = (SegmentRole::Journal as u16) << 12;
pub const SEG_ROLE_INDEX: u16 = (SegmentRole::Index as u16) << 12;
pub const SEG_ROLE_CACHE: u16 = (SegmentRole::Cache as u16) << 12;
pub const SEG_ROLE_AUDIT: u16 = (SegmentRole::Audit as u16) << 12;
pub const SEG_ROLE_SHARD: u16 = (SegmentRole::Shard as u16) << 12;
