//! Verified executable program reference.
//!
//! Wraps an AccountView and validates that it is executable. Used for
//! program accounts passed to instructions (e.g. token_program, system_program).

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use crate::check;

/// A verified executable program reference.
///
/// Construction validates `is_executable`. After construction, the
/// executable property is proven by the type system.
pub struct ProgramRef<'a> {
    view: &'a AccountView,
}

impl<'a> ProgramRef<'a> {
    /// Construct from an AccountView, validating the executable flag.
    #[inline]
    pub fn from_account(account: &'a AccountView) -> Result<Self, ProgramError> {
        check::check_executable(account)?;
        Ok(Self { view: account })
    }

    /// Construct and also verify the program's address matches expected.
    #[inline]
    pub fn from_account_checked(
        account: &'a AccountView,
        expected_key: &Address,
    ) -> Result<Self, ProgramError> {
        check::check_executable(account)?;
        check::check_address(account, expected_key)?;
        Ok(Self { view: account })
    }

    /// The program's address.
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
