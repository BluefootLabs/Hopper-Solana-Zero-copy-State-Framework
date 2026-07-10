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
//! * **Atomic per edge — under transaction-abort semantics.** Each
//!   edge bumps the header's `schema_epoch` only after its body
//!   mutation fully succeeded, so a *completed* edge is always
//!   consistent. A migrator that errors after partially writing the
//!   body, however, leaves a hybrid body under the old epoch — the
//!   returned error **must** propagate to instruction failure (the
//!   Solana runtime then rolls every byte back). Callers must not
//!   swallow migration errors and continue using the account.
//! * **Idempotent**. re-running an already-applied edge is a no-op
//!   (the header epoch mismatch returns `MigrationMismatch`).
//! * **Deterministic**. edges are applied in strict
// ---------------------------------------------------------------------

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
    account: &AccountView<'_>,
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
        // A declared edge must not overshoot the layout's current
        // epoch: stamping the header past SCHEMA_EPOCH would mark the
        // account as from-the-future and make every subsequent typed
        // load refuse it — silent corruption from a misdeclared chain.
        // Refuse before touching a byte.
        if edge.to_epoch > target_epoch {
            return Err(ProgramError::InvalidAccountData);
        }
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

/// Typed, in-place, cross-VERSION layout migration: `Old` → `New`.
///
/// The epoch machinery above evolves an account *within* one layout
/// version through raw `&mut [u8]` edges. This is the other half of the
/// versioning story: the account's layout **version byte** changes
/// (`#[hopper::state(version = 1)]` → `version = 2`), the wire
/// fingerprint changes with the field set, and the transform is
/// **typed on both sides** — no hand-offsetting bytes:
///
/// ```ignore
/// hopper_runtime::migrate::migrate_layout::<VaultV1, VaultV2, _>(
///     account,
///     |old, new| {
///         new.authority = old.authority;
///         // Widen the counter; every other V2 field keeps its
///         // deterministic all-zero default.
///         new.total = WireU64::new(old.total_u32.get() as u64);
///         Ok(())
///     },
/// )?;
/// ```
///
/// Contrast with anchor-next's `Migration` account shape, which
/// deserializes the old form and RESERIALIZES the new one through
/// borsh. Here both shapes are zero-copy overlays of the same buffer:
/// one stack copy of `Old` (so the transform can still read it after
/// the buffer is re-purposed), one `fill(0)` of the `New` span, no
/// (de)serialization, no heap.
///
/// # Sequence
///
/// 1. `Old::validate_header` — the full identity check (disc, version,
///    layout_id, epoch). An already-migrated account no longer matches
///    `Old` and is refused, which is the idempotence rule: migrate
///    exactly once, route repeat calls to the `New` load path.
/// 2. The `New` shape must FIT the existing allocation
///    (`required_len`); resizing is `realloc`'s job, done separately
///    BEFORE migrating when `New` is larger.
/// 3. `Old` is copied to the stack, the `New` span is zeroed (so every
///    field the transform does not set has the framework's canonical
///    all-zero default — stale `Old` bytes never leak through), and the
///    typed transform fills `New` from the copy.
/// 4. Only after the transform returns `Ok` is the header re-stamped —
///    `New`'s disc/version/layout_id/schema-epoch, with the header's
///    FLAGS bytes preserved (flags are account state, not layout
///    identity). A transform error therefore leaves the header on
///    `Old` — same transaction-abort atomicity contract as the epoch
///    edges above: the error must propagate to instruction failure so
///    the runtime rolls the partially-written body back.
///
/// # Guard rails (all refuse before touching a byte)
///
/// * `New::DISC == Old::DISC` — a migration must not repurpose the
///   account kind; both consts are known at monomorphization, so the
///   check folds away when it passes.
/// * `New::VERSION > Old::VERSION` — versions only move forward
///   (also const-folded).
#[inline]
pub fn migrate_layout<Old, New, F>(
    account: &AccountView<'_>,
    transform: F,
) -> Result<(), ProgramError>
where
    Old: LayoutContract + crate::Pod,
    New: LayoutContract + crate::Pod,
    F: FnOnce(&Old, &mut New) -> Result<(), ProgramError>,
{
    // Const-foldable direction guards: same account kind, strictly
    // forward version. (Written as runtime `if`s so they work on every
    // toolchain; the comparisons are monomorphized constants and the
    // passing branch compiles to nothing.)
    if New::DISC != Old::DISC {
        return Err(ProgramError::InvalidAccountData);
    }
    if New::VERSION <= Old::VERSION {
        return Err(ProgramError::InvalidAccountData);
    }

    let mut data = account.try_borrow_mut()?;
    Old::validate_header(&data)?;
    if data.len() < New::required_len() {
        // In-place only: a larger New needs `realloc` FIRST. Refusing
        // here (before any write) keeps the account a valid Old.
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Stack-copy the old shape so the transform can read it after the
    // buffer below is re-purposed as `New`.
    // SAFETY: `validate_header` proved the buffer holds a valid `Old`
    // at TYPE_OFFSET with at least `required_len()` bytes; `Old: Pod`
    // makes any bit pattern valid, and `read_unaligned` lifts the
    // bytes without an alignment requirement.
    let old: Old =
        unsafe { core::ptr::read_unaligned((*data).as_ptr().add(Old::TYPE_OFFSET) as *const Old) };

    // Deterministic defaults: zero the New span so unset fields carry
    // the framework's canonical empty value rather than stale bytes.
    let new_start = New::TYPE_OFFSET;
    let new_end = new_start + core::mem::size_of::<New>();
    data[new_start..new_end].fill(0);

    // SAFETY: the length check above proved `new_end <= data.len()`;
    // `New: Pod` accepts the all-zero pattern just written; Hopper
    // layout types are wire-aligned (align 1 by construction, same
    // contract `load_mut` relies on when it projects at TYPE_OFFSET).
    let new: &mut New = unsafe { &mut *(data.as_bytes_mut_ptr().add(new_start) as *mut New) };
    transform(&old, new)?;

    // Success: re-stamp the header as New — LAST, so an erroring
    // transform leaves the header on Old (see atomicity note above).
    // Flags are preserved: they describe the account, not the layout.
    let flags = crate::layout::read_flags(&data).unwrap_or(0);
    crate::layout::write_header_with_epoch(
        &mut data,
        New::DISC,
        New::VERSION,
        &New::LAYOUT_ID,
        New::SCHEMA_EPOCH,
    )?;
    data[2..4].copy_from_slice(&flags.to_le_bytes());
    Ok(())
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

    mod overshoot {
        use super::*;
        use crate::layout::{HopperHeader, LayoutContract};
        use crate::zerocopy::AccountLayout;
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct EpochTwo {
            v: [u8; 8],
        }
        // SAFETY: repr(C), byte-array field — every bit pattern valid,
        // align 1, no padding.
        unsafe impl crate::Zeroable for EpochTwo {}
        // SAFETY: as above.
        unsafe impl crate::Pod for EpochTwo {}
        // SAFETY: test-local layout upholding the sealed overlay contract.
        unsafe impl crate::zerocopy::__sealed::HopperZeroCopySealed for EpochTwo {}
        impl crate::field_map::FieldMap for EpochTwo {
            const FIELDS: &'static [crate::field_map::FieldInfo] =
                &[crate::field_map::FieldInfo::new("v", HopperHeader::SIZE, 8)];
        }
        impl LayoutContract for EpochTwo {
            const DISC: u8 = 91;
            const VERSION: u8 = 1;
            const LAYOUT_ID: [u8; 8] = [0x91; 8];
            const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
            const SCHEMA_EPOCH: u32 = 2;
        }
        impl LayoutMigration for EpochTwo {
            // Misdeclared chain: jumps 1 → 3 while SCHEMA_EPOCH is 2.
            const MIGRATIONS: &'static [MigrationEdge] = &[MigrationEdge {
                from_epoch: 1,
                to_epoch: 3,
                migrator: identity,
            }];
        }

        #[test]
        fn overshooting_edge_is_refused_before_writing() {
            let mut backing = std::vec![0u8; RuntimeAccount::SIZE + HopperHeader::SIZE + 8];
            let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
            // SAFETY: backing is sized for the header plus data and
            // outlives the view.
            unsafe {
                raw.write(RuntimeAccount {
                    borrow_state: NOT_BORROWED,
                    is_signer: 0,
                    is_writable: 1,
                    executable: 0,
                    resize_delta: 0,
                    address: NativeAddress::new_from_array([5; 32]),
                    owner: NativeAddress::new_from_array([6; 32]),
                    lamports: 1,
                    data_len: (HopperHeader::SIZE + 8) as u64,
                });
            }
            // SAFETY: raw points at a fully initialized RuntimeAccount.
            let backend = unsafe { NativeAccountView::new_unchecked(raw) };
            let account = crate::AccountView::from_backend(backend);

            // The 1→3 edge would stamp the header past SCHEMA_EPOCH=2:
            // must refuse instead of silently marking the account as
            // from the future.
            assert_eq!(
                apply_pending_migrations::<EpochTwo>(&account, 1),
                Err(ProgramError::InvalidAccountData)
            );
            let _ = <EpochTwo as AccountLayout>::SCHEMA_EPOCH;
        }
    }

    mod typed_layout_migration {
        use super::*;
        use crate::layout::{
            read_disc, read_flags, read_layout_id, read_schema_epoch, read_version, write_header,
            HopperHeader,
        };
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        const KIND: u8 = 77;

        /// Version 1: a narrow counter plus a legacy blob.
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct VaultV1 {
            count: [u8; 4],
            legacy: [u8; 4],
        }
        // SAFETY: repr(C), byte-array fields — every bit pattern valid,
        // align 1, no padding.
        unsafe impl crate::Zeroable for VaultV1 {}
        // SAFETY: as above.
        unsafe impl crate::Pod for VaultV1 {}
        // SAFETY: test-local layout upholding the sealed overlay contract.
        unsafe impl crate::zerocopy::__sealed::HopperZeroCopySealed for VaultV1 {}
        impl crate::field_map::FieldMap for VaultV1 {
            const FIELDS: &'static [crate::field_map::FieldInfo] = &[
                crate::field_map::FieldInfo::new("count", HopperHeader::SIZE, 4),
                crate::field_map::FieldInfo::new("legacy", HopperHeader::SIZE + 4, 4),
            ];
        }
        impl LayoutContract for VaultV1 {
            const DISC: u8 = KIND;
            const VERSION: u8 = 1;
            const LAYOUT_ID: [u8; 8] = [0x11; 8];
            const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
        }

        /// Version 2: the counter widens to u64, a flag appears, the
        /// legacy blob is gone. 12-byte body > V1's 8.
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct VaultV2 {
            count: [u8; 8],
            flag: u8,
            pad: [u8; 3],
        }
        // SAFETY: repr(C), byte/byte-array fields — every bit pattern
        // valid, align 1, no padding.
        unsafe impl crate::Zeroable for VaultV2 {}
        // SAFETY: as above.
        unsafe impl crate::Pod for VaultV2 {}
        // SAFETY: test-local layout upholding the sealed overlay contract.
        unsafe impl crate::zerocopy::__sealed::HopperZeroCopySealed for VaultV2 {}
        impl crate::field_map::FieldMap for VaultV2 {
            const FIELDS: &'static [crate::field_map::FieldInfo] = &[
                crate::field_map::FieldInfo::new("count", HopperHeader::SIZE, 8),
                crate::field_map::FieldInfo::new("flag", HopperHeader::SIZE + 8, 1),
                crate::field_map::FieldInfo::new("pad", HopperHeader::SIZE + 9, 3),
            ];
        }
        impl LayoutContract for VaultV2 {
            const DISC: u8 = KIND;
            const VERSION: u8 = 2;
            const LAYOUT_ID: [u8; 8] = [0x22; 8];
            const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
            // A non-default target epoch, to prove the stamp writes the
            // NEW layout's epoch rather than inheriting the old one.
            const SCHEMA_EPOCH: u32 = 5;
        }

        /// A different account kind entirely (wrong disc).
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct OtherKind {
            v: [u8; 8],
        }
        // SAFETY: repr(C), byte-array field — every bit pattern valid,
        // align 1, no padding.
        unsafe impl crate::Zeroable for OtherKind {}
        // SAFETY: as above.
        unsafe impl crate::Pod for OtherKind {}
        // SAFETY: test-local layout upholding the sealed overlay contract.
        unsafe impl crate::zerocopy::__sealed::HopperZeroCopySealed for OtherKind {}
        impl crate::field_map::FieldMap for OtherKind {
            const FIELDS: &'static [crate::field_map::FieldInfo] =
                &[crate::field_map::FieldInfo::new("v", HopperHeader::SIZE, 8)];
        }
        impl LayoutContract for OtherKind {
            const DISC: u8 = KIND + 1;
            const VERSION: u8 = 3;
            const LAYOUT_ID: [u8; 8] = [0x33; 8];
            const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
        }

        /// Same kind, same version as V1 (non-forward target).
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct VaultV1b {
            count: [u8; 8],
        }
        // SAFETY: repr(C), byte-array field — every bit pattern valid,
        // align 1, no padding.
        unsafe impl crate::Zeroable for VaultV1b {}
        // SAFETY: as above.
        unsafe impl crate::Pod for VaultV1b {}
        // SAFETY: test-local layout upholding the sealed overlay contract.
        unsafe impl crate::zerocopy::__sealed::HopperZeroCopySealed for VaultV1b {}
        impl crate::field_map::FieldMap for VaultV1b {
            const FIELDS: &'static [crate::field_map::FieldInfo] =
                &[crate::field_map::FieldInfo::new(
                    "count",
                    HopperHeader::SIZE,
                    8,
                )];
        }
        impl LayoutContract for VaultV1b {
            const DISC: u8 = KIND;
            const VERSION: u8 = 1;
            const LAYOUT_ID: [u8; 8] = [0x44; 8];
            const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
        }

        /// Build a writable account of `data_len` bytes seeded as a
        /// valid VaultV1 (count = 7, legacy = [1,2,3,4], flags =
        /// 0x0102 to prove flag preservation).
        fn seeded_v1(data_len: usize) -> (std::vec::Vec<u8>, crate::AccountView<'static>) {
            let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data_len];
            let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
            // SAFETY: backing is sized for the runtime header plus
            // `data_len` bytes and outlives the returned view (the
            // caller holds the Vec).
            unsafe {
                raw.write(RuntimeAccount {
                    borrow_state: NOT_BORROWED,
                    is_signer: 0,
                    is_writable: 1,
                    executable: 0,
                    resize_delta: 0,
                    address: NativeAddress::new_from_array([5; 32]),
                    owner: NativeAddress::new_from_array([6; 32]),
                    lamports: 1,
                    data_len: data_len as u64,
                });
            }
            // SAFETY: raw points at a fully initialized RuntimeAccount
            // with its data region in the same allocation.
            let backend = unsafe { NativeAccountView::new_unchecked(raw) };
            let account = crate::AccountView::from_backend(backend);
            {
                let mut data = account.try_borrow_mut().expect("fixture borrow");
                write_header(
                    &mut data,
                    <VaultV1 as LayoutContract>::DISC,
                    <VaultV1 as LayoutContract>::VERSION,
                    &<VaultV1 as LayoutContract>::LAYOUT_ID,
                )
                .expect("fixture header");
                // Account-state flags a migration must carry across.
                data[2..4].copy_from_slice(&0x0102u16.to_le_bytes());
                data[16..20].copy_from_slice(&7u32.to_le_bytes());
                data[20..24].copy_from_slice(&[1, 2, 3, 4]);
            }
            (backing, account)
        }

        fn widen(old: &VaultV1, new: &mut VaultV2) -> Result<(), ProgramError> {
            let count = u32::from_le_bytes(old.count) as u64;
            new.count = count.to_le_bytes();
            new.flag = 1;
            Ok(())
        }

        #[test]
        fn typed_migration_restamps_header_and_transforms_body() {
            // Allocation already big enough for V2 (the realloc-first
            // rule for larger shapes is exercised separately below).
            let (_b, account) = seeded_v1(HopperHeader::SIZE + 16);

            migrate_layout::<VaultV1, VaultV2, _>(&account, widen).expect("migrates");

            let data = account.try_borrow().expect("read back");
            // Header: New identity, OLD flags.
            assert_eq!(read_disc(&data), Some(KIND));
            assert_eq!(read_version(&data), Some(2));
            assert_eq!(read_layout_id(&data), Some(&[0x22; 8]));
            assert_eq!(read_schema_epoch(&data), Some(5));
            assert_eq!(
                read_flags(&data),
                Some(0x0102),
                "flags are account state and must survive the re-stamp"
            );
            // Body: widened counter, set flag, and NOTHING left of the
            // legacy bytes (the zeroed span is the deterministic
            // default for unset fields).
            assert_eq!(&data[16..24], &7u64.to_le_bytes());
            assert_eq!(data[24], 1);
            assert_eq!(&data[25..28], &[0, 0, 0]);
        }

        #[test]
        fn migrated_account_refuses_a_second_migration() {
            let (_b, account) = seeded_v1(HopperHeader::SIZE + 16);
            migrate_layout::<VaultV1, VaultV2, _>(&account, widen).expect("first migrates");
            // The header now reads V2: it is no longer a valid VaultV1,
            // so the identity check refuses — migrate exactly once.
            assert_eq!(
                migrate_layout::<VaultV1, VaultV2, _>(&account, widen),
                Err(ProgramError::InvalidAccountData)
            );
        }

        #[test]
        fn transform_error_leaves_the_header_on_the_old_layout() {
            let (_b, account) = seeded_v1(HopperHeader::SIZE + 16);
            let result = migrate_layout::<VaultV1, VaultV2, _>(&account, |_, _| {
                Err(ProgramError::Custom(9))
            });
            assert_eq!(result, Err(ProgramError::Custom(9)));
            let data = account.try_borrow().expect("read back");
            // The stamp is LAST: the header still says V1, so under
            // transaction-abort semantics the account is never observed
            // half-migrated (the body writes roll back with the tx).
            assert_eq!(read_version(&data), Some(1));
            assert_eq!(read_layout_id(&data), Some(&[0x11; 8]));
        }

        #[test]
        fn larger_new_shape_requires_realloc_first() {
            // Allocation fits V1 exactly (24 bytes); V2 needs 28.
            let (_b, account) = seeded_v1(HopperHeader::SIZE + 8);
            assert_eq!(
                migrate_layout::<VaultV1, VaultV2, _>(&account, widen),
                Err(ProgramError::AccountDataTooSmall)
            );
            let data = account.try_borrow().expect("read back");
            assert_eq!(read_version(&data), Some(1), "refused before any write");
            assert_eq!(&data[16..20], &7u32.to_le_bytes());
        }

        #[test]
        fn cross_kind_migration_is_refused() {
            let (_b, account) = seeded_v1(HopperHeader::SIZE + 16);
            // OtherKind::DISC != VaultV1::DISC: repurposing the account
            // kind is not a migration.
            assert_eq!(
                migrate_layout::<VaultV1, OtherKind, _>(&account, |_, _| Ok(())),
                Err(ProgramError::InvalidAccountData)
            );
        }

        #[test]
        fn non_forward_version_is_refused() {
            let (_b, account) = seeded_v1(HopperHeader::SIZE + 16);
            // Same version (1 → 1): not forward, refused before any
            // borrow or write.
            assert_eq!(
                migrate_layout::<VaultV1, VaultV1b, _>(&account, |_, _| Ok(())),
                Err(ProgramError::InvalidAccountData)
            );
            // And backward (2 → 1) likewise.
            assert_eq!(
                migrate_layout::<VaultV2, VaultV1b, _>(&account, |_, _| Ok(())),
                Err(ProgramError::InvalidAccountData)
            );
        }
    }
}
