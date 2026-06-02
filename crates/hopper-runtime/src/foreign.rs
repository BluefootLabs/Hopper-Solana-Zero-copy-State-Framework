//! Manifest-backed foreign-account lenses.
//!
//! The Hopper Safety Audit (page 14, "Manifest-backed foreign account
//! lenses") proposed a verifiable cross-program read API as the next
//! step beyond ad-hoc offset-based foreign reads. This module
//! implements it.
//!
//! # Problem
//!
//! Today, reading a field from an account owned by a *different* program
//! either imports the foreign program's crate (tight coupling, forces
//! version-lock) or reads raw bytes by hand-maintained offset
//! (no ABI-drift detection. if the foreign program changes its layout,
//! silent misreads result).
//!
//! # Design
//!
//! A `ForeignManifest` is an opaque witness (supplied by the caller)
//! that carries the foreign program's `wire_fp64` hash plus the layout
//! discriminator it expects for a particular `T: AccountLayout`. When
//! `ctx.foreign::<T>(idx, &manifest)?` is called:
//!
//! 1. The account's owner must match `manifest.program_id`
//! 2. The account's header discriminator must match `T::DISC` and
//!    `manifest.expected_disc`
//! 3. The header's `wire_fp64` must match `T::WIRE_FINGERPRINT` and
//!    `manifest.expected_wire_fp`
//! 4. `schema_epoch` must fall in `manifest.supported_epochs`
//!
//! Only after all four pass does the lens expose field access. Any
//! mismatch returns `ProgramError::InvalidAccountData`. never silent
//! mis-reads, never UB.
//!
//! # Manifest sourcing
//!
//! Hopper does not fetch manifests from RPC inside a program (that
//! would be round-trip CPI with no caching story). Manifests are
//! caller-supplied, typically from:
//!
//! - An embedded `const ForeignManifest` authored when the program was
//!   built (works when the foreign program's ABI is known at build time)
//! - A manifest account located at the canonical manifest PDA
//!   (`find_program_address(&[MANIFEST_SEED], &foreign_program_id)`)
//!   whose payload has already been verified by a prior instruction
//! - A Hopper-authored IDL that emits manifest constants as part of
//!   its client-generation output

use crate::account::AccountView;
use crate::address::Address;
use crate::borrow::Ref;
use crate::error::ProgramError;
use crate::layout::{HopperHeader, LayoutContract};
use crate::ProgramResult;
use crate::zerocopy::{AccountLayout, ZeroCopy};
use core::marker::PhantomData;

/// Validation contract for known non-Hopper account layouts.
///
/// Hopper-owned layouts use Hopper headers and [`Account`](crate::Account).
/// Known foreign accounts usually do not. Implement this trait on a marker or
/// adapter type to validate owner, fixed byte prefix/discriminator, minimum
/// length, version bytes, oracle freshness gates, or any other external
/// invariant before binding an [`ExternalAccount`].
pub trait ExternalZeroCopy {
    /// Single expected owner program, when the adapter has one.
    const OWNER: Option<Address> = None;
    /// Optional byte prefix/discriminator at offset 0.
    const DISCRIMINATOR: Option<&'static [u8]> = None;
    /// Minimum account data length accepted by this adapter.
    const MIN_LEN: usize = 0;

    /// Validate this external account. Override for multi-owner layouts,
    /// versioned dispatch, or custom invariants.
    #[inline]
    fn validate(view: &AccountView) -> ProgramResult {
        if let Some(owner) = Self::OWNER {
            view.check_owned_by(&owner)?;
        }
        if view.data_len() < Self::MIN_LEN {
            return Err(ProgramError::AccountDataTooSmall);
        }
        if let Some(discriminator) = Self::DISCRIMINATOR {
            let data = view.try_borrow()?;
            if data.len() < discriminator.len() {
                return Err(ProgramError::AccountDataTooSmall);
            }
            if !data.starts_with(discriminator) {
                return Err(ProgramError::InvalidAccountData);
            }
        }
        Ok(())
    }
}

/// Validated handle to a known external account.
///
/// This wrapper is intentionally transparent over [`AccountView`]. It proves
/// the adapter's [`ExternalZeroCopy::validate`] contract, but it does not imply
/// a Hopper header is present. Use [`ExternalAccount::data`] or adapter helper
/// methods to read bytes, and keep raw `AccountView`/`UncheckedAccount` for
/// accounts that truly have no known schema.
#[repr(transparent)]
pub struct ExternalAccount<'info, T: ExternalZeroCopy> {
    inner: &'info AccountView,
    _ty: PhantomData<T>,
}

impl<'info, T: ExternalZeroCopy> Clone for ExternalAccount<'info, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'info, T: ExternalZeroCopy> Copy for ExternalAccount<'info, T> {}

impl<T: ExternalZeroCopy> core::fmt::Debug for ExternalAccount<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExternalAccount")
            .field("key", self.key())
            .field("owner", &self.owner())
            .field("data_len", &self.data_len())
            .finish()
    }
}

impl<'info, T: ExternalZeroCopy> ExternalAccount<'info, T> {
    /// Wrap an account that has already been validated by `T`.
    #[inline(always)]
    ///
    /// # Safety
    ///
    /// Caller must have verified `T::validate(view)` for this account.
    pub unsafe fn new_unchecked(view: &'info AccountView) -> Self {
        Self {
            inner: view,
            _ty: PhantomData,
        }
    }

    /// Validate and bind a known external account.
    #[inline]
    pub fn try_new(view: &'info AccountView) -> Result<Self, ProgramError> {
        T::validate(view)?;
        Ok(Self {
            inner: view,
            _ty: PhantomData,
        })
    }

    /// The underlying account view.
    #[inline(always)]
    pub fn as_account(&self) -> &'info AccountView {
        self.inner
    }

    /// The account public key.
    #[inline(always)]
    pub fn key(&self) -> &Address {
        self.inner.address()
    }

    /// The owning program, copied out of the account header.
    #[inline(always)]
    pub fn owner(&self) -> Address {
        self.inner.read_owner()
    }

    /// Current external account data length.
    #[inline(always)]
    pub fn data_len(&self) -> usize {
        self.inner.data_len()
    }

    /// Borrow the external account bytes after adapter validation.
    #[inline(always)]
    pub fn data(&self) -> Result<Ref<'_, [u8]>, ProgramError> {
        self.inner.try_borrow()
    }

    /// Borrow the bytes for the duration of a closure.
    #[inline]
    pub fn with_data<R, F>(&self, f: F) -> Result<R, ProgramError>
    where
        F: FnOnce(&[u8]) -> Result<R, ProgramError>,
    {
        let data = self.data()?;
        f(&data)
    }
}

impl<'info, T: ExternalZeroCopy> core::ops::Deref for ExternalAccount<'info, T> {
    type Target = AccountView;

    #[inline(always)]
    fn deref(&self) -> &AccountView {
        self.inner
    }
}

/// Opaque witness to a foreign program's layout ABI.
///
/// Callers construct this once per foreign program they want to read
/// from, typically as a `const` from build-time-embedded metadata or
/// from the foreign program's Hopper manifest account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignManifest {
    /// Owner program that authored the layout. The account's owner
    /// must match this address exactly.
    pub program_id: Address,
    /// Discriminator byte the foreign layout expects.
    pub expected_disc: u8,
    /// Canonical wire-fingerprint hash from the foreign program's
    /// schema manifest. Matches `AccountLayout::WIRE_FINGERPRINT` on
    /// the reader side.
    pub expected_wire_fp: u64,
    /// Inclusive range of `schema_epoch` values the reader supports.
    /// Accounts outside this range fail verification. the caller can
    /// then fall back to a migration path or a different manifest.
    pub supported_epochs: core::ops::RangeInclusive<u32>,
}

impl ForeignManifest {
    /// Build a single-epoch manifest covering `expected_wire_fp` for
    /// `program_id` at exactly the given schema epoch.
    pub const fn single_epoch(
        program_id: Address,
        expected_disc: u8,
        expected_wire_fp: u64,
        epoch: u32,
    ) -> Self {
        Self {
            program_id,
            expected_disc,
            expected_wire_fp,
            supported_epochs: epoch..=epoch,
        }
    }
}

/// A verified read-only handle into a foreign account.
///
/// `ForeignLens<'a, T>` borrows the underlying account data for its
/// lifetime. Field access (`.get()`, `.field::<F, OFFSET>()`) performs
/// only pointer arithmetic. no further verification, because all
/// cross-program invariants were pinned at construction.
pub struct ForeignLens<'a, T: AccountLayout + LayoutContract> {
    inner: Ref<'a, T>,
}

impl<'a, T: AccountLayout + LayoutContract> ForeignLens<'a, T> {
    /// Verify a foreign account against the supplied manifest and, on
    /// success, return a read-only lens into its body.
    ///
    /// The four verification steps correspond one-to-one with the
    /// audit's page-14 requirements:
    ///
    /// 1. owner match
    /// 2. discriminator match (both `T::DISC` *and* `manifest.expected_disc`)
    /// 3. wire-fingerprint match
    /// 4. schema_epoch in supported range
    #[inline]
    pub fn open(
        account: &'a AccountView,
        manifest: &ForeignManifest,
    ) -> Result<Self, ProgramError> {
        // 1. Owner match. `check_owned_by` compares address bytes.
        account.check_owned_by(&manifest.program_id)?;

        // 2-4. Header inspection. must happen behind a byte borrow
        //     so the data can't mutate underneath us. We use the same
        //     load path authored accounts use, which verifies the
        //     discriminator too. That closes #2.
        let loaded: Ref<'a, T> = account.load::<T>()?;
        if <T as AccountLayout>::DISC != manifest.expected_disc {
            return Err(ProgramError::InvalidAccountData);
        }

        // Re-read the header bytes directly so we can match the
        // manifest's wire-fingerprint and epoch fields. The load
        // above already verified disc/version, so this step only
        // checks the manifest-specific fields. HopperHeader is
        // `#[repr(C, packed)]` at 16 bytes. `from_bytes` returns a
        // properly bounds-checked reference without touching unaligned
        // primitives (we copy packed fields out by value below).
        let data = account.try_borrow()?;
        let header = HopperHeader::from_bytes(&data).ok_or(ProgramError::AccountDataTooSmall)?;
        // Packed-field reads must go through a local copy.
        let layout_id = header.layout_id;
        let schema_epoch = header.schema_epoch;
        let actual_wire_fp = u64::from_le_bytes(layout_id);
        if actual_wire_fp != manifest.expected_wire_fp {
            return Err(ProgramError::InvalidAccountData);
        }
        if actual_wire_fp != <T as AccountLayout>::WIRE_FINGERPRINT {
            return Err(ProgramError::InvalidAccountData);
        }
        if !manifest.supported_epochs.contains(&schema_epoch) {
            return Err(ProgramError::InvalidAccountData);
        }

        // Explicit drop so the re-borrow guard releases before we
        // hand out `loaded`, which already pins its own guard.
        drop(data);

        Ok(Self { inner: loaded })
    }

    /// The full verified layout. Field access through this path is
    /// zero-cost; no further checks fire.
    #[inline(always)]
    pub fn get(&self) -> &T {
        &self.inner
    }

    /// Project a typed field by byte offset. Returns a pointer-cast
    /// reference with the lens's lifetime.
    ///
    /// `OFFSET` must be the field's offset *within the layout body*
    /// (i.e. already past the 16-byte Hopper header). Callers should
    /// prefer the auto-emitted `{FIELD}_OFFSET` constants from
    /// `#[hopper::state]`.
    #[inline(always)]
    pub fn field<F: ZeroCopy, const OFFSET: usize>(&self) -> Result<&F, ProgramError> {
        let body_size = core::mem::size_of::<T>();
        let field_size = core::mem::size_of::<F>();
        if OFFSET
            .checked_add(field_size)
            .map(|end| end > body_size)
            .unwrap_or(true)
        {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: We checked the byte range lies entirely inside the
        // body. The layout is `Pod` (from `T: AccountLayout: ZeroCopy`),
        // so every byte pattern is valid for `F: ZeroCopy`. The
        // returned reference inherits the lens's lifetime and thus
        // cannot outlive the underlying borrow guard.
        // `Ref<T>` derefs to `T`; take the address via `&*`.
        let layout_ref: &T = &*self.inner;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe {
            let base = layout_ref as *const T as *const u8;
            let field_ptr = base.add(OFFSET) as *const F;
            Ok(&*field_ptr)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "hopper-native-backend")]
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    #[cfg(feature = "hopper-native-backend")]
    const EXTERNAL_OWNER: Address = Address::new_from_array([7; 32]);

    #[cfg(feature = "hopper-native-backend")]
    struct SampleExternal;

    #[cfg(feature = "hopper-native-backend")]
    impl ExternalZeroCopy for SampleExternal {
        const OWNER: Option<Address> = Some(EXTERNAL_OWNER);
        const DISCRIMINATOR: Option<&'static [u8]> = Some(b"PX");
        const MIN_LEN: usize = 4;
    }

    #[cfg(feature = "hopper-native-backend")]
    fn make_external_account(owner: Address, data: &[u8]) -> (std::vec::Vec<u8>, AccountView) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data.len()];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 0,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([3; 32]),
                owner: NativeAddress::new_from_array(owner.to_bytes()),
                lamports: 1,
                data_len: data.len() as u64,
            });
            let data_ptr = backing.as_mut_ptr().add(RuntimeAccount::SIZE);
            core::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, data.len());
        }
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    #[test]
    fn manifest_single_epoch_is_inclusive_single_value() {
        let program = Address::new_from_array([7u8; 32]);
        let m = ForeignManifest::single_epoch(program, 42, 0xDEAD_BEEF_1234_5678, 3);
        assert!(m.supported_epochs.contains(&3));
        assert!(!m.supported_epochs.contains(&2));
        assert!(!m.supported_epochs.contains(&4));
        assert_eq!(m.expected_disc, 42);
        assert_eq!(m.expected_wire_fp, 0xDEAD_BEEF_1234_5678);
    }

    #[test]
    fn manifest_range_spans_inclusive() {
        let program = Address::new_from_array([0u8; 32]);
        let m = ForeignManifest {
            program_id: program,
            expected_disc: 1,
            expected_wire_fp: 0,
            supported_epochs: 2..=5,
        };
        for ok in [2u32, 3, 4, 5] {
            assert!(m.supported_epochs.contains(&ok), "{ok}");
        }
        for fail in [0u32, 1, 6, 100] {
            assert!(!m.supported_epochs.contains(&fail), "{fail}");
        }
    }

    #[cfg(feature = "hopper-native-backend")]
    #[test]
    fn external_account_validates_owner_discriminator_and_length() {
        let (_backing, account) = make_external_account(EXTERNAL_OWNER, b"PX12");
        let external = ExternalAccount::<SampleExternal>::try_new(&account).unwrap();
        assert_eq!(external.owner(), EXTERNAL_OWNER);
        assert_eq!(external.data_len(), 4);
        external
            .with_data(|data| {
                assert_eq!(data, b"PX12");
                Ok(())
            })
            .unwrap();
    }

    #[cfg(feature = "hopper-native-backend")]
    #[test]
    fn external_account_rejects_wrong_owner_or_prefix() {
        let (_wrong_owner_backing, wrong_owner) =
            make_external_account(Address::new_from_array([8; 32]), b"PX12");
        assert_eq!(
            ExternalAccount::<SampleExternal>::try_new(&wrong_owner).unwrap_err(),
            ProgramError::IncorrectProgramId
        );

        let (_wrong_prefix_backing, wrong_prefix) = make_external_account(EXTERNAL_OWNER, b"NO12");
        assert_eq!(
            ExternalAccount::<SampleExternal>::try_new(&wrong_prefix).unwrap_err(),
            ProgramError::InvalidAccountData
        );

        let (_short_backing, short) = make_external_account(EXTERNAL_OWNER, b"PX");
        assert_eq!(
            ExternalAccount::<SampleExternal>::try_new(&short).unwrap_err(),
            ProgramError::AccountDataTooSmall
        );
    }
}
