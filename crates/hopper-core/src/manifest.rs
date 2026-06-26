//! Tier 2 of the three-tier metadata model: the on-chain program
//! registry.
//!
//! See [`docs/THREE_TIER_METADATA.md`](../../../docs/THREE_TIER_METADATA.md)
//! for the full picture. In short:
//!
//! - **Tier 1** is the hot path: compact accounts `[disc:u8][body]`
//!   (see [`hopper_runtime::compact`]).
//! - **Tier 2** (this module) is one optional PDA per program holding a
//!   compact, zero-copy, `no_std`-readable registry of every account
//!   layout: discriminator → (version, sizes, layout_id, flags, name
//!   hash) plus deterministic schema/registry hashes.
//! - **Tier 3** is off-chain generated metadata (JSON manifest, IDL,
//!   SDKs) in `hopper-schema`.
//!
//! The registry is a sibling of the JSON manifest PDA that
//! `hopper-schema` stores at `b"hopper:manifest"`; it lives at its own
//! [`REGISTRY_SEED`] so the binary, hot-path-readable form never
//! clobbers the JSON publication form.
//!
//! Every multi-byte field uses an alignment-1 wire integer, so both
//! [`ProgramManifestHeader`] and [`AccountLayoutEntry`] are `Pod` and
//! overlay directly on raw account bytes at any offset.

use crate::abi::{WireU16, WireU32, WireU64};
use crate::account::{FixedLayout, Pod, Zeroable};
use hopper_runtime::error::ProgramError;

/// PDA seed for the on-chain binary registry account.
///
/// Distinct from `hopper_schema::MANIFEST_SEED` (`b"hopper:manifest"`,
/// the JSON manifest PDA) to avoid colliding with that account. The
/// registry PDA is derived as
/// `find_program_address(&[REGISTRY_SEED, program_id], program_id)`.
pub const REGISTRY_SEED: &[u8] = b"hopper:registry";

/// 8-byte magic at the start of a binary registry account.
pub const REGISTRY_MAGIC: [u8; 8] = *b"HOPRREG1";

/// Current binary registry wire-format version.
pub const REGISTRY_VERSION: u16 = 1;

// ── Header flags ─────────────────────────────────────────────────────

/// Header flag: the program gates upgrades/migrations on this registry
/// (the `governed` manifest profile).
pub const HEADER_FLAG_GOVERNED: u32 = 1 << 0;
/// Header flag: `schema_hash` is populated and pins an off-chain schema.
pub const HEADER_FLAG_HAS_SCHEMA_HASH: u32 = 1 << 1;

// ── Entry flags ──────────────────────────────────────────────────────

/// Entry flag: this account uses the Tier-1 compact layout (`[disc][body]`).
pub const ENTRY_FLAG_COMPACT: u32 = 1 << 0;
/// Entry flag: this account uses the 16-byte `HopperHeader`.
pub const ENTRY_FLAG_HEADERED: u32 = 1 << 1;
/// Entry flag: this account carries a variable-length tail.
pub const ENTRY_FLAG_DYNAMIC_TAIL: u32 = 1 << 2;
/// Entry flag: this layout is deprecated (readable, not writable).
pub const ENTRY_FLAG_DEPRECATED: u32 = 1 << 3;

// ══════════════════════════════════════════════════════════════════════
//  Deterministic FNV-1a-64 hashing (feature-independent)
// ══════════════════════════════════════════════════════════════════════
//
// The crate-level `__fnv_expand_const` is gated to the non-`sha2` build.
// The registry needs a hash that is identical across feature flags (it is
// written on-chain and verified by foreign readers), so it carries its own
// small FNV-1a-64 implementation with the same block-expansion scheme.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a-64 of `bytes` mixed with a per-block differentiating seed.
#[inline]
const fn fnv1a64(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET ^ seed;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    h
}

/// Expand `bytes` into a 32-byte deterministic digest (4 FNV-1a-64
/// blocks, each with a differentiating seed). Matches the scheme used by
/// the crate-level layout-id fallback so on-chain and off-chain agree.
#[inline]
pub const fn expand_hash(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut block: u8 = 0;
    while block < 4 {
        let seed = (block as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let le = fnv1a64(seed, bytes).to_le_bytes();
        let base = (block as usize) * 8;
        let mut j = 0;
        while j < 8 {
            out[base + j] = le[j];
            j += 1;
        }
        block += 1;
    }
    out
}

/// Deterministic 8-byte name hash for an account type name.
#[inline]
pub const fn name_hash(name: &str) -> [u8; 8] {
    fnv1a64(0, name.as_bytes()).to_le_bytes()
}

// ══════════════════════════════════════════════════════════════════════
//  AccountLayoutEntry -- one row per account type
// ══════════════════════════════════════════════════════════════════════

/// A single account-layout row in the registry.
///
/// All multi-byte fields are alignment-1 wire integers, so the whole
/// struct is `Pod` (size 31, alignment 1, no padding) and overlays
/// directly on account bytes.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AccountLayoutEntry {
    /// Account discriminator (byte 0 of a compact account, or the
    /// `disc` of a headered account).
    pub disc: u8,
    /// Layout/schema version for this account type.
    pub version: WireU16,
    /// Minimum account data length required to load this type.
    pub min_size: WireU32,
    /// Fixed body size (excluding any dynamic tail).
    pub fixed_size: WireU32,
    /// First 8 bytes of the layout fingerprint.
    pub layout_id: WireU64,
    /// Entry flags (`ENTRY_FLAG_*`).
    pub flags: WireU32,
    /// Deterministic 8-byte hash of the account type name.
    pub name_hash: [u8; 8],
}

const _: () = assert!(core::mem::size_of::<AccountLayoutEntry>() == 31);
const _: () = assert!(core::mem::align_of::<AccountLayoutEntry>() == 1);

impl AccountLayoutEntry {
    /// Byte size of one entry on the wire.
    pub const SIZE: usize = 31;

    /// Construct an entry from native values.
    #[inline]
    pub fn new(
        disc: u8,
        version: u16,
        min_size: u32,
        fixed_size: u32,
        layout_id: u64,
        flags: u32,
        name_hash: [u8; 8],
    ) -> Self {
        Self {
            disc,
            version: WireU16::new(version),
            min_size: WireU32::new(min_size),
            fixed_size: WireU32::new(fixed_size),
            layout_id: WireU64::new(layout_id),
            flags: WireU32::new(flags),
            name_hash,
        }
    }

    /// View this entry as raw bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8; Self::SIZE] {
        // SAFETY: `AccountLayoutEntry` is `Pod`, size `SIZE`, alignment 1.
        unsafe { &*(self as *const Self as *const [u8; Self::SIZE]) }
    }

    /// Whether the entry carries a given flag.
    #[inline]
    pub fn has_flag(&self, flag: u32) -> bool {
        self.flags.get() & flag != 0
    }
}

// SAFETY: all fields are alignment-1, every bit pattern valid, no padding.
unsafe impl Zeroable for AccountLayoutEntry {}
unsafe impl Pod for AccountLayoutEntry {}
impl FixedLayout for AccountLayoutEntry {
    const SIZE: usize = AccountLayoutEntry::SIZE;
}

// ══════════════════════════════════════════════════════════════════════
//  ProgramManifestHeader -- the registry header
// ══════════════════════════════════════════════════════════════════════

/// The 80-byte header at the start of a binary registry account.
///
/// Followed by `account_count` contiguous [`AccountLayoutEntry`] records.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProgramManifestHeader {
    /// `REGISTRY_MAGIC`.
    pub magic: [u8; 8],
    /// Registry wire-format version.
    pub version: WireU16,
    /// Number of `AccountLayoutEntry` records that follow.
    pub account_count: WireU16,
    /// Header flags (`HEADER_FLAG_*`).
    pub flags: WireU32,
    /// Deterministic hash linking this registry to the off-chain schema.
    pub schema_hash: [u8; 32],
    /// Deterministic hash of the entry table (drift/tamper detection).
    pub registry_hash: [u8; 32],
}

const _: () = assert!(core::mem::size_of::<ProgramManifestHeader>() == 80);
const _: () = assert!(core::mem::align_of::<ProgramManifestHeader>() == 1);

impl ProgramManifestHeader {
    /// Byte size of the header.
    pub const SIZE: usize = 80;

    /// View this header as raw bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8; Self::SIZE] {
        // SAFETY: `ProgramManifestHeader` is `Pod`, size `SIZE`, alignment 1.
        unsafe { &*(self as *const Self as *const [u8; Self::SIZE]) }
    }

    /// Whether the magic matches `REGISTRY_MAGIC`.
    #[inline]
    pub fn magic_ok(&self) -> bool {
        self.magic == REGISTRY_MAGIC
    }

    /// Whether the header carries a given flag.
    #[inline]
    pub fn has_flag(&self, flag: u32) -> bool {
        self.flags.get() & flag != 0
    }
}

// SAFETY: all fields are alignment-1, every bit pattern valid, no padding.
unsafe impl Zeroable for ProgramManifestHeader {}
unsafe impl Pod for ProgramManifestHeader {}
impl FixedLayout for ProgramManifestHeader {
    const SIZE: usize = ProgramManifestHeader::SIZE;
}

/// Required byte length for a registry with `count` entries.
#[inline]
pub const fn registry_len(count: usize) -> usize {
    ProgramManifestHeader::SIZE + count * AccountLayoutEntry::SIZE
}

// ══════════════════════════════════════════════════════════════════════
//  ProgramManifestView -- bounds-checked zero-copy reader
// ══════════════════════════════════════════════════════════════════════

/// A bounds-checked zero-copy reader over registry account bytes.
pub struct ProgramManifestView<'a> {
    header: &'a ProgramManifestHeader,
    entries: &'a [u8],
    count: usize,
}

impl<'a> ProgramManifestView<'a> {
    /// Parse and validate a registry from raw account bytes.
    ///
    /// Checks the magic, that the buffer holds the declared number of
    /// entries, and returns a view with bounds-checked entry access.
    pub fn parse(data: &'a [u8]) -> Result<Self, ProgramError> {
        if data.len() < ProgramManifestHeader::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: length checked; header is Pod, alignment 1.
        let header = unsafe { &*(data.as_ptr() as *const ProgramManifestHeader) };
        if !header.magic_ok() {
            return Err(ProgramError::InvalidAccountData);
        }
        if header.version.get() != REGISTRY_VERSION {
            return Err(ProgramError::InvalidAccountData);
        }
        let count = header.account_count.get() as usize;
        let need = registry_len(count);
        if data.len() < need {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let entries = &data[ProgramManifestHeader::SIZE..need];
        Ok(Self {
            header,
            entries,
            count,
        })
    }

    /// The parsed header.
    #[inline]
    pub fn header(&self) -> &ProgramManifestHeader {
        self.header
    }

    /// Number of account-layout entries.
    #[inline]
    pub fn account_count(&self) -> usize {
        self.count
    }

    /// Borrow the `i`-th entry, or `None` if out of range.
    #[inline]
    pub fn entry(&self, i: usize) -> Option<&'a AccountLayoutEntry> {
        if i >= self.count {
            return None;
        }
        let start = i * AccountLayoutEntry::SIZE;
        let bytes = self.entries.get(start..start + AccountLayoutEntry::SIZE)?;
        // SAFETY: slice is exactly one entry; entry is Pod, alignment 1.
        Some(unsafe { &*(bytes.as_ptr() as *const AccountLayoutEntry) })
    }

    /// Iterate over all entries.
    #[inline]
    pub fn entries(&self) -> impl Iterator<Item = &'a AccountLayoutEntry> + '_ {
        (0..self.count).filter_map(move |i| self.entry(i))
    }

    /// Find the first entry with a given discriminator.
    #[inline]
    pub fn find_by_disc(&self, disc: u8) -> Option<&'a AccountLayoutEntry> {
        self.entries().find(|e| e.disc == disc)
    }

    /// Recompute the registry hash over the entry table.
    #[inline]
    pub fn compute_registry_hash(&self) -> [u8; 32] {
        expand_hash(self.entries)
    }

    /// Whether the stored `registry_hash` matches the entry table.
    #[inline]
    pub fn verify_registry_hash(&self) -> bool {
        self.header.registry_hash == self.compute_registry_hash()
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Writer -- serialize a registry into a buffer
// ══════════════════════════════════════════════════════════════════════

/// Write a complete registry (header + entries) into `buf`.
///
/// Computes `registry_hash` over the serialized entries. Returns the
/// number of bytes written, or `AccountDataTooSmall` if `buf` is too
/// short.
pub fn write_registry(
    buf: &mut [u8],
    version: u16,
    flags: u32,
    schema_hash: &[u8; 32],
    entries: &[AccountLayoutEntry],
) -> Result<usize, ProgramError> {
    if version != REGISTRY_VERSION || entries.len() > u16::MAX as usize {
        return Err(ProgramError::InvalidArgument);
    }
    let total = registry_len(entries.len());
    if buf.len() < total {
        return Err(ProgramError::AccountDataTooSmall);
    }
    // Entries first, so we can hash them before stamping the header.
    let mut off = ProgramManifestHeader::SIZE;
    for e in entries {
        buf[off..off + AccountLayoutEntry::SIZE].copy_from_slice(e.as_bytes());
        off += AccountLayoutEntry::SIZE;
    }
    let registry_hash = expand_hash(&buf[ProgramManifestHeader::SIZE..total]);

    let header = ProgramManifestHeader {
        magic: REGISTRY_MAGIC,
        version: WireU16::new(version),
        account_count: WireU16::new(entries.len() as u16),
        flags: WireU32::new(flags),
        schema_hash: *schema_hash,
        registry_hash,
    };
    buf[..ProgramManifestHeader::SIZE].copy_from_slice(header.as_bytes());
    Ok(total)
}

// ══════════════════════════════════════════════════════════════════════
//  ManifestProfile -- how much registry machinery a program wants
// ══════════════════════════════════════════════════════════════════════

/// How a program opts into the metadata tiers.
///
/// Maps to `#[hopper::program(manifest = "...")]` (macro wiring is the
/// documented next step; the semantics are usable today).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestProfile {
    /// Off-chain artifacts only (JSON / IDL / SDKs). No on-chain registry.
    Offchain,
    /// Publishes and reads the on-chain registry PDA.
    Onchain,
    /// Like `Onchain`, and upgrades/migrations must match the on-chain
    /// `registry_hash` before proceeding.
    Governed,
}

impl ManifestProfile {
    /// Whether this profile publishes/reads the on-chain registry PDA.
    #[inline]
    pub const fn publishes_onchain(self) -> bool {
        matches!(self, Self::Onchain | Self::Governed)
    }

    /// Whether this profile gates upgrades/migrations on the registry.
    #[inline]
    pub const fn gates_upgrades(self) -> bool {
        matches!(self, Self::Governed)
    }

    /// Canonical lowercase name (matches the macro attribute value).
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offchain => "offchain",
            Self::Onchain => "onchain",
            Self::Governed => "governed",
        }
    }

    /// Parse from the macro attribute string.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "offchain" => Some(Self::Offchain),
            "onchain" => Some(Self::Onchain),
            "governed" => Some(Self::Governed),
            _ => None,
        }
    }

    /// The header flags this profile implies.
    #[inline]
    pub const fn header_flags(self) -> u32 {
        match self {
            Self::Governed => HEADER_FLAG_GOVERNED,
            _ => 0,
        }
    }
}

impl ManifestProfile {
    /// Parse from the macro attribute string, failing closed on unknown
    /// values. Unlike [`from_str`](Self::from_str) this distinguishes an
    /// unknown profile (returned in the `Err`) so callers can surface a
    /// precise diagnostic.
    #[inline]
    pub fn try_parse(s: &str) -> Result<Self, ProgramError> {
        Self::from_str(s).ok_or(ProgramError::InvalidArgument)
    }

    /// Whether an upgrade producing `compat` is permitted under this profile.
    ///
    /// - `Offchain` / `Onchain` allow anything except a `Breaking` change.
    /// - `Governed` additionally blocks `MigrationRequired` unless an
    ///   explicit migration has been registered (the caller is responsible
    ///   for re-classifying a registered migration as `Additive`).
    #[inline]
    pub const fn permits_upgrade(self, compat: RegistryCompat) -> bool {
        match self {
            Self::Offchain | Self::Onchain => !matches!(compat, RegistryCompat::Breaking),
            Self::Governed => {
                matches!(compat, RegistryCompat::Unchanged | RegistryCompat::Additive)
            }
        }
    }
}

impl Default for ManifestProfile {
    #[inline]
    fn default() -> Self {
        Self::Offchain
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Registry compatibility -- diff two registry snapshots
// ══════════════════════════════════════════════════════════════════════

/// Compatibility verdict between two registry snapshots (or two entries).
///
/// Variants are ordered by increasing severity, so combining several
/// per-entry verdicts is a `max`. A governed upgrade gate consults
/// [`ManifestProfile::permits_upgrade`] with the combined verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegistryCompat {
    /// Entry tables are byte-identical.
    Unchanged,
    /// Only new account types were added; every existing row is unchanged.
    Additive,
    /// An existing account type changed in a forward-compatible way that
    /// still needs a migration (version bump with the same `layout_id`, or
    /// a grown `fixed_size`).
    MigrationRequired,
    /// An existing account type changed incompatibly: a discriminator now
    /// maps to a different `layout_id`, a body shrank, the compact/headered
    /// flag flipped, or a previously-present type was removed.
    Breaking,
}

impl RegistryCompat {
    /// The more severe of two verdicts.
    #[inline]
    pub fn worst(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }
}

/// The compact/headered shape bits that may never change in place.
const ENTRY_SHAPE_FLAGS: u32 = ENTRY_FLAG_COMPACT | ENTRY_FLAG_HEADERED | ENTRY_FLAG_DYNAMIC_TAIL;

/// Classify the change from `old` to `new` for one account type.
///
/// Both entries are assumed to share a discriminator (the caller matches
/// them up). The verdict captures whether redeploying `new` over `old`
/// is safe, needs a migration, or breaks existing accounts.
pub fn diff_entry(old: &AccountLayoutEntry, new: &AccountLayoutEntry) -> RegistryCompat {
    if old == new {
        return RegistryCompat::Unchanged;
    }
    // A discriminator that now maps to a different wire layout, or whose
    // structural shape flipped, breaks every account already on chain.
    if old.layout_id != new.layout_id
        || (old.flags.get() & ENTRY_SHAPE_FLAGS) != (new.flags.get() & ENTRY_SHAPE_FLAGS)
    {
        return RegistryCompat::Breaking;
    }
    // A body that shrank invalidates existing accounts; a body that grew is
    // a migration (the loader's min_size check rejects stale-short accounts).
    if new.fixed_size.get() < old.fixed_size.get() || new.min_size.get() < old.min_size.get() {
        return RegistryCompat::Breaking;
    }
    if new.fixed_size.get() > old.fixed_size.get() || new.version.get() > old.version.get() {
        return RegistryCompat::MigrationRequired;
    }
    // Same shape and sizes, only non-structural metadata differs (e.g. a
    // deprecated flag or name hash). Treat as a migration to be safe.
    RegistryCompat::MigrationRequired
}

/// Classify the change from registry `old` to registry `new`.
///
/// Walks both entry tables (no allocation): every old discriminator must
/// still be present and compatible, and any brand-new discriminator is
/// additive. The returned verdict is the worst across all rows.
pub fn diff_registries(
    old: &ProgramManifestView<'_>,
    new: &ProgramManifestView<'_>,
) -> RegistryCompat {
    let mut verdict = RegistryCompat::Unchanged;

    for old_entry in old.entries() {
        match new.find_by_disc(old_entry.disc) {
            Some(new_entry) => verdict = verdict.worst(diff_entry(old_entry, new_entry)),
            // A type that used to exist is gone: existing accounts orphaned.
            None => verdict = verdict.worst(RegistryCompat::Breaking),
        }
    }

    for new_entry in new.entries() {
        if old.find_by_disc(new_entry.disc).is_none() {
            verdict = verdict.worst(RegistryCompat::Additive);
        }
    }

    verdict
}

/// Verify an on-chain registry view against expected off-chain hashes.
///
/// Returns `true` only if the stored `registry_hash` is internally
/// consistent (matches the entry table) **and** both the schema hash and
/// registry hash equal the values the off-chain build produced. This is
/// the governed-upgrade gate: a deploy proceeds only when the binary the
/// validator will run matches the artifacts the client/manager generated.
pub fn registry_matches(
    view: &ProgramManifestView<'_>,
    expected_schema_hash: &[u8; 32],
    expected_registry_hash: &[u8; 32],
) -> bool {
    view.verify_registry_hash()
        && &view.header().schema_hash == expected_schema_hash
        && &view.header().registry_hash == expected_registry_hash
}

// ══════════════════════════════════════════════════════════════════════
//  AccountDescriptor / LayoutDescriptor -- the one-source-of-truth view
// ══════════════════════════════════════════════════════════════════════

/// A single, compile-time description of one account layout that every
/// tier reads from: the hot-path loader (length + discriminator), the
/// Tier-2 registry row, off-chain schema/client metadata, and field
/// offsets.
///
/// Both compact (`[disc:u8][body]`) and headered (16-byte `HopperHeader`)
/// layouts produce an `AccountDescriptor` through [`LayoutDescriptor`], so
/// Hopper has *one* registry/identity model while keeping the 1-byte
/// compact hot path. The descriptor is fully `const`: building it costs no
/// CU and reads no on-chain registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountDescriptor {
    /// Account type name (matches the Rust struct identifier).
    pub name: &'static str,
    /// Deterministic 8-byte hash of `name` (Tier-2 row key).
    pub name_hash: [u8; 8],
    /// Discriminator stored at byte 0.
    pub disc: u8,
    /// Layout/schema version.
    pub version: u16,
    /// Fixed body size in bytes (the zero-copy struct, no header/disc).
    pub body_size: u32,
    /// Minimum account data length to load this type (header/disc + body).
    pub min_size: u32,
    /// Byte offset at which the typed body begins (`1` compact, `16`
    /// headered). The discriminator is always at byte 0.
    pub body_offset: u8,
    /// First 8 bytes of the layout fingerprint.
    pub layout_id: [u8; 8],
    /// Entry flags (`ENTRY_FLAG_*`): the structural shape of the layout.
    pub flags: u32,
}

impl AccountDescriptor {
    /// Build a descriptor for a compact `[disc:u8][body]` layout.
    #[inline]
    pub const fn compact(
        name: &'static str,
        disc: u8,
        version: u16,
        body_size: u32,
        layout_id: [u8; 8],
    ) -> Self {
        Self {
            name,
            name_hash: name_hash(name),
            disc,
            version,
            body_size,
            min_size: hopper_runtime::COMPACT_BODY_OFFSET as u32 + body_size,
            body_offset: hopper_runtime::COMPACT_BODY_OFFSET as u8,
            layout_id,
            flags: ENTRY_FLAG_COMPACT,
        }
    }

    /// Build a descriptor for a headered layout (16-byte `HopperHeader`).
    #[inline]
    pub const fn headered(
        name: &'static str,
        disc: u8,
        version: u16,
        body_size: u32,
        layout_id: [u8; 8],
    ) -> Self {
        let header_len = crate::account::HEADER_LEN as u32;
        Self {
            name,
            name_hash: name_hash(name),
            disc,
            version,
            body_size,
            min_size: header_len + body_size,
            body_offset: crate::account::HEADER_LEN as u8,
            layout_id,
            flags: ENTRY_FLAG_HEADERED,
        }
    }

    /// Mark this layout as carrying a variable-length tail. `min_size`
    /// stays the fixed prefix length (the loader rejects anything shorter);
    /// the tail lives beyond it.
    #[inline]
    pub const fn with_dynamic_tail(mut self) -> Self {
        self.flags |= ENTRY_FLAG_DYNAMIC_TAIL;
        self
    }

    /// Mark this layout as deprecated (readable, not writable).
    #[inline]
    pub const fn deprecated(mut self) -> Self {
        self.flags |= ENTRY_FLAG_DEPRECATED;
        self
    }

    /// Whether this descriptor is a compact `[disc][body]` layout.
    #[inline]
    pub const fn is_compact(self) -> bool {
        self.flags & ENTRY_FLAG_COMPACT != 0
    }

    /// Whether this descriptor is a 16-byte-headered layout.
    #[inline]
    pub const fn is_headered(self) -> bool {
        self.flags & ENTRY_FLAG_HEADERED != 0
    }

    /// Whether this descriptor carries a dynamic tail.
    #[inline]
    pub const fn has_dynamic_tail(self) -> bool {
        self.flags & ENTRY_FLAG_DYNAMIC_TAIL != 0
    }

    /// The Tier-2 registry row for this layout. Single source of truth:
    /// the macro-generated `registry_entry()` delegates here.
    #[inline]
    pub fn registry_entry(self) -> AccountLayoutEntry {
        AccountLayoutEntry::new(
            self.disc,
            self.version,
            self.min_size,
            self.body_size,
            u64::from_le_bytes(self.layout_id),
            self.flags,
            self.name_hash,
        )
    }

    /// Hot-path validation: length + discriminator only, **no registry
    /// read**. Inlined and branch-light so a compact loader pays nothing
    /// for the unified model. The discriminator is at byte 0 for both
    /// compact and headered layouts.
    #[inline(always)]
    pub fn validate(self, data: &[u8]) -> Result<(), ProgramError> {
        if data.len() < self.min_size as usize {
            return Err(ProgramError::AccountDataTooSmall);
        }
        if data[0] != self.disc {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }
}

/// One layout, one descriptor. Implemented by every Hopper account type
/// (compact or headered) so a program can enumerate, register, and
/// validate all of its layouts through a single trait.
///
/// The macros emit the impl; the provided methods route the Tier-2 row and
/// the hot-path check through the single [`AccountDescriptor`] constant so
/// the loader, the registry, and the off-chain metadata can never drift.
pub trait LayoutDescriptor {
    /// The compile-time descriptor for this layout.
    const DESCRIPTOR: AccountDescriptor;

    /// The Tier-2 registry row for this layout.
    #[inline]
    fn registry_entry() -> AccountLayoutEntry {
        Self::DESCRIPTOR.registry_entry()
    }

    /// Hot-path length + discriminator check, no registry read.
    #[inline(always)]
    fn validate_hot(data: &[u8]) -> Result<(), ProgramError> {
        Self::DESCRIPTOR.validate(data)
    }
}

/// Classify deploying a set of *generated* descriptors over an *on-chain*
/// registry, for the governed-upgrade gate. No allocation: both sides are
/// walked in place.
///
/// - An on-chain discriminator with no matching descriptor means a type
///   was removed: `Breaking`.
/// - A descriptor with no on-chain row is a new type: `Additive`.
/// - A descriptor that matches an on-chain row is classified by
///   [`diff_entry`] (on-chain row is the `old` side).
///
/// The result is the worst verdict across all rows; feed it to
/// [`ManifestProfile::permits_upgrade`].
pub fn diff_descriptors_vs_registry(
    descriptors: &[AccountDescriptor],
    onchain: &ProgramManifestView<'_>,
) -> RegistryCompat {
    let mut verdict = RegistryCompat::Unchanged;

    for entry in onchain.entries() {
        let mut described = false;
        for d in descriptors {
            if d.disc == entry.disc {
                described = true;
                break;
            }
        }
        if !described {
            verdict = verdict.worst(RegistryCompat::Breaking);
        }
    }

    for d in descriptors {
        match onchain.find_by_disc(d.disc) {
            Some(entry) => verdict = verdict.worst(diff_entry(entry, &d.registry_entry())),
            None => verdict = verdict.worst(RegistryCompat::Additive),
        }
    }

    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    fn sample_entries() -> [AccountLayoutEntry; 2] {
        [
            AccountLayoutEntry::new(
                1,
                1,
                33,
                32,
                0xDEAD_BEEF,
                ENTRY_FLAG_COMPACT,
                name_hash("Vault"),
            ),
            AccountLayoutEntry::new(
                2,
                3,
                64,
                48,
                0xCAFE_BABE,
                ENTRY_FLAG_HEADERED | ENTRY_FLAG_DYNAMIC_TAIL,
                name_hash("Order"),
            ),
        ]
    }

    #[test]
    fn struct_sizes_are_alignment_one_and_fixed() {
        assert_eq!(core::mem::size_of::<ProgramManifestHeader>(), 80);
        assert_eq!(core::mem::align_of::<ProgramManifestHeader>(), 1);
        assert_eq!(core::mem::size_of::<AccountLayoutEntry>(), 31);
        assert_eq!(core::mem::align_of::<AccountLayoutEntry>(), 1);
    }

    #[test]
    fn write_then_parse_roundtrip() {
        let entries = sample_entries();
        let schema_hash = expand_hash(b"schema:v1");
        let mut buf = [0u8; registry_len(2)];
        let n = write_registry(&mut buf, REGISTRY_VERSION, 0, &schema_hash, &entries).unwrap();
        assert_eq!(n, registry_len(2));

        let view = ProgramManifestView::parse(&buf).unwrap();
        assert!(view.header().magic_ok());
        assert_eq!(view.header().version.get(), REGISTRY_VERSION);
        assert_eq!(view.account_count(), 2);
        assert_eq!(view.header().schema_hash, schema_hash);

        let e0 = view.entry(0).unwrap();
        assert_eq!(e0.disc, 1);
        assert_eq!(e0.fixed_size.get(), 32);
        assert_eq!(e0.layout_id.get(), 0xDEAD_BEEF);
        assert!(e0.has_flag(ENTRY_FLAG_COMPACT));
        assert_eq!(e0.name_hash, name_hash("Vault"));

        let e1 = view.find_by_disc(2).unwrap();
        assert_eq!(e1.version.get(), 3);
        assert!(e1.has_flag(ENTRY_FLAG_DYNAMIC_TAIL));

        assert!(view.entry(2).is_none());
        assert!(view.find_by_disc(99).is_none());
    }

    #[test]
    fn registry_hash_detects_tampering() {
        let entries = sample_entries();
        let schema_hash = [0u8; 32];
        let mut buf = [0u8; registry_len(2)];
        write_registry(&mut buf, REGISTRY_VERSION, 0, &schema_hash, &entries).unwrap();

        let view = ProgramManifestView::parse(&buf).unwrap();
        assert!(view.verify_registry_hash());

        // Flip a byte inside the entry table.
        let mut tampered = buf;
        tampered[ProgramManifestHeader::SIZE + 4] ^= 0xFF;
        let tview = ProgramManifestView::parse(&tampered).unwrap();
        assert!(!tview.verify_registry_hash());
    }

    #[test]
    fn parse_rejects_bad_magic_version_and_short_buffers() {
        let mut buf = [0u8; registry_len(1)];
        let entries = [AccountLayoutEntry::new(1, 1, 1, 0, 0, 0, [0; 8])];
        write_registry(&mut buf, 1, 0, &[0; 32], &entries).unwrap();

        // Truncated below header.
        assert!(matches!(
            ProgramManifestView::parse(&buf[..10]),
            Err(ProgramError::AccountDataTooSmall)
        ));

        // Bad magic.
        let mut bad = buf;
        bad[0] ^= 0xFF;
        assert!(matches!(
            ProgramManifestView::parse(&bad),
            Err(ProgramError::InvalidAccountData)
        ));

        // Unsupported wire-format version.
        let mut bad_version = buf;
        bad_version[8..10].copy_from_slice(&(REGISTRY_VERSION + 1).to_le_bytes());
        assert!(matches!(
            ProgramManifestView::parse(&bad_version),
            Err(ProgramError::InvalidAccountData)
        ));

        // Declares one entry but the entry bytes are missing.
        assert!(matches!(
            ProgramManifestView::parse(&buf[..ProgramManifestHeader::SIZE + 4]),
            Err(ProgramError::AccountDataTooSmall)
        ));
    }

    #[test]
    fn write_registry_rejects_short_buffer() {
        let entries = sample_entries();
        let mut buf = [0u8; 8];
        assert!(matches!(
            write_registry(&mut buf, 1, 0, &[0; 32], &entries),
            Err(ProgramError::AccountDataTooSmall)
        ));
    }

    #[test]
    fn write_registry_rejects_bad_version_and_entry_count_overflow() {
        let entries = sample_entries();
        let mut buf = [0u8; registry_len(2)];
        assert!(matches!(
            write_registry(&mut buf, REGISTRY_VERSION + 1, 0, &[0; 32], &entries),
            Err(ProgramError::InvalidArgument)
        ));

        let too_many = vec![AccountLayoutEntry::default(); u16::MAX as usize + 1];
        assert!(matches!(
            write_registry(&mut buf, REGISTRY_VERSION, 0, &[0; 32], &too_many),
            Err(ProgramError::InvalidArgument)
        ));
    }

    #[test]
    fn profiles_behave() {
        assert!(!ManifestProfile::Offchain.publishes_onchain());
        assert!(ManifestProfile::Onchain.publishes_onchain());
        assert!(ManifestProfile::Governed.publishes_onchain());
        assert!(!ManifestProfile::Onchain.gates_upgrades());
        assert!(ManifestProfile::Governed.gates_upgrades());
        assert_eq!(
            ManifestProfile::Governed.header_flags(),
            HEADER_FLAG_GOVERNED
        );

        for p in [
            ManifestProfile::Offchain,
            ManifestProfile::Onchain,
            ManifestProfile::Governed,
        ] {
            assert_eq!(ManifestProfile::from_str(p.as_str()), Some(p));
        }
        assert_eq!(ManifestProfile::from_str("nope"), None);
        assert_eq!(ManifestProfile::default(), ManifestProfile::Offchain);
    }

    #[test]
    fn name_hash_is_deterministic_and_distinct() {
        assert_eq!(name_hash("Vault"), name_hash("Vault"));
        assert_ne!(name_hash("Vault"), name_hash("Order"));
    }

    #[test]
    fn profile_try_parse_fails_closed() {
        assert_eq!(
            ManifestProfile::try_parse("governed"),
            Ok(ManifestProfile::Governed)
        );
        assert!(matches!(
            ManifestProfile::try_parse("bogus"),
            Err(ProgramError::InvalidArgument)
        ));
    }

    /// Build a one-entry registry view backed by `buf` for diff testing.
    fn write_one(buf: &mut [u8], entry: AccountLayoutEntry) -> usize {
        write_registry(buf, REGISTRY_VERSION, 0, &[0; 32], &[entry]).unwrap()
    }

    #[test]
    fn diff_entry_classifies_each_kind() {
        let base = AccountLayoutEntry::new(1, 1, 41, 40, 0xABCD, ENTRY_FLAG_COMPACT, [1; 8]);

        // Identical -> Unchanged.
        assert_eq!(diff_entry(&base, &base), RegistryCompat::Unchanged);

        // Grown body, same layout_id -> MigrationRequired.
        let grown = AccountLayoutEntry::new(1, 1, 49, 48, 0xABCD, ENTRY_FLAG_COMPACT, [1; 8]);
        assert_eq!(diff_entry(&base, &grown), RegistryCompat::MigrationRequired);

        // Version bump only -> MigrationRequired.
        let bumped = AccountLayoutEntry::new(1, 2, 41, 40, 0xABCD, ENTRY_FLAG_COMPACT, [1; 8]);
        assert_eq!(
            diff_entry(&base, &bumped),
            RegistryCompat::MigrationRequired
        );

        // Different layout_id -> Breaking.
        let relaid = AccountLayoutEntry::new(1, 1, 41, 40, 0x9999, ENTRY_FLAG_COMPACT, [1; 8]);
        assert_eq!(diff_entry(&base, &relaid), RegistryCompat::Breaking);

        // Shrunk body -> Breaking.
        let shrunk = AccountLayoutEntry::new(1, 1, 33, 32, 0xABCD, ENTRY_FLAG_COMPACT, [1; 8]);
        assert_eq!(diff_entry(&base, &shrunk), RegistryCompat::Breaking);

        // Compact -> headered shape flip -> Breaking.
        let reshaped = AccountLayoutEntry::new(1, 1, 41, 40, 0xABCD, ENTRY_FLAG_HEADERED, [1; 8]);
        assert_eq!(diff_entry(&base, &reshaped), RegistryCompat::Breaking);
    }

    #[test]
    fn diff_registries_combines_rows() {
        let v = AccountLayoutEntry::new(1, 1, 41, 40, 0xABCD, ENTRY_FLAG_COMPACT, name_hash("V"));

        let mut old_buf = [0u8; registry_len(1)];
        let n0 = write_one(&mut old_buf, v);
        let old = ProgramManifestView::parse(&old_buf[..n0]).unwrap();

        // Same registry -> Unchanged.
        assert_eq!(diff_registries(&old, &old), RegistryCompat::Unchanged);

        // Add a brand-new account type -> Additive.
        let w = AccountLayoutEntry::new(2, 1, 25, 24, 0x1234, ENTRY_FLAG_COMPACT, name_hash("W"));
        let mut add_buf = [0u8; registry_len(2)];
        write_registry(&mut add_buf, REGISTRY_VERSION, 0, &[0; 32], &[v, w]).unwrap();
        let added = ProgramManifestView::parse(&add_buf).unwrap();
        assert_eq!(diff_registries(&old, &added), RegistryCompat::Additive);

        // Dropping an existing type -> Breaking.
        assert_eq!(diff_registries(&added, &old), RegistryCompat::Breaking);
    }

    #[test]
    fn permits_upgrade_gates_by_profile() {
        // Offchain/Onchain allow migrations; only Breaking is blocked.
        assert!(ManifestProfile::Onchain.permits_upgrade(RegistryCompat::MigrationRequired));
        assert!(!ManifestProfile::Onchain.permits_upgrade(RegistryCompat::Breaking));
        // Governed blocks anything beyond Additive.
        assert!(ManifestProfile::Governed.permits_upgrade(RegistryCompat::Additive));
        assert!(!ManifestProfile::Governed.permits_upgrade(RegistryCompat::MigrationRequired));
    }

    #[test]
    fn registry_matches_checks_hashes() {
        let entries = sample_entries();
        let schema_hash = expand_hash(b"schema:v7");
        let mut buf = [0u8; registry_len(2)];
        write_registry(&mut buf, REGISTRY_VERSION, 0, &schema_hash, &entries).unwrap();
        let view = ProgramManifestView::parse(&buf).unwrap();

        let registry_hash = view.compute_registry_hash();
        assert!(registry_matches(&view, &schema_hash, &registry_hash));
        // Wrong expected schema hash -> rejected.
        assert!(!registry_matches(&view, &[9; 32], &registry_hash));
    }

    #[test]
    fn descriptor_compact_and_headered_shapes() {
        let c = AccountDescriptor::compact("Vault", 1, 1, 40, [1; 8]);
        assert!(c.is_compact());
        assert!(!c.is_headered());
        // Compact folds in the single discriminator byte.
        assert_eq!(c.body_offset, 1);
        assert_eq!(c.min_size, 41);
        assert_eq!(c.body_size, 40);

        let h = AccountDescriptor::headered("Config", 2, 1, 40, [2; 8]);
        assert!(h.is_headered());
        assert!(!h.is_compact());
        // Headered folds in the 16-byte universal header.
        assert_eq!(h.body_offset, 16);
        assert_eq!(h.min_size, 56);

        let t = h.with_dynamic_tail();
        assert!(t.has_dynamic_tail());
        // min_size stays the fixed prefix; the tail lives beyond it.
        assert_eq!(t.min_size, 56);
    }

    #[test]
    fn descriptor_is_single_source_for_registry_entry() {
        let d = AccountDescriptor::compact("Vault", 1, 2, 40, [7; 8]);
        let e = d.registry_entry();
        assert_eq!(e.disc, 1);
        assert_eq!(e.version.get(), 2);
        assert_eq!(e.min_size.get(), 41);
        assert_eq!(e.fixed_size.get(), 40);
        assert_eq!(e.layout_id.get(), u64::from_le_bytes([7; 8]));
        assert!(e.has_flag(ENTRY_FLAG_COMPACT));
        assert_eq!(e.name_hash, name_hash("Vault"));
    }

    #[test]
    fn descriptor_validate_is_len_and_disc_only() {
        let d = AccountDescriptor::compact("Vault", 7, 1, 40, [1; 8]);
        let mut buf = [0u8; 41];
        buf[0] = 7;
        assert!(d.validate(&buf).is_ok());
        buf[0] = 8;
        assert!(matches!(
            d.validate(&buf),
            Err(ProgramError::InvalidAccountData)
        ));
        buf[0] = 7;
        assert!(matches!(
            d.validate(&buf[..40]),
            Err(ProgramError::AccountDataTooSmall)
        ));
    }

    #[test]
    fn diff_descriptors_vs_registry_classifies() {
        let v = AccountDescriptor::compact("V", 1, 1, 40, [0xAB; 8]);
        let entry = v.registry_entry();
        let mut buf = [0u8; registry_len(1)];
        let n = write_one(&mut buf, entry);
        let onchain = ProgramManifestView::parse(&buf[..n]).unwrap();

        // Identical descriptor set -> Unchanged.
        assert_eq!(
            diff_descriptors_vs_registry(&[v], &onchain),
            RegistryCompat::Unchanged
        );

        // Add a new descriptor not yet on chain -> Additive.
        let w = AccountDescriptor::compact("W", 2, 1, 24, [0x12; 8]);
        assert_eq!(
            diff_descriptors_vs_registry(&[v, w], &onchain),
            RegistryCompat::Additive
        );

        // Drop the on-chain type from the descriptor set -> Breaking.
        assert_eq!(
            diff_descriptors_vs_registry(&[w], &onchain),
            RegistryCompat::Breaking
        );

        // Grow the body of an existing type -> MigrationRequired.
        let v_grown = AccountDescriptor::compact("V", 1, 2, 48, [0xAB; 8]);
        assert_eq!(
            diff_descriptors_vs_registry(&[v_grown], &onchain),
            RegistryCompat::MigrationRequired
        );
    }

    #[test]
    fn descriptor_governed_gate_end_to_end() {
        // Generated descriptors for the redeploy.
        let descriptors = [
            AccountDescriptor::compact("V", 1, 1, 40, [0xAB; 8]),
            AccountDescriptor::compact("W", 2, 1, 24, [0x12; 8]),
        ];
        // On-chain registry only knows V.
        let mut buf = [0u8; registry_len(1)];
        let n = write_one(&mut buf, descriptors[0].registry_entry());
        let onchain = ProgramManifestView::parse(&buf[..n]).unwrap();

        let compat = diff_descriptors_vs_registry(&descriptors, &onchain);
        assert_eq!(compat, RegistryCompat::Additive);
        // Additive is allowed even under the strict governed profile.
        assert!(ManifestProfile::Governed.permits_upgrade(compat));
    }
}
