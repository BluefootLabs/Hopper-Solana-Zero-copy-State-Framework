//! Remaining-accounts accessor with strict and passthrough modes.
//!
//! The declared context validates exactly `ACCOUNT_COUNT` accounts.
//! Any accounts beyond that index are "remaining": pool participants,
//! keeper bot recipients, arbitrary fanout destinations, remainder
//! destinations for sweeps, and so on. Hopper exposes two ways to
//! consume them.
//!
//! ## Strict mode
//!
//! Default. The accessor rejects any remaining account whose address
//! matches a previously seen account (either declared or already
//! yielded). Protects against accidental double-spending when a
//! caller tries to alias one slot into two different roles.
//!
//! ```ignore
//! let rem = ctx.remaining_accounts();
//! for maybe_acc in rem.iter() {
//!     let acc = maybe_acc?; // errors on duplicate
//!     // ...
//! }
//! ```
//!
//! ## Passthrough mode
//!
//! Opt-in. Preserves duplicates verbatim. Use when the caller is
//! expected to pass the same account in multiple roles (batched CPI
//! fan-in, for example).
//!
//! ```ignore
//! let rem = ctx.remaining_accounts_passthrough();
//! ```
//!
//! Both modes are O(n) with no heap and no syscalls. Strict mode
//! keeps a small const-sized seen-address cache sized at 64; past
//! that, it falls back to a linear scan of the declared slice plus
//! the yielded-view cursor.

use crate::{
    account::AccountView,
    account_wrappers::{Signer, UncheckedAccount},
    error::ProgramError,
    foreign::{ExternalAccount, ExternalZeroCopy},
    ProgramResult,
};

/// Upper bound on remaining-account iterator length. Matches Quasar's
/// `MAX_REMAINING_ACCOUNTS` so programs porting from one framework to
/// the other see the same ceiling. Exceeding this returns an error
/// rather than risking unbounded stack usage in the seen-address cache.
pub const MAX_REMAINING_ACCOUNTS: usize = 64;

/// Error surface for the remaining-accounts accessor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RemainingError {
    /// Two remaining-account slots resolved to the same address, or a
    /// remaining-account address matched an already-declared account.
    /// Only strict mode emits this.
    DuplicateAccount,
    /// More than [`MAX_REMAINING_ACCOUNTS`] were accessed via the
    /// iterator.
    Overflow,
}

impl From<RemainingError> for ProgramError {
    fn from(e: RemainingError) -> Self {
        match e {
            RemainingError::DuplicateAccount => ProgramError::InvalidAccountData,
            RemainingError::Overflow => ProgramError::InvalidArgument,
        }
    }
}

/// Duplicate-handling policy for a [`RemainingAccounts`] view.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum RemainingMode {
    /// Reject any yielded account whose address matches a declared or
    /// previously-yielded account. Safe default for pool programs
    /// and anything that intends every slot to be distinct.
    Strict,
    /// Yield every slot as is. Use when the caller is expected to
    /// pass aliases (batched fan-in, self-transfers, etc.).
    Passthrough,
}

/// Zero-allocation remaining-accounts view.
///
/// Construct via [`RemainingAccounts::strict`] or
/// [`RemainingAccounts::passthrough`] from the declared slice and the
/// full accounts slice. `#[hopper::context]` emits
/// `ctx.remaining_accounts()` and `ctx.remaining_accounts_passthrough()`
/// accessors that wire these up for you.
pub struct RemainingAccounts<'a> {
    /// Already-validated context accounts, used for dedup in strict mode.
    declared: &'a [AccountView],
    /// Accounts beyond the declared count.
    remaining: &'a [AccountView],
    /// Duplicate-handling policy.
    mode: RemainingMode,
}

impl<'a> RemainingAccounts<'a> {
    /// Build a strict accessor. Iteration rejects duplicates.
    #[inline(always)]
    pub fn strict(declared: &'a [AccountView], remaining: &'a [AccountView]) -> Self {
        Self {
            declared,
            remaining,
            mode: RemainingMode::Strict,
        }
    }

    /// Build a passthrough accessor. Iteration preserves duplicates.
    #[inline(always)]
    pub fn passthrough(declared: &'a [AccountView], remaining: &'a [AccountView]) -> Self {
        Self {
            declared,
            remaining,
            mode: RemainingMode::Passthrough,
        }
    }

    /// Length of the remaining slice, irrespective of mode.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.remaining.len()
    }

    /// True when there are no remaining accounts.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }

    /// The active duplicate-handling policy for this view.
    #[inline(always)]
    pub fn mode(&self) -> RemainingMode {
        self.mode
    }

    /// The raw remaining-account slice backing this view.
    #[inline(always)]
    pub fn as_slice(&self) -> &'a [AccountView] {
        self.remaining
    }

    /// Random access by index. Passthrough returns the slot as is;
    /// strict returns an error when the resolved slot aliases a
    /// previously-seen account (declared or yielded before `index`).
    pub fn get(&self, index: usize) -> Result<Option<&'a AccountView>, ProgramError> {
        if index >= self.remaining.len() {
            return Ok(None);
        }
        let candidate = &self.remaining[index];
        match self.mode {
            RemainingMode::Passthrough => Ok(Some(candidate)),
            RemainingMode::Strict => {
                if index >= MAX_REMAINING_ACCOUNTS {
                    return Err(RemainingError::Overflow.into());
                }
                // Scan declared.
                for d in self.declared {
                    if d.address() == candidate.address() {
                        return Err(RemainingError::DuplicateAccount.into());
                    }
                }
                // Scan remaining[0..index].
                for r in &self.remaining[..index] {
                    if r.address() == candidate.address() {
                        return Err(RemainingError::DuplicateAccount.into());
                    }
                }
                Ok(Some(candidate))
            }
        }
    }

    /// Validate the remaining tail as at most `N` account views.
    ///
    /// In strict mode this also rejects aliases to declared accounts
    /// and duplicate remaining slots before returning the typed set.
    pub fn account_views<const N: usize>(
        &self,
    ) -> Result<RemainingAccountViews<'a, N>, ProgramError> {
        if self.remaining.len() > N {
            return Err(RemainingError::Overflow.into());
        }
        let mut items: [Option<&'a AccountView>; N] = [None; N];
        let mut index = 0;
        while index < self.remaining.len() {
            let account = self.get(index)?.ok_or(ProgramError::NotEnoughAccountKeys)?;
            items[index] = Some(account);
            index += 1;
        }
        Ok(RemainingAccountViews { items, len: index })
    }

    /// Validate the remaining tail as at most `N` signer accounts.
    ///
    /// This is the common multisig case: the handler gets a bounded,
    /// duplicate-safe signer set instead of raw account iteration.
    pub fn signers<const N: usize>(&self) -> Result<RemainingSigners<'a, N>, ProgramError> {
        if self.remaining.len() > N {
            return Err(RemainingError::Overflow.into());
        }
        let mut items: [Option<Signer<'a>>; N] = [None; N];
        let mut index = 0;
        while index < self.remaining.len() {
            let account = self.get(index)?.ok_or(ProgramError::NotEnoughAccountKeys)?;
            items[index] = Some(Signer::try_new(account)?);
            index += 1;
        }
        Ok(RemainingSigners { items, len: index })
    }

    /// Sequential iterator. Yields each account in declaration order,
    /// errors on duplicates in strict mode, preserves them in
    /// passthrough mode.
    #[inline(always)]
    pub fn iter(&self) -> RemainingIter<'a> {
        RemainingIter {
            declared: self.declared,
            remaining: self.remaining,
            mode: self.mode,
            index: 0,
        }
    }

    /// Sequential typed parser over the remaining-account tail.
    ///
    /// This preserves the current duplicate policy while letting handlers bind
    /// each slot as a signer, raw account, or known external account in the
    /// order protocol instructions naturally expect.
    #[inline(always)]
    pub fn typed(&self) -> RemainingTyped<'a> {
        RemainingTyped {
            declared: self.declared,
            remaining: self.remaining,
            mode: self.mode,
            index: 0,
        }
    }
}

/// Iterator yielded by [`RemainingAccounts::iter`].
pub struct RemainingIter<'a> {
    declared: &'a [AccountView],
    remaining: &'a [AccountView],
    mode: RemainingMode,
    index: usize,
}

impl<'a> Iterator for RemainingIter<'a> {
    type Item = Result<&'a AccountView, ProgramError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.remaining.len() {
            return None;
        }
        if self.index >= MAX_REMAINING_ACCOUNTS {
            // Pin the cursor so repeated calls after overflow stay
            // cheap and deterministic.
            self.index = self.remaining.len();
            return Some(Err(RemainingError::Overflow.into()));
        }
        let candidate = &self.remaining[self.index];
        let i = self.index;
        self.index = self.index.wrapping_add(1);

        if matches!(self.mode, RemainingMode::Strict) {
            for d in self.declared {
                if d.address() == candidate.address() {
                    return Some(Err(RemainingError::DuplicateAccount.into()));
                }
            }
            for r in &self.remaining[..i] {
                if r.address() == candidate.address() {
                    return Some(Err(RemainingError::DuplicateAccount.into()));
                }
            }
        }
        Some(Ok(candidate))
    }
}

/// Bounded, validated remaining account-view set.
pub struct RemainingAccountViews<'a, const N: usize> {
    items: [Option<&'a AccountView>; N],
    len: usize,
}

impl<'a, const N: usize> RemainingAccountViews<'a, N> {
    /// Number of parsed account views.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when the parsed set is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return account `index` if it exists.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&'a AccountView> {
        if index >= self.len {
            None
        } else {
            self.items[index]
        }
    }

    /// Iterate over the parsed account views.
    #[inline(always)]
    pub fn iter(&self) -> RemainingAccountViewIter<'_, 'a, N> {
        RemainingAccountViewIter {
            set: self,
            index: 0,
        }
    }
}

/// Iterator over a bounded account-view set.
pub struct RemainingAccountViewIter<'set, 'a, const N: usize> {
    set: &'set RemainingAccountViews<'a, N>,
    index: usize,
}

impl<'a, const N: usize> Iterator for RemainingAccountViewIter<'_, 'a, N> {
    type Item = &'a AccountView;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.set.len {
            return None;
        }
        let item = self.set.items[self.index];
        self.index += 1;
        item
    }
}

/// Bounded, validated remaining signer set.
pub struct RemainingSigners<'a, const N: usize> {
    items: [Option<Signer<'a>>; N],
    len: usize,
}

impl<'a, const N: usize> RemainingSigners<'a, N> {
    /// Number of parsed signers.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when the parsed set is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return signer `index` if it exists.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<Signer<'a>> {
        if index >= self.len {
            None
        } else {
            self.items[index]
        }
    }

    /// Iterate over the parsed signers.
    #[inline(always)]
    pub fn iter(&self) -> RemainingSignerIter<'_, 'a, N> {
        RemainingSignerIter {
            set: self,
            index: 0,
        }
    }
}

/// Iterator over a bounded signer set.
pub struct RemainingSignerIter<'set, 'a, const N: usize> {
    set: &'set RemainingSigners<'a, N>,
    index: usize,
}

impl<'a, const N: usize> Iterator for RemainingSignerIter<'_, 'a, N> {
    type Item = Signer<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.set.len {
            return None;
        }
        let item = self.set.items[self.index];
        self.index += 1;
        item
    }
}

/// Sequential typed parser for remaining accounts.
pub struct RemainingTyped<'a> {
    declared: &'a [AccountView],
    remaining: &'a [AccountView],
    mode: RemainingMode,
    index: usize,
}

impl<'a> RemainingTyped<'a> {
    #[inline(always)]
    fn view(&self) -> RemainingAccounts<'a> {
        RemainingAccounts {
            declared: self.declared,
            remaining: self.remaining,
            mode: self.mode,
        }
    }

    /// Number of slots already consumed by typed parsing.
    #[inline(always)]
    pub const fn consumed(&self) -> usize {
        self.index
    }

    /// Number of unconsumed remaining slots.
    #[inline(always)]
    pub fn remaining_len(&self) -> usize {
        self.remaining.len().saturating_sub(self.index)
    }

    /// True when all remaining slots have been consumed.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.index >= self.remaining.len()
    }

    /// Consume and return the next raw account view.
    pub fn next_account(&mut self) -> Result<&'a AccountView, ProgramError> {
        let account = self
            .view()
            .get(self.index)?
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        self.index += 1;
        Ok(account)
    }

    /// Consume and return the next raw account view as an explicit unchecked role.
    #[inline]
    pub fn next_unchecked(&mut self) -> Result<UncheckedAccount<'a>, ProgramError> {
        Ok(UncheckedAccount::new(self.next_account()?))
    }

    /// Consume and validate the next account as a signer.
    #[inline]
    pub fn next_signer(&mut self) -> Result<Signer<'a>, ProgramError> {
        Signer::try_new(self.next_account()?)
    }

    /// Consume and validate the next account as a known external layout.
    #[inline]
    pub fn next_external<T: ExternalZeroCopy>(
        &mut self,
    ) -> Result<ExternalAccount<'a, T>, ProgramError> {
        ExternalAccount::try_new(self.next_account()?)
    }

    /// Verify every remaining slot is distinct from declared and sibling slots.
    pub fn assert_no_duplicates(&self) -> ProgramResult {
        let strict = RemainingAccounts {
            declared: self.declared,
            remaining: self.remaining,
            mode: RemainingMode::Strict,
        };
        let mut index = 0;
        while index < self.remaining.len() {
            strict.get(index)?.ok_or(ProgramError::NotEnoughAccountKeys)?;
            index += 1;
        }
        Ok(())
    }

    /// Verify all remaining accounts are sorted by a caller-supplied key.
    pub fn assert_sorted_by<K, F>(&self, mut key: F) -> ProgramResult
    where
        K: Ord,
        F: FnMut(&'a AccountView) -> Result<K, ProgramError>,
    {
        let view = self.view();
        let mut previous: Option<K> = None;
        let mut index = 0;
        while index < self.remaining.len() {
            let account = view.get(index)?.ok_or(ProgramError::NotEnoughAccountKeys)?;
            let current = key(account)?;
            if let Some(ref last) = previous {
                if current < *last {
                    return Err(ProgramError::InvalidAccountData);
                }
            }
            previous = Some(current);
            index += 1;
        }
        Ok(())
    }

    /// Require the typed parser to have consumed the full tail.
    #[inline]
    pub fn assert_empty(&self) -> ProgramResult {
        if self.is_empty() {
            Ok(())
        } else {
            Err(ProgramError::InvalidArgument)
        }
    }
}

/// Ergonomic fall-through used by the proc-macro codegen when the user
/// wants to just burn through remaining accounts without a mode.
#[inline(always)]
pub fn strict<'a>(
    declared: &'a [AccountView],
    remaining: &'a [AccountView],
) -> RemainingAccounts<'a> {
    RemainingAccounts::strict(declared, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "hopper-native-backend")]
    use crate::Address;

    #[cfg(feature = "hopper-native-backend")]
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    #[cfg(feature = "hopper-native-backend")]
    const EXTERNAL_OWNER: Address = Address::new_from_array([5; 32]);

    #[cfg(feature = "hopper-native-backend")]
    struct SampleExternal;

    #[cfg(feature = "hopper-native-backend")]
    impl ExternalZeroCopy for SampleExternal {
        const OWNER: Option<Address> = Some(EXTERNAL_OWNER);
        const DISCRIMINATOR: Option<&'static [u8]> = Some(b"EX");
        const MIN_LEN: usize = 4;
    }

    #[cfg(feature = "hopper-native-backend")]
    fn make_account(
        address: [u8; 32],
        owner: Address,
        signer: bool,
        data: &[u8],
    ) -> (std::vec::Vec<u8>, AccountView) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data.len()];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: signer as u8,
                is_writable: 0,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array(address),
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

    // `AccountView` is backend-specific; we cannot construct one under
    // a non-Solana `cfg`. These tests exist to keep the module
    // exercised at compile time even when the construction helpers
    // live behind `target_os = "solana"`.

    #[test]
    fn error_variants_surface_as_program_error() {
        let dup: ProgramError = RemainingError::DuplicateAccount.into();
        assert_eq!(dup, ProgramError::InvalidAccountData);
        let ovf: ProgramError = RemainingError::Overflow.into();
        assert_eq!(ovf, ProgramError::InvalidArgument);
    }

    #[test]
    fn max_remaining_matches_quasar() {
        // If we ever change this, also update the Quasar parity doc.
        assert_eq!(MAX_REMAINING_ACCOUNTS, 64);
    }

    #[cfg(feature = "hopper-native-backend")]
    #[test]
    fn typed_remaining_parses_external_signer_and_raw_slots() {
        let (_declared_backing, declared) =
            make_account([1; 32], Address::new_from_array([9; 32]), false, b"");
        let (_external_backing, external) = make_account([2; 32], EXTERNAL_OWNER, false, b"EX12");
        let (_signer_backing, signer) =
            make_account([3; 32], Address::new_from_array([9; 32]), true, b"");
        let (_raw_backing, raw) = make_account([4; 32], Address::new_from_array([9; 32]), false, b"");

        let declared_accounts = [declared];
        let remaining_accounts = [external, signer, raw];
        let mut typed = RemainingAccounts::strict(&declared_accounts, &remaining_accounts).typed();

        let external = typed.next_external::<SampleExternal>().unwrap();
        assert_eq!(external.key(), remaining_accounts[0].address());
        let signer = typed.next_signer().unwrap();
        assert_eq!(signer.key(), remaining_accounts[1].address());
        let raw = typed.next_unchecked().unwrap();
        assert_eq!(raw.key(), remaining_accounts[2].address());
        assert!(typed.assert_empty().is_ok());
    }

    #[cfg(feature = "hopper-native-backend")]
    #[test]
    fn typed_remaining_duplicate_and_sort_assertions_are_explicit() {
        let (_declared_backing, declared) =
            make_account([1; 32], Address::new_from_array([9; 32]), false, b"");
        let (_duplicate_backing, duplicate) =
            make_account([1; 32], Address::new_from_array([9; 32]), false, b"");
        let declared_accounts = [declared];
        let remaining_accounts = [duplicate];
        let typed = RemainingAccounts::passthrough(&declared_accounts, &remaining_accounts).typed();
        assert_eq!(
            typed.assert_no_duplicates().unwrap_err(),
            ProgramError::InvalidAccountData
        );

        let (_a_backing, a) = make_account([3; 32], Address::new_from_array([9; 32]), false, b"");
        let (_b_backing, b) = make_account([2; 32], Address::new_from_array([9; 32]), false, b"");
        let remaining_accounts = [a, b];
        let typed = RemainingAccounts::passthrough(&[], &remaining_accounts).typed();
        assert_eq!(
            typed
                .assert_sorted_by(|account| Ok(account.address().as_bytes()[0]))
                .unwrap_err(),
            ProgramError::InvalidAccountData
        );
    }
}
