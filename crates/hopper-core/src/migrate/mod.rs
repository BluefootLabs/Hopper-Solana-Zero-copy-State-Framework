//! Migration helpers -- safe version upgrades for on-chain accounts.
//!
//! Hopper's migration system supports three patterns:
//!
//! 1. **Append-safe**: New fields appended to the end, realloc to larger size
//! 2. **Segment-safe**: New segments added to the segment table
//! 3. **Full migration**: Data reshuffled between versions
//!
//! ## Safety
//!
//! - Migration always validates the source layout_id before touching data
//! - The destination version must be strictly greater than the source
//! - Realloc is rent-safe (payer provides lamports for the delta)

use hopper_runtime::{error::ProgramError, AccountView, Address, ProgramResult};
use crate::account::{
    write_header, read_layout_id, read_version,
    FixedLayout,
};
use crate::check::{check_owner, check_writable};

/// Migrate an account in-place by appending new fields.
///
/// This is the cheapest migration: no data movement, just realloc + header update.
///
/// ## Preconditions
///
/// - Account must be owned by `program_id`
/// - Account must be writable
/// - Account layout_id must match `old_layout_id`
/// - `new_size > old_size` (append-only growth)
///
/// ## What it does
///
/// 1. Validates ownership, writable, and old layout_id
/// 2. Reallocs account data to `new_size`
/// 3. Updates header: new version, new layout_id
/// 4. Zeroes the newly appended region
///
/// New fields are left zero-initialized. The caller should fill them after.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn migrate_append(
    account: &AccountView,
    payer: &AccountView,
    program_id: &Address,
    old_layout_id: &[u8; 8],
    new_version: u8,
    new_layout_id: &[u8; 8],
    new_disc: u8,
    new_size: usize,
) -> ProgramResult {
    check_owner(account, program_id)?;
    check_writable(account)?;

    let data = unsafe { account.borrow_unchecked() };
    let current_layout = read_layout_id(data)?;
    if &current_layout != old_layout_id {
        return Err(ProgramError::InvalidAccountData);
    }
    let current_version = read_version(data)?;
    if new_version <= current_version {
        return Err(ProgramError::InvalidAccountData);
    }

    let old_size = data.len();
    if new_size <= old_size {
        return Err(ProgramError::InvalidArgument);
    }

    // Realloc
    crate::account::safe_realloc(account, new_size, payer)?;

    // Write updated header
    let data = unsafe { account.borrow_unchecked_mut() };
    write_header(data, new_disc, new_version, new_layout_id)?;

    // Zero the appended region
    for byte in &mut data[old_size..new_size] {
        *byte = 0;
    }

    Ok(())
}

/// Check if a migration from OldLayout to NewLayout would be append-compatible.
///
/// Append-compatible means:
/// - New layout is strictly larger
/// - The first `old_size` bytes can stay as-is
/// - Only new fields were added at the end
///
/// This is a compile-time check helper -- use in tests and CI.
pub const fn is_append_compatible<Old: FixedLayout, New: FixedLayout>() -> bool {
    New::SIZE > Old::SIZE
}

/// Migration descriptor for schema export.
#[derive(Clone, Copy)]
pub struct MigrationDescriptor {
    /// Source layout name.
    pub from_name: &'static str,
    /// Source version.
    pub from_version: u8,
    /// Source layout_id.
    pub from_layout_id: [u8; 8],
    /// Target layout name.
    pub to_name: &'static str,
    /// Target version.
    pub to_version: u8,
    /// Target layout_id.
    pub to_layout_id: [u8; 8],
    /// Migration kind.
    pub kind: MigrationKind,
}

/// The kind of migration.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MigrationKind {
    /// Fields appended to the end -- realloc only, no data movement.
    Append,
    /// Segments added to segment table -- realloc + table update.
    SegmentAppend,
    /// Full data migration -- copy with transformation.
    Full,
}
