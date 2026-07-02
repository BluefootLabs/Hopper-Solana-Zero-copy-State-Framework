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
// SIZE defaults to size_of::<Self>() == 31, proven by the trait (I15);
// the wire size stays pinned by the const asserts above.
impl FixedLayout for AccountLayoutEntry {}

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
// SIZE defaults to size_of::<Self>() == 80, proven by the trait (I15);
// the wire size stays pinned by the const asserts above.
impl FixedLayout for ProgramManifestHeader {}

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

    /// The deterministic [`LayoutFingerprint`] a generated client checks
    /// before zero-copy-decoding a fetched account. `const`: a client can
    /// embed it as a compile-time constant and compare without a registry
    /// read. Folds in only wire-identity fields (name, disc, version,
    /// sizes, body offset, shape flags, layout_id), never the `deprecated`
    /// lifecycle bit -- so deprecating a layout never changes how it decodes.
    pub const fn fingerprint(self) -> LayoutFingerprint {
        // Canonical 32-byte wire-identity encoding (shape flags only).
        let mut buf = [0u8; 32];
        let mut i = 0;
        while i < 8 {
            buf[i] = self.name_hash[i];
            i += 1;
        }
        buf[8] = self.disc;
        let v = self.version.to_le_bytes();
        buf[9] = v[0];
        buf[10] = v[1];
        let bs = self.body_size.to_le_bytes();
        buf[11] = bs[0];
        buf[12] = bs[1];
        buf[13] = bs[2];
        buf[14] = bs[3];
        let ms = self.min_size.to_le_bytes();
        buf[15] = ms[0];
        buf[16] = ms[1];
        buf[17] = ms[2];
        buf[18] = ms[3];
        buf[19] = self.body_offset;
        let sf = (self.flags & ENTRY_SHAPE_FLAGS).to_le_bytes();
        buf[20] = sf[0];
        buf[21] = sf[1];
        buf[22] = sf[2];
        buf[23] = sf[3];
        let mut j = 0;
        while j < 8 {
            buf[24 + j] = self.layout_id[j];
            j += 1;
        }
        let digest = expand_hash(&buf);
        let mut fp = [0u8; 16];
        let mut k = 0;
        while k < 16 {
            fp[k] = digest[k];
            k += 1;
        }
        LayoutFingerprint(fp)
    }

    /// Project this descriptor into a [`DescriptorIdlNode`] for IDL / Codama
    /// generators -- sourced from the same descriptor the loader enforces.
    #[inline]
    pub const fn idl_node(self) -> DescriptorIdlNode {
        DescriptorIdlNode {
            name: self.name,
            disc: self.disc,
            version: self.version,
            body_offset: self.body_offset,
            body_size: self.body_size,
            min_size: self.min_size,
            kind: if self.is_compact() {
                LayoutKind::Compact
            } else {
                LayoutKind::Headered
            },
            has_dynamic_tail: self.has_dynamic_tail(),
            deprecated: self.flags & ENTRY_FLAG_DEPRECATED != 0,
            layout_id: self.layout_id,
            fingerprint: self.fingerprint(),
        }
    }

    /// The direct-mapping account-data cost profile for this layout: how
    /// many bytes a first write copies (copy-on-write), the size class, and
    /// whether it can grow (the expensive realloc path).
    #[inline]
    pub const fn cost_profile(self) -> CostProfile {
        CostProfile {
            cow_copy_bytes: self.min_size,
            class: SizeClass::of(self.min_size),
            growable: self.has_dynamic_tail(),
        }
    }

    /// A descriptor-level advisory lint under the account-data cost model.
    /// Large fixed layouts make first-write CoW copies expensive; growable
    /// large layouts also pay the realloc growth cost.
    #[inline]
    pub const fn cost_lint(self) -> CostLint {
        let class = SizeClass::of(self.min_size);
        if self.has_dynamic_tail() {
            match class {
                SizeClass::Large | SizeClass::VeryLarge => return CostLint::ExpensiveGrowth,
                _ => {}
            }
        }
        match class {
            SizeClass::Large | SizeClass::VeryLarge => CostLint::LargeFixedCopy,
            _ => CostLint::Ok,
        }
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

    /// The client decode fingerprint a generated SDK checks before casting
    /// fetched bytes. Derived from the same descriptor the loader uses.
    #[inline]
    fn fingerprint() -> LayoutFingerprint {
        Self::DESCRIPTOR.fingerprint()
    }

    /// The IDL / Codama projection for this layout, for off-chain codegen.
    #[inline]
    fn idl_node() -> DescriptorIdlNode {
        Self::DESCRIPTOR.idl_node()
    }

    /// The direct-mapping account-data cost profile for this layout.
    #[inline]
    fn cost_profile() -> CostProfile {
        Self::DESCRIPTOR.cost_profile()
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

// ══════════════════════════════════════════════════════════════════════
//  Client decode fingerprint
// ══════════════════════════════════════════════════════════════════════

/// A 16-byte deterministic identity for one wire layout. A generated client
/// embeds the fingerprint of the descriptor it was built against as a
/// constant, then compares it to the fingerprint computed from the layout
/// the program actually advertises (its on-chain registry row) **before**
/// casting fetched bytes. A mismatch means the program was redeployed with a
/// different layout at that discriminator: the client must fail closed
/// rather than zero-copy-decode stale or mis-shaped bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LayoutFingerprint(pub [u8; 16]);

impl LayoutFingerprint {
    /// The raw 16 bytes.
    #[inline]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Lowercase-hex encoding (32 ASCII bytes), for embedding the
    /// fingerprint in generated client source as a string literal.
    pub const fn to_hex(self) -> [u8; 32] {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = [0u8; 32];
        let mut i = 0;
        while i < 16 {
            out[i * 2] = HEX[(self.0[i] >> 4) as usize];
            out[i * 2 + 1] = HEX[(self.0[i] & 0x0f) as usize];
            i += 1;
        }
        out
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Loaded-accounts data-size budgeting (setLoadedAccountsDataSizeLimit)
// ══════════════════════════════════════════════════════════════════════

/// Sum the minimum loaded byte size of a set of account layouts.
///
/// The descriptor-sourced floor for a transaction's
/// `setLoadedAccountsDataSizeLimit`: every listed account contributes at
/// least its `min_size` (header/disc + fixed body). Saturating.
pub fn min_loaded_data_size(descriptors: &[AccountDescriptor]) -> u64 {
    let mut total: u64 = 0;
    let mut i = 0;
    while i < descriptors.len() {
        total = total.saturating_add(descriptors[i].min_size as u64);
        i += 1;
    }
    total
}

/// Recommend a `setLoadedAccountsDataSizeLimit` value from descriptors.
///
/// Starts from [`min_loaded_data_size`], adds `tail_headroom` bytes for each
/// dynamic-tail layout (whose on-wire length exceeds the fixed prefix the
/// descriptor records), and adds a flat `extra` margin for loaders,
/// programs, and sysvars the descriptor set does not model. Saturating; the
/// caller clamps to the runtime maximum.
pub fn recommend_loaded_data_limit(
    descriptors: &[AccountDescriptor],
    tail_headroom: u32,
    extra: u32,
) -> u64 {
    let mut total = min_loaded_data_size(descriptors);
    let mut i = 0;
    while i < descriptors.len() {
        if descriptors[i].has_dynamic_tail() {
            total = total.saturating_add(tail_headroom as u64);
        }
        i += 1;
    }
    total.saturating_add(extra as u64)
}

// ══════════════════════════════════════════════════════════════════════
//  Dynamic-tail-aware layout change classification
// ══════════════════════════════════════════════════════════════════════

/// A precise classification of how one layout row changed between two
/// registry generations -- finer-grained than [`RegistryCompat`], and
/// aware that a dynamic-tail layout's capacity is an off-chain concern.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutChange {
    /// Byte-identical.
    Unchanged,
    /// Only the name hash changed (rename); wire layout identical.
    NameOnly,
    /// A dynamic-tail layout kept its fixed prefix and identity but bumped
    /// version: a tail capacity/policy change that lives off-chain and does
    /// not invalidate existing accounts (the loader only checks the prefix).
    TailCapacity,
    /// Version bumped on a fixed-size layout, prefix unchanged.
    VersionBump,
    /// The fixed prefix grew: the loader's `min_size` check rejects
    /// stale-short accounts until they are migrated / realloced.
    FixedPrefixGrew,
    /// The fixed prefix shrank: existing accounts are now invalid.
    FixedPrefixShrank,
    /// The structural shape flipped (compact <-> headered, or fixed <->
    /// dynamic-tail).
    ShapeFlipped,
    /// The layout fingerprint changed: a different wire type at this disc.
    IdentityChanged,
}

impl LayoutChange {
    /// Map the precise change to the coarse upgrade verdict. A dynamic-tail
    /// capacity bump is `Additive`, unlike a fixed-prefix grow.
    #[inline]
    pub const fn compat(self) -> RegistryCompat {
        match self {
            LayoutChange::Unchanged => RegistryCompat::Unchanged,
            LayoutChange::NameOnly | LayoutChange::TailCapacity => RegistryCompat::Additive,
            LayoutChange::VersionBump | LayoutChange::FixedPrefixGrew => {
                RegistryCompat::MigrationRequired
            }
            LayoutChange::FixedPrefixShrank
            | LayoutChange::ShapeFlipped
            | LayoutChange::IdentityChanged => RegistryCompat::Breaking,
        }
    }
}

/// Classify the change between two registry rows sharing a discriminator,
/// distinguishing a dynamic-tail capacity change from a fixed-prefix change.
pub fn classify_entry_change(old: &AccountLayoutEntry, new: &AccountLayoutEntry) -> LayoutChange {
    if old == new {
        return LayoutChange::Unchanged;
    }
    if old.layout_id != new.layout_id {
        return LayoutChange::IdentityChanged;
    }
    if (old.flags.get() & ENTRY_SHAPE_FLAGS) != (new.flags.get() & ENTRY_SHAPE_FLAGS) {
        return LayoutChange::ShapeFlipped;
    }
    if new.fixed_size.get() < old.fixed_size.get() || new.min_size.get() < old.min_size.get() {
        return LayoutChange::FixedPrefixShrank;
    }
    if new.fixed_size.get() > old.fixed_size.get() || new.min_size.get() > old.min_size.get() {
        return LayoutChange::FixedPrefixGrew;
    }
    // Same identity, shape, and sizes: only version/name metadata differs.
    if new.version.get() != old.version.get() {
        // A dynamic-tail layout with an unchanged fixed prefix only moved
        // its (off-chain) tail capacity/policy: additive, not a migration.
        if new.flags.get() & ENTRY_FLAG_DYNAMIC_TAIL != 0 {
            return LayoutChange::TailCapacity;
        }
        return LayoutChange::VersionBump;
    }
    LayoutChange::NameOnly
}

/// Tail-aware variant of [`diff_descriptors_vs_registry`]: classifies each
/// matched row with [`classify_entry_change`], so a dynamic-tail capacity
/// bump reads as `Additive` rather than `MigrationRequired`. Removal and
/// addition rules are identical; returns the worst verdict.
pub fn diff_descriptors_vs_registry_detailed(
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
            Some(entry) => {
                verdict = verdict.worst(classify_entry_change(entry, &d.registry_entry()).compat())
            }
            None => verdict = verdict.worst(RegistryCompat::Additive),
        }
    }

    verdict
}

// ══════════════════════════════════════════════════════════════════════
//  CoW / account-data-cost layout linting
// ══════════════════════════════════════════════════════════════════════

/// Inclusive upper byte bound for the `Small` size class.
pub const SIZE_CLASS_SMALL_MAX: u32 = 256;
/// Inclusive upper byte bound for the `Medium` size class.
pub const SIZE_CLASS_MEDIUM_MAX: u32 = 4 * 1024;
/// Inclusive upper byte bound for the `Large` size class (one CPI realloc
/// step under the current 10 KiB-per-instruction growth cap).
pub const SIZE_CLASS_LARGE_MAX: u32 = 10 * 1024;

/// Coarse size class for an account layout, aligned with the direct-mapping
/// cost gradient: reads are ~free, the first write copies the whole account
/// (copy-on-write), and growth (realloc) is the most expensive operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeClass {
    /// `<= SIZE_CLASS_SMALL_MAX`: copy-on-first-write is cheap.
    Small,
    /// `<= SIZE_CLASS_MEDIUM_MAX`.
    Medium,
    /// `<= SIZE_CLASS_LARGE_MAX`.
    Large,
    /// Above the large bound: every first-write copy is costly; keep off
    /// hot write paths.
    VeryLarge,
}

impl SizeClass {
    /// Classify a fixed byte size.
    #[inline]
    pub const fn of(bytes: u32) -> Self {
        if bytes <= SIZE_CLASS_SMALL_MAX {
            SizeClass::Small
        } else if bytes <= SIZE_CLASS_MEDIUM_MAX {
            SizeClass::Medium
        } else if bytes <= SIZE_CLASS_LARGE_MAX {
            SizeClass::Large
        } else {
            SizeClass::VeryLarge
        }
    }
}

/// A descriptor-level cost profile under the direct-mapping account model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CostProfile {
    /// Bytes copied on the first write to a fully-populated account
    /// (copy-on-write). For fixed layouts this equals `min_size`.
    pub cow_copy_bytes: u32,
    /// Size class of the fixed prefix.
    pub class: SizeClass,
    /// Whether the layout can grow (dynamic tail), which triggers the
    /// expensive realloc path rather than a fixed-size CoW copy.
    pub growable: bool,
}

/// An advisory lint verdict for a layout under the account-data cost model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CostLint {
    /// No concern: small/medium fixed layout.
    Ok,
    /// Large fixed layout: first-write CoW copies are non-trivial. Keep it
    /// off the hottest write paths, or split hot fields into a small account.
    LargeFixedCopy,
    /// Growable and already large: realloc growth is expensive and may
    /// exceed per-instruction growth limits. Consider a paged design.
    ExpensiveGrowth,
}

// ══════════════════════════════════════════════════════════════════════
//  IDL / Codama export hook
// ══════════════════════════════════════════════════════════════════════

/// The structural kind of a layout, for IDL / codegen consumers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutKind {
    /// Tier-1 compact `[disc:u8][body]`.
    Compact,
    /// 16-byte-headered.
    Headered,
}

/// A minimal, stable, `no_std` projection of an [`AccountDescriptor`] for
/// IDL / Codama-style generators. It carries exactly what a generator needs
/// to emit an account node and a fail-closed decode guard, sourced from the
/// same descriptor the on-chain loader enforces -- so the IDL can never
/// describe a different layout than the program runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DescriptorIdlNode {
    /// Account type name.
    pub name: &'static str,
    /// Discriminator byte.
    pub disc: u8,
    /// Layout version.
    pub version: u16,
    /// Byte offset of the typed body (after disc / header).
    pub body_offset: u8,
    /// Fixed body size.
    pub body_size: u32,
    /// Minimum total account size.
    pub min_size: u32,
    /// Structural kind.
    pub kind: LayoutKind,
    /// Whether a variable-length tail follows the fixed body.
    pub has_dynamic_tail: bool,
    /// Whether the layout is deprecated.
    pub deprecated: bool,
    /// 8-byte layout id.
    pub layout_id: [u8; 8],
    /// 16-byte client decode fingerprint.
    pub fingerprint: LayoutFingerprint,
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

    #[test]
    fn fingerprint_is_deterministic_and_identity_sensitive() {
        let d = AccountDescriptor::compact("Vault", 1, 1, 40, [0xAB; 8]);
        // Deterministic: same descriptor -> same fingerprint.
        assert_eq!(d.fingerprint(), d.fingerprint());
        // Independent rebuild of the same identity matches.
        let same = AccountDescriptor::compact("Vault", 1, 1, 40, [0xAB; 8]);
        assert_eq!(d.fingerprint(), same.fingerprint());

        // Each wire-identity field shifts the fingerprint.
        let diff_id = AccountDescriptor::compact("Vault", 1, 1, 40, [0xCD; 8]);
        assert_ne!(d.fingerprint(), diff_id.fingerprint());
        let diff_disc = AccountDescriptor::compact("Vault", 2, 1, 40, [0xAB; 8]);
        assert_ne!(d.fingerprint(), diff_disc.fingerprint());
        let diff_ver = AccountDescriptor::compact("Vault", 1, 2, 40, [0xAB; 8]);
        assert_ne!(d.fingerprint(), diff_ver.fingerprint());
        let diff_size = AccountDescriptor::compact("Vault", 1, 1, 48, [0xAB; 8]);
        assert_ne!(d.fingerprint(), diff_size.fingerprint());
        // Same identity but headered shape -> different fingerprint.
        let headered = AccountDescriptor::headered("Vault", 1, 1, 40, [0xAB; 8]);
        assert_ne!(d.fingerprint(), headered.fingerprint());
    }

    #[test]
    fn fingerprint_ignores_deprecated_bit() {
        // Marking a layout deprecated must not change how a client decodes it.
        let d = AccountDescriptor::compact("Vault", 1, 1, 40, [0xAB; 8]);
        assert_eq!(d.fingerprint(), d.deprecated().fingerprint());
    }

    #[test]
    fn fingerprint_hex_roundtrips_bytes() {
        let d = AccountDescriptor::headered("Config", 3, 1, 40, [0x10; 8]);
        let fp = d.fingerprint();
        let hex = fp.to_hex();
        assert_eq!(hex.len(), 32);
        // Recompose bytes from the hex and compare.
        let decode = |c: u8| -> u8 {
            match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                _ => unreachable!(),
            }
        };
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = (decode(hex[i * 2]) << 4) | decode(hex[i * 2 + 1]);
        }
        assert_eq!(bytes, fp.as_bytes());
    }

    #[test]
    fn loaded_data_size_sums_min_size() {
        let v = AccountDescriptor::compact("V", 1, 1, 40, [1; 8]); // min_size 41
        let c = AccountDescriptor::headered("C", 2, 1, 40, [2; 8]); // min_size 56
        assert_eq!(min_loaded_data_size(&[v, c]), 41 + 56);
        assert_eq!(min_loaded_data_size(&[]), 0);
    }

    #[test]
    fn recommend_limit_adds_tail_headroom_and_margin() {
        let fixed = AccountDescriptor::compact("V", 1, 1, 40, [1; 8]); // 41
        let tail = AccountDescriptor::headered("Log", 2, 1, 40, [2; 8]).with_dynamic_tail(); // 56
                                                                                             // 41 + 56 + (1 tail * 1024 headroom) + 128 margin.
        assert_eq!(
            recommend_loaded_data_limit(&[fixed, tail], 1024, 128),
            41 + 56 + 1024 + 128
        );
        // No dynamic tails -> headroom not applied.
        assert_eq!(recommend_loaded_data_limit(&[fixed], 1024, 0), 41);
    }

    #[test]
    fn classify_entry_change_is_tail_aware() {
        let fixed = AccountLayoutEntry::new(1, 1, 41, 40, 0xAB, ENTRY_FLAG_COMPACT, [1; 8]);
        let fixed_v2 = AccountLayoutEntry::new(1, 2, 41, 40, 0xAB, ENTRY_FLAG_COMPACT, [1; 8]);
        // Version bump on a fixed layout -> migration.
        assert_eq!(
            classify_entry_change(&fixed, &fixed_v2),
            LayoutChange::VersionBump
        );
        assert_eq!(
            classify_entry_change(&fixed, &fixed_v2).compat(),
            RegistryCompat::MigrationRequired
        );

        let tail_flags = ENTRY_FLAG_HEADERED | ENTRY_FLAG_DYNAMIC_TAIL;
        let tail = AccountLayoutEntry::new(2, 1, 56, 40, 0xCD, tail_flags, [2; 8]);
        let tail_v2 = AccountLayoutEntry::new(2, 2, 56, 40, 0xCD, tail_flags, [2; 8]);
        // Version bump on a dynamic-tail layout with unchanged prefix is a
        // tail capacity/policy change -> additive, not a migration.
        assert_eq!(
            classify_entry_change(&tail, &tail_v2),
            LayoutChange::TailCapacity
        );
        assert_eq!(
            classify_entry_change(&tail, &tail_v2).compat(),
            RegistryCompat::Additive
        );

        // Grown prefix even on a tail layout -> migration.
        let tail_grown = AccountLayoutEntry::new(2, 2, 64, 48, 0xCD, tail_flags, [2; 8]);
        assert_eq!(
            classify_entry_change(&tail, &tail_grown),
            LayoutChange::FixedPrefixGrew
        );

        // Toggling the tail flag is a shape flip -> breaking.
        let detailed = AccountLayoutEntry::new(2, 1, 56, 40, 0xCD, ENTRY_FLAG_HEADERED, [2; 8]);
        assert_eq!(
            classify_entry_change(&detailed, &tail),
            LayoutChange::ShapeFlipped
        );
        // Identity change dominates.
        let reid = AccountLayoutEntry::new(1, 1, 41, 40, 0x99, ENTRY_FLAG_COMPACT, [1; 8]);
        assert_eq!(
            classify_entry_change(&fixed, &reid),
            LayoutChange::IdentityChanged
        );
    }

    #[test]
    fn detailed_diff_treats_tail_capacity_as_additive() {
        let tail = AccountDescriptor::headered("Log", 1, 1, 40, [0xCD; 8]).with_dynamic_tail();
        let mut buf = [0u8; registry_len(1)];
        let n = write_one(&mut buf, tail.registry_entry());
        let onchain = ProgramManifestView::parse(&buf[..n]).unwrap();

        // Same prefix/identity, bumped version (tail capacity bump).
        let tail_v2 = AccountDescriptor::headered("Log", 1, 2, 40, [0xCD; 8]).with_dynamic_tail();
        // The coarse diff calls this a migration...
        assert_eq!(
            diff_descriptors_vs_registry(&[tail_v2], &onchain),
            RegistryCompat::MigrationRequired
        );
        // ...the tail-aware diff recognises it as additive.
        assert_eq!(
            diff_descriptors_vs_registry_detailed(&[tail_v2], &onchain),
            RegistryCompat::Additive
        );
        assert!(ManifestProfile::Governed
            .permits_upgrade(diff_descriptors_vs_registry_detailed(&[tail_v2], &onchain)));
    }

    #[test]
    fn cost_profile_and_lint_track_size_and_growth() {
        // Small fixed layout: cheap CoW, Ok.
        let small = AccountDescriptor::compact("V", 1, 1, 40, [1; 8]);
        let p = small.cost_profile();
        assert_eq!(p.cow_copy_bytes, 41);
        assert_eq!(p.class, SizeClass::Small);
        assert!(!p.growable);
        assert_eq!(small.cost_lint(), CostLint::Ok);

        // Large fixed layout: non-trivial first-write copy.
        let large = AccountDescriptor::headered("Book", 2, 1, 9000, [2; 8]);
        assert_eq!(large.cost_profile().class, SizeClass::Large);
        assert_eq!(large.cost_lint(), CostLint::LargeFixedCopy);

        // Large + growable: expensive realloc growth.
        let growable = AccountDescriptor::headered("Log", 3, 1, 9000, [3; 8]).with_dynamic_tail();
        assert!(growable.cost_profile().growable);
        assert_eq!(growable.cost_lint(), CostLint::ExpensiveGrowth);

        // Very large boundary.
        assert_eq!(
            SizeClass::of(SIZE_CLASS_LARGE_MAX + 1),
            SizeClass::VeryLarge
        );
    }

    #[test]
    fn idl_node_projects_descriptor_fields() {
        let d = AccountDescriptor::headered("Config", 5, 2, 40, [0x42; 8]).with_dynamic_tail();
        let node = d.idl_node();
        assert_eq!(node.name, "Config");
        assert_eq!(node.disc, 5);
        assert_eq!(node.version, 2);
        assert_eq!(node.body_offset, 16);
        assert_eq!(node.body_size, 40);
        assert_eq!(node.kind, LayoutKind::Headered);
        assert!(node.has_dynamic_tail);
        assert!(!node.deprecated);
        assert_eq!(node.layout_id, [0x42; 8]);
        // The node carries the same fingerprint a client checks.
        assert_eq!(node.fingerprint, d.fingerprint());

        let dep = AccountDescriptor::compact("Old", 6, 1, 8, [0; 8]).deprecated();
        assert!(dep.idl_node().deprecated);
        assert_eq!(dep.idl_node().kind, LayoutKind::Compact);
    }
}
