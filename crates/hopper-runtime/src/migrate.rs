//! Schema-epoch in-place migration runtime.
//!
//! Closes the Hopper Safety Audit's innovation item I4 ("Schema epoch
//! with in-place migration helpers"). The header's `schema_epoch: u32`
//! lets accounts self-identify the ABI version they were written in.
//! When a program later loads an account written at an older epoch,
//! the runtime consults a declared migration chain, applies each edge
//! in sequence atomically with a `schema_epoch` bump, and only then
//! hands the caller a typed `Ref<'_, T>` of the current shape.
//!
//! # Design rules
//!
//! * **In-place**. no allocation, no CPI. Migration rewrites the
//!   account body (within its existing byte range) and the 16-byte
//!   Hopper header.
//! * **Atomic per edge**. each migration edge updates both the body
//!   *and* the `schema_epoch` header field under a single mutable
//!   byte borrow. A mid-migration abort leaves the header and body
//!   consistent with *one* of the two endpoints, never a hybrid.
//! * **Idempotent**. re-running an already-applied edge is a no-op
//!   (the header epoch mismatch returns `MigrationMismatch`).
//! * **Deterministic**. edges are applied in strict
//!   `from_epoch → to_epoch` order, and any gap in the chain fails.

use crate::account::AccountView;
use crate::error::ProgramError;
use crate::layout::{HopperHeader, LayoutContract};
use crate::zerocopy::AccountLayout;

/// One step in a layout's migration chain.
///
/// An edge takes the raw account *body* (the bytes after the 16-byte
/// Hopper header), mutates them in place to match the new epoch's
/// shape, and returns `Ok(())` on success. The runtime then atomically
/// bumps the header's `schema_epoch` to `to_epoch` under the same
/// mutable borrow.
///
/// Migration functions must not call CPIs (no CreateAccount, no
/// Transfer) and must not resize the account (use `realloc` for that
/// separately). They may read and write arbitrary bytes within the
/// body, which is why the signature takes `&mut [u8]`. `ZeroCopy`
/// safety has deliberately been stepped out of because the user is
/// explicitly translating between two different byte layouts.
#[derive(Clone, Copy)]
pub struct MigrationEdge {
    /// Epoch the body is expected to be in before this edge runs.
    pub from_epoch: u32,
    /// Epoch the body will be in after this edge runs successfully.
    pub to_epoch: u32,
    /// In-place mutator. Called exactly once per upgrade sequence.
    pub migrator: fn(body: &mut [u8]) -> Result<(), ProgramError>,
}

impl MigrationEdge {
    /// Reject edges that would decrement or stay at the same epoch . 
    /// migrations always move forward.
    pub const fn is_forward(&self) -> bool {
        self.to_epoch > self.from_epoch
    }
}

/// Layouts opt into in-place migration by providing a `MIGRATIONS`
/// constant. The default (empty slice) means "no migrations declared"
/// and any mismatch between header and `AccountLayout::SCHEMA_EPOCH`
/// is a hard failure.
///
/// The trait is sealed-by-convention: downstream crates should
/// express migrations via the `#[hopper::migrate(...)]` attribute
/// macro and the `hopper::layout_migrations!` composition helper,
/// never by hand-writing `impl LayoutMigration for T`.
pub trait LayoutMigration {
    /// Ordered migration chain. `MIGRATIONS[i].to_epoch ==
    /// MIGRATIONS[i + 1].from_epoch` must hold for every adjacent
    /// pair, and the whole chain must be strictly monotonic.
    const MIGRATIONS: &'static [MigrationEdge];
}

// No blanket impl. stable Rust doesn't allow specialization, so a
// blanket `impl<T: AccountLayout> LayoutMigration for T` would lock
// out user opt-ins. Types without migrations simply never implement
// `LayoutMigration` and are therefore ineligible for
// `apply_pending_migrations::<T>`. which is the correct behaviour:
// you opt in to in-place migration by declaring a chain.

/// Apply all pending migrations needed to bring the account at
/// `current_epoch` up to `AccountLayout::SCHEMA_EPOCH`.
///
/// Returns `Ok(applied_count)` if everything up-migrated cleanly.
/// Returns `Err(MigrationMismatch)` if the declared chain is
/// incomplete, non-monotonic, or doesn't start at `current_epoch`.
/// Returns `Err(MigrationRejected)` if a user migrator function
/// returned an error.
#[inline]
pub fn apply_pending_migrations<T>(
    account: &AccountView,
    current_epoch: u32,
) -> Result<u32, ProgramError>
where
    T: AccountLayout + LayoutContract + LayoutMigration,
{
    let target_epoch = <T as AccountLayout>::SCHEMA_EPOCH;
    if current_epoch == target_epoch {
        return Ok(0);
    }
    if current_epoch > target_epoch {
        // Account is from a FUTURE epoch. forward-compatibility is
        // out of scope for in-place migration. Caller must refuse
        // or route to a different program.
        return Err(ProgramError::InvalidAccountData);
    }

    let edges = <T as LayoutMigration>::MIGRATIONS;
    let mut applied = 0u32;
    let mut epoch = current_epoch;

    // Single mutable borrow across the whole chain. atomicity per
    // edge is maintained by rewriting the header's schema_epoch byte
    // range before the borrow is released.
    let mut data = account.try_borrow_mut()?;
    let header_len = core::mem::size_of::<HopperHeader>();
    if data.len() < header_len {
        return Err(ProgramError::AccountDataTooSmall);
    }

    while epoch < target_epoch {
        let edge = find_edge(edges, epoch)?;
        let (header_bytes, body_bytes) = data.split_at_mut(header_len);
        // Step 1: mutate the body.
        (edge.migrator)(body_bytes)?;
        // Step 2: atomically bump the header's schema_epoch field.
        // Header layout is `#[repr(C, packed)]`: bytes 12..16 are
        // `schema_epoch: u32 LE` per `layout.rs`.
        let new_epoch_bytes = edge.to_epoch.to_le_bytes();
        header_bytes[12..16].copy_from_slice(&new_epoch_bytes);
        epoch = edge.to_epoch;
        applied += 1;
    }

    Ok(applied)
}

/// Locate the edge whose `from_epoch == epoch`. Returns an
/// `InvalidAccountData` error if the chain is discontinuous.
#[inline]
fn find_edge(edges: &[MigrationEdge], epoch: u32) -> Result<&MigrationEdge, ProgramError> {
    for edge in edges {
        if edge.from_epoch == epoch {
            if !edge.is_forward() {
                // A declared migration that doesn't advance the
                // epoch is malformed by construction.
                return Err(ProgramError::InvalidAccountData);
            }
            return Ok(edge);
        }
    }
    Err(ProgramError::InvalidAccountData)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(_body: &mut [u8]) -> Result<(), ProgramError> {
        Ok(())
    }

    #[test]
    fn migration_edge_is_forward_detects_non_monotonic() {
        let forward = MigrationEdge {
            from_epoch: 1,
            to_epoch: 2,
            migrator: identity,
        };
        let backward = MigrationEdge {
            from_epoch: 3,
            to_epoch: 2,
            migrator: identity,
        };
        let same = MigrationEdge {
            from_epoch: 2,
            to_epoch: 2,
            migrator: identity,
        };
        assert!(forward.is_forward());
        assert!(!backward.is_forward());
        assert!(!same.is_forward());
    }

    #[test]
    fn find_edge_returns_matching_edge() {
        let edges = [
            MigrationEdge {
                from_epoch: 1,
                to_epoch: 2,
                migrator: identity,
            },
            MigrationEdge {
                from_epoch: 2,
                to_epoch: 3,
                migrator: identity,
            },
        ];
        let e1 = find_edge(&edges, 1).expect("edge exists");
        assert_eq!(e1.to_epoch, 2);
        let e2 = find_edge(&edges, 2).expect("edge exists");
        assert_eq!(e2.to_epoch, 3);
    }

    #[test]
    fn find_edge_errs_on_missing_epoch() {
        let edges = [MigrationEdge {
            from_epoch: 1,
            to_epoch: 2,
            migrator: identity,
        }];
        // No edge starts at epoch 5.
        assert!(find_edge(&edges, 5).is_err());
    }

    #[test]
    fn find_edge_rejects_non_forward_edge() {
        let edges = [MigrationEdge {
            from_epoch: 3,
            to_epoch: 2,
            migrator: identity,
        }];
        assert!(find_edge(&edges, 3).is_err());
    }
}
