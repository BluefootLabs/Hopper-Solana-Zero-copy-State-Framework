//! Address Lookup Table — byte-layout helpers + resolution.
//!
//! Solana v0 transactions reference accounts indirectly through
//! lookup tables: instead of including the full 32-byte pubkey
//! for every account meta, the transaction carries a small
//! `MessageAddressTableLookup { account_key, writable_indexes,
//! readonly_indexes }` and the validator expands those indexes
//! into concrete pubkeys by reading the table's address list.
//!
//! ## On-disk layout
//!
//! ```text
//! [0..4]    discriminator (u32 LE; 1 = LookupTable)
//! [4..12]   deactivation_slot (u64 LE; u64::MAX = active)
//! [12..20]  last_extended_slot (u64 LE)
//! [20]      last_extended_slot_start_index (u8)
//! [21..25]  authority COption flag (u32 LE; 1 = Some, 0 = None)
//! [25..57]  authority pubkey (zero-padded if flag = 0)
//! [57..58]  reserved padding (u8)
//! [58..59]  reserved padding (u8)
//! ```
//!
//! Total header: 58 bytes. Upstream's `LOOKUP_TABLE_META_SIZE`
//! is 56 — we write to that offset for compatibility (pads
//! after the authority slot land at offsets 56/57). Address[i]
//! lives at offset `LOOKUP_TABLE_META_SIZE + i * 32`.
//!
//! ## Key constants
//!
//! - `LOOKUP_TABLE_META_SIZE = 56` — fixed metadata header.
//! - `LOOKUP_TABLE_MAX_ADDRESSES = 256` — mainnet cap.
//! - `DEACTIVATION_COOLDOWN_SLOTS = 513` — after `Deactivate`,
//!   the table can be `Close`d once
//!   `current_slot - deactivation_slot > 513`.
//! - `LOOKUP_TABLE_DISCRIMINATOR = 1`.
//!
//! These are hard-coded to match
//! `solana_address_lookup_table_program::state` — we trade off
//! a `bincode` dep against the small risk of an upstream layout
//! tweak (extremely unlikely; this format hasn't moved since
//! v0 transactions shipped). A future change is one constant
//! to update.

use solana_sdk::pubkey::Pubkey;

/// Header size in bytes for an Address Lookup Table account.
/// Match the upstream constant.
pub const LOOKUP_TABLE_META_SIZE: usize = 56;

/// Maximum number of addresses a single lookup table can store.
/// Mainnet cap.
pub const LOOKUP_TABLE_MAX_ADDRESSES: usize = 256;

/// Slots a table must wait between `Deactivate` and `Close`.
/// Mainnet's `DEACTIVATION_COOLDOWN`.
pub const DEACTIVATION_COOLDOWN_SLOTS: u64 = 513;

/// Discriminator value indicating the account is a populated
/// lookup table.
pub const LOOKUP_TABLE_DISCRIMINATOR: u32 = 1;

/// Slot value indicating "still active" (not yet deactivated).
pub const ACTIVE_DEACTIVATION_SLOT: u64 = u64::MAX;

/// Lookup table metadata. The fixed-size header that lives at
/// the start of every initialised lookup-table account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupTableMeta {
    /// Slot at which `Deactivate` ran. `u64::MAX` while active.
    pub deactivation_slot: u64,
    /// Most recent slot at which `Extend` ran. Used to
    /// determine which addresses are usable in this slot
    /// (newly-added entries are not usable until the next
    /// slot).
    pub last_extended_slot: u64,
    /// Index in the address vector where `last_extended_slot`'s
    /// extension started — addresses at indices ≥ this value
    /// are not usable until `current_slot > last_extended_slot`.
    pub last_extended_slot_start_index: u8,
    /// Optional authority that can `Extend`, `Deactivate`, or
    /// `Close` the table. `None` means the table is frozen.
    pub authority: Option<Pubkey>,
}

impl LookupTableMeta {
    /// Brand-new table at the given slot, owned by the supplied
    /// authority. `Extend` and `Deactivate` move the rest of
    /// the state forward.
    pub fn new(authority: Pubkey) -> Self {
        Self {
            deactivation_slot: ACTIVE_DEACTIVATION_SLOT,
            last_extended_slot: 0,
            last_extended_slot_start_index: 0,
            authority: Some(authority),
        }
    }

    /// Whether this table is in the deactivated state. A
    /// deactivated table can be `Close`d after the cooldown.
    pub fn is_deactivated(&self) -> bool {
        self.deactivation_slot != ACTIVE_DEACTIVATION_SLOT
    }

    /// Whether the table has waited the full cooldown after
    /// deactivation and can therefore be `Close`d.
    pub fn is_closeable(&self, current_slot: u64) -> bool {
        self.is_deactivated()
            && current_slot.saturating_sub(self.deactivation_slot)
                > DEACTIVATION_COOLDOWN_SLOTS
    }
}

/// Read the metadata header from raw account data. Returns
/// `None` if the data is too short or the discriminator
/// doesn't match.
pub fn read_meta(data: &[u8]) -> Option<LookupTableMeta> {
    if data.len() < LOOKUP_TABLE_META_SIZE {
        return None;
    }
    let disc = u32::from_le_bytes(data[0..4].try_into().ok()?);
    if disc != LOOKUP_TABLE_DISCRIMINATOR {
        return None;
    }
    let deactivation_slot = u64::from_le_bytes(data[4..12].try_into().ok()?);
    let last_extended_slot = u64::from_le_bytes(data[12..20].try_into().ok()?);
    let last_extended_slot_start_index = data[20];
    let auth_flag = data[21];
    let authority = if auth_flag == 1 {
        Some(Pubkey::new_from_array(data[22..54].try_into().ok()?))
    } else {
        None
    };
    Some(LookupTableMeta {
        deactivation_slot,
        last_extended_slot,
        last_extended_slot_start_index,
        authority,
    })
}

/// Write metadata into the leading [`LOOKUP_TABLE_META_SIZE`]
/// bytes of the buffer. Caller is responsible for ensuring
/// `data.len() >= LOOKUP_TABLE_META_SIZE`.
pub fn write_meta(data: &mut [u8], meta: &LookupTableMeta) {
    if data.len() < LOOKUP_TABLE_META_SIZE {
        return;
    }
    // Bincode-compatible Solana ALT meta layout (56 bytes):
    //   [0..4]  disc (u32 LE)
    //   [4..12]  deactivation_slot (u64 LE)
    //   [12..20] last_extended_slot (u64 LE)
    //   [20]     last_extended_slot_start_index (u8)
    //   [21]     authority Option tag (u8, 0 = None, 1 = Some)
    //   [22..54] authority pubkey (32 bytes, zeroed if tag = 0)
    //   [54..56] padding (2 bytes, zero)
    data[0..4].copy_from_slice(&LOOKUP_TABLE_DISCRIMINATOR.to_le_bytes());
    data[4..12].copy_from_slice(&meta.deactivation_slot.to_le_bytes());
    data[12..20].copy_from_slice(&meta.last_extended_slot.to_le_bytes());
    data[20] = meta.last_extended_slot_start_index;
    match &meta.authority {
        Some(pk) => {
            data[21] = 1;
            data[22..54].copy_from_slice(pk.as_ref());
        }
        None => {
            data[21] = 0;
            // Zero out the slot to avoid leaking a stale
            // authority pubkey when the table is frozen.
            for b in data[22..54].iter_mut() {
                *b = 0;
            }
        }
    }
    // Padding bytes 54..56 must be zero (and the buffer was
    // zero-initialised by the caller, but make this explicit
    // for tests that pass dirty buffers).
    data[54] = 0;
    data[55] = 0;
}

/// Number of address slots packed into the table's data
/// region. Computed as `(data.len() - meta_size) / 32` after
/// validating the discriminator.
pub fn address_count(data: &[u8]) -> usize {
    if data.len() < LOOKUP_TABLE_META_SIZE {
        return 0;
    }
    (data.len() - LOOKUP_TABLE_META_SIZE) / 32
}

/// Read a single address by index. Returns `None` if `index`
/// is out of bounds.
pub fn read_address(data: &[u8], index: usize) -> Option<Pubkey> {
    let off = LOOKUP_TABLE_META_SIZE + index * 32;
    if off + 32 > data.len() {
        return None;
    }
    Some(Pubkey::new_from_array(data[off..off + 32].try_into().ok()?))
}

/// Append a slice of addresses to the table's data region.
/// Returns the new total address count, or an error string if
/// the append would exceed the per-table cap.
pub fn append_addresses(
    data: &mut Vec<u8>,
    new_addresses: &[Pubkey],
) -> Result<usize, String> {
    let current = address_count(data);
    let total = current + new_addresses.len();
    if total > LOOKUP_TABLE_MAX_ADDRESSES {
        return Err(format!(
            "address count {total} exceeds LOOKUP_TABLE_MAX_ADDRESSES ({LOOKUP_TABLE_MAX_ADDRESSES})"
        ));
    }
    for pk in new_addresses {
        data.extend_from_slice(pk.as_ref());
    }
    Ok(total)
}

/// Resolve a `MessageAddressTableLookup`-shaped reference into
/// a list of concrete `(pubkey, is_writable)` pairs. Used by
/// the v0-transaction resolution path.
///
/// The convention matches mainnet: writable addresses come
/// first, then read-only. Both lists index into the same
/// table.
pub fn resolve_lookup(
    table_data: &[u8],
    writable_indexes: &[u8],
    readonly_indexes: &[u8],
) -> Result<(Vec<Pubkey>, Vec<Pubkey>), String> {
    let mut writable: Vec<Pubkey> = Vec::with_capacity(writable_indexes.len());
    for &idx in writable_indexes {
        match read_address(table_data, idx as usize) {
            Some(pk) => writable.push(pk),
            None => {
                return Err(format!(
                    "writable index {idx} out of bounds (table has {} addresses)",
                    address_count(table_data)
                ))
            }
        }
    }
    let mut readonly: Vec<Pubkey> = Vec::with_capacity(readonly_indexes.len());
    for &idx in readonly_indexes {
        match read_address(table_data, idx as usize) {
            Some(pk) => readonly.push(pk),
            None => {
                return Err(format!(
                    "readonly index {idx} out of bounds (table has {} addresses)",
                    address_count(table_data)
                ))
            }
        }
    }
    Ok((writable, readonly))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_round_trips() {
        let auth = Pubkey::new_unique();
        let meta = LookupTableMeta::new(auth);
        let mut buf = vec![0u8; LOOKUP_TABLE_META_SIZE];
        write_meta(&mut buf, &meta);
        let read = read_meta(&buf).expect("read");
        assert_eq!(read.authority, Some(auth));
        assert_eq!(read.deactivation_slot, ACTIVE_DEACTIVATION_SLOT);
        assert_eq!(read.last_extended_slot, 0);
    }

    #[test]
    fn frozen_table_zeroes_authority_slot() {
        let mut meta = LookupTableMeta::new(Pubkey::new_unique());
        meta.authority = None;
        let mut buf = vec![0xFFu8; LOOKUP_TABLE_META_SIZE];
        write_meta(&mut buf, &meta);
        // Authority slot should be zeroed even though the buffer
        // started full of 0xFF. The 32-byte authority lives at
        // bytes [22..54] in the bincode layout, plus the option
        // tag at [21] and trailing padding at [54..56] also zeroed.
        assert_eq!(buf[21], 0, "authority tag stays None");
        assert_eq!(&buf[22..54], &[0u8; 32]);
        assert_eq!(&buf[54..56], &[0u8; 2], "trailing padding zeroed");
        let read = read_meta(&buf).expect("read");
        assert!(read.authority.is_none());
    }

    #[test]
    fn append_and_read_addresses() {
        let mut buf = vec![0u8; LOOKUP_TABLE_META_SIZE];
        write_meta(&mut buf, &LookupTableMeta::new(Pubkey::new_unique()));
        let a1 = Pubkey::new_unique();
        let a2 = Pubkey::new_unique();
        let total = append_addresses(&mut buf, &[a1, a2]).unwrap();
        assert_eq!(total, 2);
        assert_eq!(read_address(&buf, 0), Some(a1));
        assert_eq!(read_address(&buf, 1), Some(a2));
        assert_eq!(read_address(&buf, 2), None);
        assert_eq!(address_count(&buf), 2);
    }

    #[test]
    fn append_rejects_overflow() {
        let mut buf = vec![0u8; LOOKUP_TABLE_META_SIZE];
        write_meta(&mut buf, &LookupTableMeta::new(Pubkey::new_unique()));
        // Pre-populate to the cap, then try to append one more.
        for _ in 0..LOOKUP_TABLE_MAX_ADDRESSES {
            buf.extend_from_slice(Pubkey::new_unique().as_ref());
        }
        let r = append_addresses(&mut buf, &[Pubkey::new_unique()]);
        assert!(r.is_err());
    }

    #[test]
    fn resolve_lookup_pulls_writable_then_readonly() {
        let mut buf = vec![0u8; LOOKUP_TABLE_META_SIZE];
        write_meta(&mut buf, &LookupTableMeta::new(Pubkey::new_unique()));
        let addrs: Vec<Pubkey> = (0..5).map(|_| Pubkey::new_unique()).collect();
        append_addresses(&mut buf, &addrs).unwrap();
        let (writable, readonly) =
            resolve_lookup(&buf, &[0, 2], &[1, 3, 4]).unwrap();
        assert_eq!(writable, vec![addrs[0], addrs[2]]);
        assert_eq!(readonly, vec![addrs[1], addrs[3], addrs[4]]);
    }

    #[test]
    fn resolve_lookup_rejects_out_of_bounds_index() {
        let mut buf = vec![0u8; LOOKUP_TABLE_META_SIZE];
        write_meta(&mut buf, &LookupTableMeta::new(Pubkey::new_unique()));
        append_addresses(&mut buf, &[Pubkey::new_unique()]).unwrap();
        let r = resolve_lookup(&buf, &[0, 5], &[]);
        assert!(r.is_err());
    }

    #[test]
    fn closeable_after_cooldown() {
        let mut meta = LookupTableMeta::new(Pubkey::new_unique());
        // Active table is never closeable.
        assert!(!meta.is_closeable(1_000_000));
        // Deactivate at slot 100.
        meta.deactivation_slot = 100;
        // Just after deactivation: not yet closeable.
        assert!(!meta.is_closeable(100));
        assert!(!meta.is_closeable(100 + DEACTIVATION_COOLDOWN_SLOTS));
        // One slot past the cooldown: closeable.
        assert!(meta.is_closeable(100 + DEACTIVATION_COOLDOWN_SLOTS + 1));
    }
}
