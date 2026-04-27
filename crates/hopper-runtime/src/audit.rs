use crate::error::ProgramError;
use crate::{AccountView, Address, ProgramResult};

/// Duplicate-account details discovered during instruction-scope auditing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuplicateAccount {
    pub first_index: usize,
    pub second_index: usize,
    pub address: Address,
}

/// Instruction-scope duplicate-account audit over a slice of account views.
#[derive(Clone, Copy)]
pub struct AccountAudit<'a> {
    accounts: &'a [AccountView],
}

impl<'a> AccountAudit<'a> {
    #[inline(always)]
    pub const fn new(accounts: &'a [AccountView]) -> Self {
        Self { accounts }
    }

    #[inline(always)]
    pub fn accounts(&self) -> &'a [AccountView] {
        self.accounts
    }

    #[inline]
    pub fn first_duplicate(&self) -> Option<DuplicateAccount> {
        self.first_duplicate_where(|_, _| true)
    }

    #[inline]
    pub fn first_duplicate_writable(&self) -> Option<DuplicateAccount> {
        self.first_duplicate_where(|left, right| left.is_writable() || right.is_writable())
    }

    #[inline]
    pub fn first_duplicate_signer(&self) -> Option<DuplicateAccount> {
        self.first_duplicate_where(|left, right| left.is_signer() || right.is_signer())
    }

    #[inline]
    pub fn require_all_unique(&self) -> ProgramResult {
        if self.first_duplicate().is_some() {
            Err(ProgramError::InvalidArgument)
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn require_unique_writable(&self) -> ProgramResult {
        if self.first_duplicate_writable().is_some() {
            Err(ProgramError::InvalidArgument)
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn require_unique_signers(&self) -> ProgramResult {
        if self.first_duplicate_signer().is_some() {
            Err(ProgramError::InvalidArgument)
        } else {
            Ok(())
        }
    }

    #[inline]
    fn first_duplicate_where(
        &self,
        predicate: impl Fn(&AccountView, &AccountView) -> bool,
    ) -> Option<DuplicateAccount> {
        let mut i = 0;
        while i < self.accounts.len() {
            let left = &self.accounts[i];
            let mut j = i + 1;
            while j < self.accounts.len() {
                let right = &self.accounts[j];
                if left.address() == right.address() && predicate(left, right) {
                    return Some(DuplicateAccount {
                        first_index: i,
                        second_index: j,
                        address: *left.address(),
                    });
                }
                j += 1;
            }
            i += 1;
        }
        None
    }
}

#[cfg(all(test, feature = "hopper-native-backend"))]
mod tests {
    use super::*;

    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make_account(
        address_byte: u8,
        is_signer: bool,
        is_writable: bool,
    ) -> (std::vec::Vec<u8>, AccountView) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + 16];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: is_signer as u8,
                is_writable: is_writable as u8,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([address_byte; 32]),
                owner: NativeAddress::new_from_array([7; 32]),
                lamports: 1,
                data_len: 16,
            });
        }
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    #[test]
    fn detects_any_duplicate() {
        let (_a_backing, first) = make_account(1, false, false);
        let (_b_backing, second) = make_account(1, false, false);
        let accounts = [first, second];
        let audit = AccountAudit::new(&accounts);

        let duplicate = audit.first_duplicate().unwrap();
        assert_eq!(duplicate.first_index, 0);
        assert_eq!(duplicate.second_index, 1);
        assert_eq!(duplicate.address, Address::new_from_array([1; 32]));
        assert_eq!(audit.require_all_unique(), Err(ProgramError::InvalidArgument));
    }

    #[test]
    fn read_only_duplicates_do_not_fail_writable_audit() {
        let (_a_backing, first) = make_account(2, false, false);
        let (_b_backing, second) = make_account(2, false, false);
        let accounts = [first, second];
        let audit = AccountAudit::new(&accounts);

        assert!(audit.first_duplicate_writable().is_none());
        assert_eq!(audit.require_unique_writable(), Ok(()));
    }

    #[test]
    fn writable_duplicates_are_rejected() {
        let (_a_backing, first) = make_account(3, false, true);
        let (_b_backing, second) = make_account(3, false, false);
        let accounts = [first, second];
        let audit = AccountAudit::new(&accounts);

        let duplicate = audit.first_duplicate_writable().unwrap();
        assert_eq!(duplicate.address, Address::new_from_array([3; 32]));
        assert_eq!(audit.require_unique_writable(), Err(ProgramError::InvalidArgument));
    }

    #[test]
    fn signer_duplicates_are_rejected() {
        let (_a_backing, first) = make_account(4, true, false);
        let (_b_backing, second) = make_account(4, false, false);
        let accounts = [first, second];
        let audit = AccountAudit::new(&accounts);

        let duplicate = audit.first_duplicate_signer().unwrap();
        assert_eq!(duplicate.address, Address::new_from_array([4; 32]));
        assert_eq!(audit.require_unique_signers(), Err(ProgramError::InvalidArgument));
    }
}