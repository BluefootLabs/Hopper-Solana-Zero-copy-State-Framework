//! # Snapshot diff
//!
//! Off-chain symmetric of `hopper-core::diff`. Given the raw `before` and
//! `after` blobs of a Hopper account, this module produces a structured
//! field-level diff using the layout manifest.
//!
//! Indexers care about this because receipts only tell them *which* fields
//! changed by index. This module answers *what they changed to*.

use hopper_schema::{FieldDescriptor, LayoutManifest};

#[cfg(feature = "std")]
use alloc::vec::Vec;

/// One field's before/after value, as raw bytes.
#[derive(Debug, Clone, Copy)]
pub struct FieldDelta<'a> {
    /// The field that changed.
    pub field: &'a FieldDescriptor,
    /// Previous bytes (length `field.size`).
    pub before: &'a [u8],
    /// Next bytes (length `field.size`).
    pub after: &'a [u8],
}

/// Error surface for diff operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffError {
    /// `before` and `after` lengths differ; resize deltas require a richer
    /// diff type (future work. see `DiffWithResize`).
    LengthMismatch {
        /// Length of `before`.
        before: usize,
        /// Length of `after`.
        after: usize,
    },
    /// Either input was shorter than the manifest's `total_size`.
    BufferTooShort,
}

/// Compute a fixed-size diff (no resize). Returns a list of `FieldDelta`s
///. one per changed field.
///
/// # Errors
/// - `LengthMismatch` if `before.len() != after.len()`.
/// - `BufferTooShort` if either input is shorter than `manifest.total_size`.
#[cfg(feature = "std")]
pub fn fixed_size_diff<'a>(
    before: &'a [u8],
    after: &'a [u8],
    manifest: &'a LayoutManifest,
) -> Result<Vec<FieldDelta<'a>>, DiffError> {
    if before.len() != after.len() {
        return Err(DiffError::LengthMismatch {
            before: before.len(),
            after: after.len(),
        });
    }
    if before.len() < manifest.total_size {
        return Err(DiffError::BufferTooShort);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < manifest.fields.len() {
        let f = &manifest.fields[i];
        let start = f.offset as usize;
        let end = start + f.size as usize;
        if end > before.len() { break; }
        if before[start..end] != after[start..end] {
            out.push(FieldDelta {
                field: f,
                before: &before[start..end],
                after: &after[start..end],
            });
        }
        i += 1;
    }
    Ok(out)
}

/// Bitmask version. same scan but returns a `u64` whose `i`th bit is set
/// when the `i`th field differs. Useful when comparing to the `changed_fields`
/// mask from a receipt.
pub fn field_change_mask(
    before: &[u8],
    after: &[u8],
    manifest: &LayoutManifest,
) -> u64 {
    let mut mask = 0u64;
    let common = core::cmp::min(before.len(), after.len());
    let mut i = 0;
    while i < manifest.fields.len() && i < 64 {
        let f = &manifest.fields[i];
        let start = f.offset as usize;
        let end = start + f.size as usize;
        if end > common { break; }
        if before[start..end] != after[start..end] {
            mask |= 1u64 << i;
        }
        i += 1;
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_schema::FieldIntent;

    fn fields() -> &'static [FieldDescriptor] {
        static F: [FieldDescriptor; 2] = [
            FieldDescriptor { name: "a", canonical_type: "u64", size: 8, offset: 0,  intent: FieldIntent::Counter },
            FieldDescriptor { name: "b", canonical_type: "u64", size: 8, offset: 8,  intent: FieldIntent::Balance },
        ];
        &F
    }

    fn manifest() -> LayoutManifest {
        LayoutManifest {
            name: "Pair", disc: 1, version: 1, layout_id: [0; 8],
            total_size: 16, field_count: 2, fields: fields(),
        }
    }

    #[test]
    fn mask_detects_changed_field() {
        let mut before = [0u8; 16];
        let mut after  = [0u8; 16];
        before[8..16].copy_from_slice(&1u64.to_le_bytes());
        after[8..16].copy_from_slice(&2u64.to_le_bytes());
        let m = manifest();
        assert_eq!(field_change_mask(&before, &after, &m), 0b10);
    }

    #[cfg(feature = "std")]
    #[test]
    fn fixed_size_diff_returns_deltas() {
        let mut before = [0u8; 16];
        let mut after  = [0u8; 16];
        after[0..8].copy_from_slice(&5u64.to_le_bytes());
        let m = manifest();
        let d = fixed_size_diff(&before, &after, &m).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].field.name, "a");
    }
}
