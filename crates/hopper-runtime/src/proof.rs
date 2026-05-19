//! Proof-carrying account markers.
//!
//! This is the lightweight capability layer over `AccountView` checks. Each
//! method consumes a proof and returns the same account with an extra marker in
//! the type, so downstream helpers can require `OwnerChecked`, `SignerChecked`,
//! `LayoutChecked<T>`, or any tuple composition of those markers.

use core::marker::PhantomData;

use crate::{AccountView, Address, LayoutContract, ProgramError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unchecked;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnerChecked;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutChecked<T>(PhantomData<T>);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SignerChecked;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WritableChecked;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutableChecked;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenExtensionsChecked;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeedsChecked;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HasOneChecked;

#[derive(Clone, Copy)]
pub struct AccountProof<'a, P = Unchecked> {
    account: &'a AccountView,
    _proof: PhantomData<P>,
}

impl<'a> AccountProof<'a, Unchecked> {
    #[inline(always)]
    pub const fn new(account: &'a AccountView) -> Self {
        Self {
            account,
            _proof: PhantomData,
        }
    }
}

impl<'a, P> AccountProof<'a, P> {
    #[inline(always)]
    pub const fn account(&self) -> &'a AccountView {
        self.account
    }

    #[inline(always)]
    pub fn check_owner(
        self,
        expected: &Address,
    ) -> Result<AccountProof<'a, (P, OwnerChecked)>, ProgramError> {
        self.account.check_owned_by(expected)?;
        Ok(AccountProof::<'a, (P, OwnerChecked)> {
            account: self.account,
            _proof: PhantomData,
        })
    }

    #[inline(always)]
    pub fn check_layout<T: LayoutContract>(
        self,
    ) -> Result<AccountProof<'a, (P, LayoutChecked<T>)>, ProgramError> {
        self.account.check_layout::<T>()?;
        Ok(AccountProof::<'a, (P, LayoutChecked<T>)> {
            account: self.account,
            _proof: PhantomData,
        })
    }

    #[inline(always)]
    pub fn check_signer(self) -> Result<AccountProof<'a, (P, SignerChecked)>, ProgramError> {
        self.account.check_signer()?;
        Ok(AccountProof::<'a, (P, SignerChecked)> {
            account: self.account,
            _proof: PhantomData,
        })
    }

    #[inline(always)]
    pub fn check_writable(self) -> Result<AccountProof<'a, (P, WritableChecked)>, ProgramError> {
        self.account.check_writable()?;
        Ok(AccountProof::<'a, (P, WritableChecked)> {
            account: self.account,
            _proof: PhantomData,
        })
    }

    #[inline(always)]
    pub fn check_executable(
        self,
    ) -> Result<AccountProof<'a, (P, ExecutableChecked)>, ProgramError> {
        self.account.check_executable()?;
        Ok(AccountProof::<'a, (P, ExecutableChecked)> {
            account: self.account,
            _proof: PhantomData,
        })
    }

    #[inline(always)]
    pub const fn assume_token_extensions_checked(
        self,
    ) -> AccountProof<'a, (P, TokenExtensionsChecked)> {
        AccountProof::<'a, (P, TokenExtensionsChecked)> {
            account: self.account,
            _proof: PhantomData,
        }
    }
}
