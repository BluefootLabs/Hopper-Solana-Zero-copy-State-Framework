//! Segmented account wrapper with role-aware access.
//!
//! Wraps an account that uses Hopper's segment registry to divide data
//! into typed regions (Core, Extension, Journal, Cache, etc.).

use hopper_runtime::{AccountView, Address, Ref, RefMut};
use hopper_runtime::error::ProgramError;

use crate::account::{
    Pod, FixedLayout, SegmentEntry, SegmentId, SegmentRegistry,
    REGISTRY_HEADER_SIZE, REGISTRY_OFFSET, SEGMENT_ENTRY_SIZE,
};
use crate::check;
use crate::check::modifier::HopperLayout;

/// Borrow-carrying registry view for segmented accounts.
pub struct BorrowedSegmentRegistry<'a> {
    data: Ref<'a, [u8]>,
}

impl<'a> BorrowedSegmentRegistry<'a> {
    #[inline]
    pub fn segment_count(&self) -> Result<usize, ProgramError> {
        Ok(SegmentRegistry::from_account(&self.data)?.segment_count())
    }

    #[inline]
    pub fn data_region_offset(&self) -> Result<usize, ProgramError> {
        Ok(SegmentRegistry::from_account(&self.data)?.data_region_offset())
    }

    #[inline]
    pub fn entry(&self, index: usize) -> Result<&SegmentEntry, ProgramError> {
        let registry = SegmentRegistry::from_account(&self.data)?;
        if index >= registry.segment_count() {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = REGISTRY_OFFSET + REGISTRY_HEADER_SIZE + index * SEGMENT_ENTRY_SIZE;
        Ok(unsafe { &*(self.data.as_bytes_ptr().add(offset) as *const SegmentEntry) })
    }

    #[inline]
    pub fn segment_data(&self, id: &SegmentId) -> Result<&[u8], ProgramError> {
        let count = self.segment_count()?;
        let mut index = 0;
        while index < count {
            let entry = self.entry(index)?;
            if entry.id == *id {
                let start = entry.offset() as usize;
                let end = start
                    .checked_add(entry.size() as usize)
                    .ok_or(ProgramError::ArithmeticOverflow)?;
                if end > self.data.len() {
                    return Err(ProgramError::AccountDataTooSmall);
                }
                return Ok(&self.data[start..end]);
            }
            index += 1;
        }
        Err(ProgramError::InvalidArgument)
    }
}

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
        let data = account.try_borrow()?;
        crate::account::check_header(&data, T::DISC, T::VERSION, &T::LAYOUT_ID)?;
        Ok(Self {
            view: account,
            program_id,
            _marker: core::marker::PhantomData,
        })
    }

    /// Access the segment registry.
    #[inline]
    pub fn registry(&self) -> Result<BorrowedSegmentRegistry<'_>, ProgramError> {
        Ok(BorrowedSegmentRegistry {
            data: self.view.try_borrow()?,
        })
    }

    /// Read a segment's raw bytes by index.
    #[inline]
    pub fn segment_by_index(&self, index: usize) -> Result<Ref<'_, [u8]>, ProgramError> {
        let data = self.view.try_borrow()?;
        let (start, size) = {
            let registry = SegmentRegistry::from_account(&data)?;
            let entry = registry.entry(index)?;
            (entry.offset() as usize, entry.size() as usize)
        };
        data.slice(start, size)
    }

    /// Read a segment's data by its 4-byte ID.
    #[inline]
    pub fn segment_data(&self, id: &crate::account::SegmentId) -> Result<Ref<'_, [u8]>, ProgramError> {
        let data = self.view.try_borrow()?;
        let (start, size) = {
            let registry = SegmentRegistry::from_account(&data)?;
            let (_, entry) = registry.find(id)?;
            (entry.offset() as usize, entry.size() as usize)
        };
        data.slice(start, size)
    }

    /// Read a segment's raw bytes mutably by index.
    #[inline]
    pub fn segment_by_index_mut(&self, index: usize) -> Result<RefMut<'_, [u8]>, ProgramError> {
        let data = self.view.try_borrow_mut()?;
        let (start, size) = {
            let registry = SegmentRegistry::from_account(&data)?;
            let entry = registry.entry(index)?;
            (entry.offset() as usize, entry.size() as usize)
        };
        data.slice(start, size)
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
