//! Fixed-capacity alias registry for Hopper account borrows.
//!
//! Backends already enforce borrow rules for one canonical account handle.
//! Hopper layers an address-keyed registry on top so duplicate handles that
//! resolve to the same account identity cannot silently mix shared and mutable
//! access across the runtime API.

use crate::address::Address;
use crate::error::ProgramError;
pub(crate) use imp::{check_mutable, check_shared, BorrowToken};

#[cfg(target_os = "solana")]
mod imp {
    use super::{Address, ProgramError};

    #[derive(Debug)]
    pub(crate) struct BorrowToken;

    impl BorrowToken {
        #[inline(always)]
        pub(crate) fn shared(_address: &Address) -> Result<Self, ProgramError> {
            Ok(Self)
        }

        #[inline(always)]
        pub(crate) fn mutable(_address: &Address) -> Result<Self, ProgramError> {
            Ok(Self)
        }
    }

    #[inline(always)]
    pub(crate) fn check_shared(_address: &Address) -> Result<(), ProgramError> {
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn check_mutable(_address: &Address) -> Result<(), ProgramError> {
        Ok(())
    }
}

#[cfg(not(target_os = "solana"))]
mod imp {
    use super::{Address, ProgramError};
    use crate::MAX_TX_ACCOUNTS;

    #[cfg(test)]
    use std::cell::RefCell;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum BorrowAccess {
        Shared,
        Mutable,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct BorrowState {
        pub address: Address,
        pub shared_count: u16,
        pub mutable: bool,
    }

    impl BorrowState {
        pub const EMPTY: Self = Self {
            address: Address::new([0; 32]),
            shared_count: 0,
            mutable: false,
        };

        #[inline(always)]
        const fn is_empty(&self) -> bool {
            !self.mutable && self.shared_count == 0
        }
    }

    #[derive(Debug)]
    pub(crate) struct BorrowToken {
        address: Address,
        access: BorrowAccess,
    }

    impl BorrowToken {
        #[inline]
        pub(crate) fn shared(address: &Address) -> Result<Self, ProgramError> {
            with_registry_mut(|registry| registry.register(*address, BorrowAccess::Shared))?;
            Ok(Self {
                address: *address,
                access: BorrowAccess::Shared,
            })
        }

        #[inline]
        pub(crate) fn mutable(address: &Address) -> Result<Self, ProgramError> {
            with_registry_mut(|registry| registry.register(*address, BorrowAccess::Mutable))?;
            Ok(Self {
                address: *address,
                access: BorrowAccess::Mutable,
            })
        }
    }

    impl Drop for BorrowToken {
        fn drop(&mut self) {
            with_registry_mut(|registry| registry.release(self.address, self.access));
        }
    }

    pub(crate) fn check_shared(address: &Address) -> Result<(), ProgramError> {
        with_registry(|registry| registry.can_register(*address, BorrowAccess::Shared))
    }

    pub(crate) fn check_mutable(address: &Address) -> Result<(), ProgramError> {
        with_registry(|registry| registry.can_register(*address, BorrowAccess::Mutable))
    }

    struct BorrowRegistry {
        entries: [BorrowState; MAX_TX_ACCOUNTS],
    }

    impl BorrowRegistry {
        const fn new() -> Self {
            Self {
                entries: [BorrowState::EMPTY; MAX_TX_ACCOUNTS],
            }
        }

        fn can_register(&self, address: Address, access: BorrowAccess) -> Result<(), ProgramError> {
            if let Some(index) = self.find(address) {
                let state = self.entries[index];
                return match access {
                    BorrowAccess::Shared if state.mutable => Err(ProgramError::AccountBorrowFailed),
                    BorrowAccess::Mutable if state.mutable || state.shared_count != 0 => {
                        Err(ProgramError::AccountBorrowFailed)
                    }
                    _ => Ok(()),
                };
            }

            if self.first_empty().is_some() {
                Ok(())
            } else {
                Err(ProgramError::AccountBorrowFailed)
            }
        }

        fn register(&mut self, address: Address, access: BorrowAccess) -> Result<(), ProgramError> {
            self.can_register(address, access)?;

            let index = match self.find(address) {
                Some(index) => index,
                None => self.first_empty().ok_or(ProgramError::AccountBorrowFailed)?,
            };

            let state = &mut self.entries[index];
            if state.is_empty() {
                state.address = address;
            }

            match access {
                BorrowAccess::Shared => {
                    state.shared_count = state
                        .shared_count
                        .checked_add(1)
                        .ok_or(ProgramError::AccountBorrowFailed)?;
                }
                BorrowAccess::Mutable => {
                    state.mutable = true;
                }
            }

            Ok(())
        }

        fn release(&mut self, address: Address, access: BorrowAccess) {
            let Some(index) = self.find(address) else {
                return;
            };

            let state = &mut self.entries[index];
            match access {
                BorrowAccess::Shared => {
                    if state.shared_count != 0 {
                        state.shared_count -= 1;
                    }
                }
                BorrowAccess::Mutable => {
                    state.mutable = false;
                }
            }

            if state.is_empty() {
                *state = BorrowState::EMPTY;
            }
        }

        #[inline(always)]
        fn find(&self, address: Address) -> Option<usize> {
            let mut index = 0;
            while index < self.entries.len() {
                let state = self.entries[index];
                if !state.is_empty() && state.address == address {
                    return Some(index);
                }
                index += 1;
            }
            None
        }

        #[inline(always)]
        fn first_empty(&self) -> Option<usize> {
            let mut index = 0;
            while index < self.entries.len() {
                if self.entries[index].is_empty() {
                    return Some(index);
                }
                index += 1;
            }
            None
        }
    }

    #[cfg(test)]
    std::thread_local! {
        static REGISTRY: RefCell<BorrowRegistry> = RefCell::new(BorrowRegistry::new());
    }

    #[cfg(not(test))]
    struct RegistryCell(core::cell::UnsafeCell<BorrowRegistry>);

    #[cfg(not(test))]
    unsafe impl Sync for RegistryCell {}

    #[cfg(not(test))]
    static REGISTRY: RegistryCell = RegistryCell(core::cell::UnsafeCell::new(BorrowRegistry::new()));

    #[cfg(test)]
    fn with_registry<R>(f: impl FnOnce(&BorrowRegistry) -> R) -> R {
        REGISTRY.with(|registry| {
            let registry = registry.borrow();
            f(&registry)
        })
    }

    #[cfg(not(test))]
    fn with_registry<R>(f: impl FnOnce(&BorrowRegistry) -> R) -> R {
        unsafe { f(&*REGISTRY.0.get()) }
    }

    #[cfg(test)]
    fn with_registry_mut<R>(f: impl FnOnce(&mut BorrowRegistry) -> R) -> R {
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            f(&mut registry)
        })
    }

    #[cfg(not(test))]
    fn with_registry_mut<R>(f: impl FnOnce(&mut BorrowRegistry) -> R) -> R {
        unsafe { f(&mut *REGISTRY.0.get()) }
    }
}