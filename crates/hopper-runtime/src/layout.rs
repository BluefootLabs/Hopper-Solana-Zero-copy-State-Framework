//! Layout contracts as runtime truth.
//!
//! `LayoutContract` is the central trait for Hopper's state-first architecture.
//! It ties together discriminator, version, and layout fingerprint into a single
//! compile-time contract that the runtime can validate before granting typed access.
//!
//! This is what makes Hopper different from every other Solana framework:
//! layouts are not just metadata or serialization hints. They are runtime contracts
//! that gate account access, enforce compatibility, and enable schema evolution.
//!
//! No competitor (Pinocchio, Steel, Quasar) has anything equivalent.

use crate::error::ProgramError;
use crate::field_map::{FieldInfo, FieldMap};
use crate::ProgramResult;

// ══════════════════════════════════════════════════════════════════════
//  HopperHeader -- the 16-byte on-chain header present in every Hopper
//  account.
// ══════════════════════════════════════════════════════════════════════

/// The canonical 16-byte header at the start of every Hopper account.
///
/// The Hopper Safety Audit's "header epoching" recommendation asked
/// the reserved tail to carry a `schema_epoch: u32` so the runtime
/// can distinguish schema-compatible minor versions from wire-
/// incompatible revisions without bumping the single `version` byte.
///
/// ```text
/// byte 0     : disc (u8)
/// byte 1     : version (u8)
/// bytes 2-3  : flags (u16 LE)
/// bytes 4-11 : layout_id (first 8 bytes of canonical wire fingerprint)
/// bytes 12-15: schema_epoch (u32 LE) — audit-added
/// ```
///
/// `schema_epoch` defaults to `1` at account initialisation via
/// [`init_header`]. Programs that publish a migration bump this
/// field to advertise the new shape while retaining the same
/// `disc`/`version`; on-chain manifests (future work) pin the
/// `(disc, version, schema_epoch, layout_id)` tuple so clients can
/// verify they're reading the expected wire format.
#[repr(C, packed)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HopperHeader {
    pub disc: u8,
    pub version: u8,
    pub flags: u16,
    pub layout_id: [u8; 8],
    /// Schema-evolution epoch. Little-endian u32. `1` for freshly
    /// initialised headers; bumped by migration helpers.
    pub schema_epoch: u32,
}

impl HopperHeader {
    /// The header is always 16 bytes.
    pub const SIZE: usize = 16;

    /// Read a header from the start of a raw data slice.
    #[inline(always)]
    pub fn from_bytes(data: &[u8]) -> Option<&Self> {
        if data.len() < Self::SIZE {
            return None;
        }
        // SAFETY: HopperHeader is packed to alignment 1.
        Some(unsafe { &*(data.as_ptr() as *const Self) })
    }

    /// Read a mutable header from the start of a raw data slice.
    #[inline(always)]
    pub fn from_bytes_mut(data: &mut [u8]) -> Option<&mut Self> {
        if data.len() < Self::SIZE {
            return None;
        }
        Some(unsafe { &mut *(data.as_mut_ptr() as *mut Self) })
    }
}

// ══════════════════════════════════════════════════════════════════════
//  LayoutInfo -- runtime-inspectable metadata snapshot
// ══════════════════════════════════════════════════════════════════════

/// Runtime metadata snapshot of an account's layout identity.
///
/// Returned by `AccountView::layout_info()`. Enables manager inspection,
/// schema comparison, and version-aware loading without knowing the
/// concrete layout type at compile time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LayoutInfo {
    pub disc: u8,
    pub version: u8,
    pub flags: u16,
    pub layout_id: [u8; 8],
    /// Schema-evolution epoch read from the header's bytes 12..16.
    /// A value of `0` means "legacy" (pre-audit accounts) and is
    /// treated as equivalent to `DEFAULT_SCHEMA_EPOCH` when comparing
    /// against `AccountLayout::SCHEMA_EPOCH`.
    pub schema_epoch: u32,
    pub data_len: usize,
}

impl LayoutInfo {
    /// Read layout info from an account's raw data.
    #[inline(always)]
    pub fn from_data(data: &[u8]) -> Option<Self> {
        let hdr = HopperHeader::from_bytes(data)?;
        // Packed-struct field reads must go through a copy — reading
        // an unaligned `u32` reference directly is undefined behaviour.
        let schema_epoch = hdr.schema_epoch;
        let layout_id = hdr.layout_id;
        Some(Self {
            disc: hdr.disc,
            version: hdr.version,
            flags: hdr.flags,
            layout_id,
            schema_epoch,
            data_len: data.len(),
        })
    }

    /// Whether this account matches the given layout contract.
    #[inline(always)]
    pub fn matches<T: LayoutContract>(&self) -> bool {
        self.disc == T::DISC
            && self.version == T::VERSION
            && self.layout_id == T::LAYOUT_ID
            && self.data_len >= T::required_len()
    }

    /// Length of the account body after the Hopper header.
    #[inline(always)]
    pub const fn body_len(&self) -> usize {
        self.data_len.saturating_sub(HopperHeader::SIZE)
    }

    /// Whether the account contains bytes beyond a given absolute offset.
    #[inline(always)]
    pub const fn has_bytes_after(&self, offset: usize) -> bool {
        self.data_len > offset
    }
}

// ══════════════════════════════════════════════════════════════════════
//  LayoutContract -- the central state contract trait
// ══════════════════════════════════════════════════════════════════════

/// A compile-time layout contract binding type identity to wire format.
///
/// Implementors declare their discriminator, version, layout fingerprint,
/// and wire size. The runtime uses these to validate accounts before granting
/// typed access via `overlay` or `load`.
///
/// # Wire format (Hopper account header)
///
/// ```text
/// byte 0   : discriminator (u8)
/// byte 1   : version (u8)
/// bytes 2-3: flags (u16 LE)
/// bytes 4-11: layout_id (first 8 bytes of SHA-256 fingerprint)
/// bytes 12-15: reserved
/// ```
///
/// # Example
///
/// ```ignore
/// impl LayoutContract for Vault {
///     const DISC: u8 = 1;
///     const VERSION: u8 = 1;
///     const LAYOUT_ID: [u8; 8] = compute_layout_id("Vault", 1, "authority:[u8;32]:32,balance:LeU64:8,");
///     const SIZE: usize = 16 + 32 + 8; // header + fields
/// }
/// ```
pub trait LayoutContract: Sized + Copy + FieldMap {
    /// Account type discriminator (byte 0 of data).
    const DISC: u8;

    /// Schema version for this layout (byte 1 of data).
    const VERSION: u8;

    /// First 8 bytes of the deterministic layout fingerprint.
    /// Computed from `SHA-256("hopper:v1:" + name + ":" + version + ":" + field_spec)`.
    const LAYOUT_ID: [u8; 8];

    /// Total wire size in bytes (including the 16-byte header).
    const SIZE: usize;

    /// Byte offset where the typed projection begins.
    ///
    /// Body-only runtime layouts keep the default `HopperHeader::SIZE`, while
    /// header-inclusive layouts set this to `0` so `AccountView::load()`
    /// projects the full account struct.
    const TYPE_OFFSET: usize = HopperHeader::SIZE;

    /// Number of reserved bytes at the end of the layout. Reserved bytes
    /// provide forward-compatible padding that future versions can claim
    /// without a realloc.
    const RESERVED_BYTES: usize = 0;

    /// Byte offset where an extension region begins, if the layout supports one.
    /// Extension regions allow appending variable-length data beyond the fixed
    /// layout without breaking existing readers.
    const EXTENSION_OFFSET: Option<usize> = None;

    /// Validate a raw data slice against this contract.
    ///
    /// Returns `Ok(())` if the discriminator, version, and layout_id all match.
    /// This is the canonical "is this account what I think it is?" check.
    #[inline(always)]
    fn validate_header(data: &[u8]) -> ProgramResult {
        if data.len() < Self::required_len() {
            return ProgramError::err_data_too_small();
        }
        let disc = read_disc(data);
        if disc != Some(Self::DISC) {
            return ProgramError::err_invalid_data();
        }
        let version = read_version(data);
        if version != Some(Self::VERSION) {
            return ProgramError::err_invalid_data();
        }
        if let Some(id) = read_layout_id(data) {
            if *id != Self::LAYOUT_ID {
                return ProgramError::err_invalid_data();
            }
        } else {
            return ProgramError::err_data_too_small();
        }
        Ok(())
    }

    /// Byte length required to project this typed view safely.
    #[inline(always)]
    fn projected_len() -> usize {
        Self::TYPE_OFFSET + core::mem::size_of::<Self>()
    }

    /// Minimum account data length required by both the wire contract and projection shape.
    #[inline(always)]
    fn required_len() -> usize {
        if Self::SIZE > Self::projected_len() {
            Self::SIZE
        } else {
            Self::projected_len()
        }
    }

    /// Lightweight boolean validation helper for foreign readers and tools.
    #[inline(always)]
    fn validate(data: &[u8]) -> bool {
        Self::validate_header(data).is_ok()
    }

    /// Check only the discriminator (fast path for dispatch).
    #[inline(always)]
    fn check_disc(data: &[u8]) -> ProgramResult {
        match read_disc(data) {
            Some(d) if d == Self::DISC => Ok(()),
            _ => ProgramError::err_invalid_data(),
        }
    }

    /// Check only the version (for migration gates).
    #[inline(always)]
    fn check_version(data: &[u8]) -> ProgramResult {
        match read_version(data) {
            Some(v) if v == Self::VERSION => Ok(()),
            _ => ProgramError::err_invalid_data(),
        }
    }

    /// Check whether a given version is compatible with this layout.
    ///
    /// The default implementation accepts only the exact version, but
    /// implementors can override this to accept older versions for
    /// backward-compatible migration.
    #[inline(always)]
    fn compatible(version: u8) -> bool {
        version == Self::VERSION
    }

    /// Check whether the account data contains an extension region
    /// (data beyond the fixed layout boundary).
    #[inline(always)]
    fn has_extension_region(data: &[u8]) -> bool {
        match Self::EXTENSION_OFFSET {
            Some(offset) => data.len() > offset,
            None => false,
        }
    }

    /// Build a `LayoutInfo` snapshot from this contract's compile-time constants.
    #[inline(always)]
    fn layout_info_static() -> LayoutInfo {
        LayoutInfo {
            disc: Self::DISC,
            version: Self::VERSION,
            flags: 0,
            layout_id: Self::LAYOUT_ID,
            schema_epoch: DEFAULT_SCHEMA_EPOCH,
            data_len: Self::required_len(),
        }
    }

    /// Compile-time field metadata for this layout.
    #[inline(always)]
    fn fields() -> &'static [FieldInfo] {
        Self::FIELDS
    }
}

/// Read the discriminator from account data (byte 0).
#[inline(always)]
pub fn read_disc(data: &[u8]) -> Option<u8> {
    data.first().copied()
}

/// Read the version from account data (byte 1).
#[inline(always)]
pub fn read_version(data: &[u8]) -> Option<u8> {
    if data.len() < 2 { None } else { Some(data[1]) }
}

/// Read the 8-byte layout_id from account data (bytes 4..12).
#[inline(always)]
pub fn read_layout_id(data: &[u8]) -> Option<&[u8; 8]> {
    if data.len() < 12 {
        None
    } else {
        // SAFETY: bounds checked above, alignment is 1 for [u8; 8].
        Some(unsafe { &*(data.as_ptr().add(4) as *const [u8; 8]) })
    }
}

/// Read the flags from account data (bytes 2..4) as u16 LE.
#[inline(always)]
pub fn read_flags(data: &[u8]) -> Option<u16> {
    if data.len() < 4 {
        None
    } else {
        let bytes = [data[2], data[3]];
        Some(u16::from_le_bytes(bytes))
    }
}

/// Default schema-evolution epoch written by `init_header`.
///
/// Accounts initialised by pre-audit Hopper had the epoch region
/// zeroed, so `0` is treated as "legacy, equivalent to 1" by the
/// runtime checks that compare against an `AccountLayout::SCHEMA_EPOCH`.
/// Freshly-initialised accounts now carry `1` so migrations can bump
/// monotonically without any lookback.
pub const DEFAULT_SCHEMA_EPOCH: u32 = 1;

/// Write a complete Hopper header to the beginning of `data`.
///
/// Writes disc, version, flags (zeroed), layout_id, and the
/// audit-added `schema_epoch = 1` (bytes 12..16).
/// Returns `Err` if `data` is shorter than 16 bytes.
#[inline(always)]
pub fn write_header(
    data: &mut [u8],
    disc: u8,
    version: u8,
    layout_id: &[u8; 8],
) -> ProgramResult {
    write_header_with_epoch(data, disc, version, layout_id, DEFAULT_SCHEMA_EPOCH)
}

/// Write a Hopper header with a caller-specified schema epoch.
///
/// Used by migration helpers that need to stamp a new epoch while
/// preserving `disc`/`version`/`layout_id`. Regular account creation
/// should go through [`write_header`] (which defaults the epoch to
/// `1`) or [`init_header`].
#[inline(always)]
pub fn write_header_with_epoch(
    data: &mut [u8],
    disc: u8,
    version: u8,
    layout_id: &[u8; 8],
    schema_epoch: u32,
) -> ProgramResult {
    if data.len() < 16 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    data[0] = disc;
    data[1] = version;
    data[2] = 0;
    data[3] = 0;
    data[4..12].copy_from_slice(layout_id);
    data[12..16].copy_from_slice(&schema_epoch.to_le_bytes());
    Ok(())
}

/// Read the `schema_epoch` field from an already-written header.
///
/// Returns `None` if `data` is too short. Returns the stored value
/// verbatim — callers that want the "0 means legacy" compatibility
/// rule should apply it themselves:
///
/// ```ignore
/// let stored = read_schema_epoch(data)?;
/// let effective = if stored == 0 { DEFAULT_SCHEMA_EPOCH } else { stored };
/// ```
#[inline(always)]
pub fn read_schema_epoch(data: &[u8]) -> Option<u32> {
    if data.len() < 16 {
        return None;
    }
    Some(u32::from_le_bytes([data[12], data[13], data[14], data[15]]))
}

/// Initialize an account's header from a layout contract type.
///
/// Convenience wrapper that pulls disc, version, and layout_id from
/// the type and stamps `schema_epoch = DEFAULT_SCHEMA_EPOCH`.
#[inline(always)]
pub fn init_header<T: LayoutContract>(data: &mut [u8]) -> ProgramResult {
    write_header(data, T::DISC, T::VERSION, &T::LAYOUT_ID)
}
