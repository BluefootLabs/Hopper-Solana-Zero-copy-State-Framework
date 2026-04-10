//! Segmented account wrapper with role-aware access.
//!
//! Wraps an account that uses Hopper's segment registry to divide data
//! into typed regions (Core, Extension, Journal, Cache, etc.).

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use crate::account::{Pod, FixedLayout, SegmentRegistry};
use crate::check;
use crate::check::modifier::HopperLayout;

/// A segmented account with role-aware region access.
///
/// The account data is divided into a segment registry header followed by
/// typed segments. Each segment has a role (Core, Extension, Journal, etc.)
/// and can be accessed individually.
pub struct SegmentedAccount<'a, T: Pod + FixedLayout + HopperLayout> {
    view: &'a AccountView,
    #[allow(dead_code)] // stored for future segment CPI operations
    program_id: &'a Address,
    _marker: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout + HopperLayout> SegmentedAccount<'a, T> {
    /// Construct from an AccountView with header and owner validation.
    #[inline]
    pub fn from_account(
        account: &'a AccountView,
        program_id: &'a Address,
    ) -> Result<Self, ProgramError> {
        check::check_owner(account, program_id)?;
        let data = unsafe { account.borrow_unchecked() };
        crate::account::check_header(data, T::DISC, T::VERSION, &T::LAYOUT_ID)?;
        Ok(Self {
            view: account,
            program_id,
            _marker: core::marker::PhantomData,
        })
    }

    /// Access the segment registry.
    #[inline]
    pub fn registry(&self) -> Result<SegmentRegistry<'_>, ProgramError> {
        let data = unsafe { self.view.borrow_unchecked() };
        SegmentRegistry::from_account(data)
    }

    /// Read a segment's raw bytes by index.
    #[inline]
    pub fn segment_by_index(&self, index: usize) -> Result<&[u8], ProgramError> {
        let data = unsafe { self.view.borrow_unchecked() };
        let registry = SegmentRegistry::from_account(data)?;
        let entry = registry.entry(index)?;
        let start = entry.offset() as usize;
        let end = start + entry.size() as usize;
        if end > data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(&data[start..end])
    }

    /// Read a segment's data by its 4-byte ID.
    #[inline]
    pub fn segment_data(&self, id: &crate::account::SegmentId) -> Result<&[u8], ProgramError> {
        let data = unsafe { self.view.borrow_unchecked() };
        let registry = SegmentRegistry::from_account(data)?;
        registry.segment_data(id)
    }

    /// Read a segment's raw bytes mutably by index.
    #[inline]
    pub fn segment_by_index_mut(&self, index: usize) -> Result<&mut [u8], ProgramError> {
        let data = unsafe { self.view.borrow_unchecked_mut() };
        let len = data.len();
        let registry = SegmentRegistry::from_account(data)?;
        let entry = registry.entry(index)?;
        let start = entry.offset() as usize;
        let end = start + entry.size() as usize;
        if end > len {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(&mut data[start..end])
    }

    /// The account's address.
    #[inline(always)]
    pub fn address(&self) -> &Address {
        self.view.address()
    }

    /// The underlying AccountView.
    #[inline(always)]
    pub fn to_account_view(&self) -> &'a AccountView {
        self.view
    }
}
