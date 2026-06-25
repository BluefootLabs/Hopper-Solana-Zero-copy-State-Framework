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

impl Default for ManifestProfile {
    #[inline]
    fn default() -> Self {
        Self::Offchain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_rejects_bad_magic_and_short_buffers() {
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
}
