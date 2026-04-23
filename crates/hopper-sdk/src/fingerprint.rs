//! # Layout fingerprint verification
//!
//! Before a client decodes an account, it should verify that the account
//! header's `layout_id` matches the fingerprint the client was compiled
//! against. This prevents the classic "program was redeployed with a new
//! layout and my client silently misparsed the bytes" bug class.
//!
//! The on-chain header layout is documented in `hopper-core::account::header`;
//! the layout_id fingerprint is stored at offset **4..12** of the account
//! header. This module knows exactly that and nothing more.

use hopper_schema::{LayoutManifest, ProgramIdl};

/// Byte offset of the layout_id within the Hopper account header.
///
/// Header layout: `[0..4] = disc|version|flags|reserved`, `[4..12] = layout_id`.
pub const LAYOUT_ID_OFFSET: usize = 4;

/// Result of running a fingerprint check against some account bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintCheck {
    /// Bytes matched the expected layout_id.
    Match,
    /// Bytes decoded but the layout_id differs.
    Mismatch {
        /// Expected fingerprint (from manifest).
        expected: [u8; 8],
        /// Fingerprint actually on-chain.
        actual: [u8; 8],
    },
}

/// Error surface for fingerprint operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintError {
    /// Account bytes too short to contain a layout_id (< 12 bytes).
    TooShort,
    /// No matching layout manifest for the supplied name.
    UnknownLayout,
}

/// Read the layout_id from a Hopper account's raw bytes.
///
/// Returns `Err(TooShort)` if there aren't at least 12 bytes.
pub fn read_layout_id(bytes: &[u8]) -> Result<[u8; 8], FingerprintError> {
    if bytes.len() < LAYOUT_ID_OFFSET + 8 {
        return Err(FingerprintError::TooShort);
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&bytes[LAYOUT_ID_OFFSET..LAYOUT_ID_OFFSET + 8]);
    Ok(id)
}

/// Check a raw account blob against a specific `LayoutManifest`.
pub fn check_against_layout(
    bytes: &[u8],
    layout: &LayoutManifest,
) -> Result<FingerprintCheck, FingerprintError> {
    let actual = read_layout_id(bytes)?;
    if actual == layout.layout_id {
        Ok(FingerprintCheck::Match)
    } else {
        Ok(FingerprintCheck::Mismatch {
            expected: layout.layout_id,
            actual,
        })
    }
}

/// Check a raw account blob against the layout with the given name in the
/// supplied `ProgramIdl`.
pub fn check_in_idl(
    bytes: &[u8],
    idl: &ProgramIdl,
    layout_name: &str,
) -> Result<FingerprintCheck, FingerprintError> {
    let layout = idl
        .find_account(layout_name)
        .ok_or(FingerprintError::UnknownLayout)?;
    check_against_layout(bytes, layout)
}

/// Identify which layout in a `ProgramIdl` a blob belongs to by fingerprint.
/// Returns `None` if no layout matches.
pub fn identify_in_idl<'a>(
    bytes: &[u8],
    idl: &'a ProgramIdl,
) -> Option<&'a LayoutManifest> {
    let actual = read_layout_id(bytes).ok()?;
    let mut i = 0;
    while i < idl.accounts.len() {
        if idl.accounts[i].layout_id == actual {
            return Some(&idl.accounts[i]);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper_schema::FieldDescriptor;

    const SAMPLE_ID: [u8; 8] = [9, 8, 7, 6, 5, 4, 3, 2];

    fn mk_manifest() -> LayoutManifest {
        LayoutManifest {
            name: "Vault",
            disc: 7,
            version: 1,
            layout_id: SAMPLE_ID,
            total_size: 80,
            field_count: 0,
            fields: &[] as &[FieldDescriptor],
        }
    }

    fn mk_bytes(id: [u8; 8]) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = 7; // disc
        b[1] = 1; // version
        b[LAYOUT_ID_OFFSET..LAYOUT_ID_OFFSET + 8].copy_from_slice(&id);
        b
    }

    #[test]
    fn matches_when_equal() {
        let bytes = mk_bytes(SAMPLE_ID);
        let layout = mk_manifest();
        assert_eq!(
            check_against_layout(&bytes, &layout).unwrap(),
            FingerprintCheck::Match
        );
    }

    #[test]
    fn reports_mismatch_with_actual() {
        let other = [1, 1, 1, 1, 1, 1, 1, 1];
        let bytes = mk_bytes(other);
        let layout = mk_manifest();
        let check = check_against_layout(&bytes, &layout).unwrap();
        assert_eq!(
            check,
            FingerprintCheck::Mismatch {
                expected: SAMPLE_ID,
                actual: other,
            }
        );
    }

    #[test]
    fn too_short_returns_error() {
        let bytes = [0u8; 6];
        let layout = mk_manifest();
        assert_eq!(
            check_against_layout(&bytes, &layout),
            Err(FingerprintError::TooShort)
        );
    }
}
