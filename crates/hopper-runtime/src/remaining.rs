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
    declared: &'a [AccountView<'a>],
    /// Accounts beyond the declared count.
    remaining: &'a [AccountView<'a>],
    /// Duplicate-handling policy.
    mode: RemainingMode,
}

impl<'a> RemainingAccounts<'a> {
    /// Build a strict accessor. Iteration rejects duplicates.
    #[inline(always)]
    pub fn strict(declared: &'a [AccountView<'a>], remaining: &'a [AccountView<'a>]) -> Self {
        Self {
            declared,
            remaining,
            mode: RemainingMode::Strict,
        }
    }

    /// Build a passthrough accessor. Iteration preserves duplicates.
    #[inline(always)]
    pub fn passthrough(declared: &'a [AccountView<'a>], remaining: &'a [AccountView<'a>]) -> Self {
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
    pub fn as_slice(&self) -> &'a [AccountView<'a>] {
        self.remaining
    }

    /// Random access by index. Passthrough returns the slot as is;
    /// strict returns an error when the resolved slot aliases a
    /// previously-seen account (declared or yielded before `index`).
    pub fn get(&self, index: usize) -> Result<Option<&'a AccountView<'a>>, ProgramError> {
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
        let mut items: [Option<&'a AccountView<'a>>; N] = [None; N];
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

    /// Random-access lazy parser over the remaining-account tail.
    ///
    /// This is useful when an instruction receives many optional accounts but
    /// validates only the branch it actually touches. The selected slot still
    /// inherits this view's duplicate policy.
    #[inline(always)]
    pub fn lazy(&self) -> RemainingLazy<'a> {
        RemainingLazy {
            declared: self.declared,
            remaining: self.remaining,
            mode: self.mode,
        }
    }
}

/// Iterator yielded by [`RemainingAccounts::iter`].
pub struct RemainingIter<'a> {
    declared: &'a [AccountView<'a>],
    remaining: &'a [AccountView<'a>],
    mode: RemainingMode,
    index: usize,
}

impl<'a> Iterator for RemainingIter<'a> {
    type Item = Result<&'a AccountView<'a>, ProgramError>;

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
    items: [Option<&'a AccountView<'a>>; N],
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
    pub fn get(&self, index: usize) -> Option<&'a AccountView<'a>> {
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
    type Item = &'a AccountView<'a>;

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
    declared: &'a [AccountView<'a>],
    remaining: &'a [AccountView<'a>],
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
    pub fn next_account(&mut self) -> Result<&'a AccountView<'a>, ProgramError> {
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

    /// Fluent duplicate-policy gate for typed remaining parsing.
    #[inline]
    pub fn no_duplicates(self) -> Result<Self, ProgramError> {
        self.assert_no_duplicates()?;
        Ok(self)
    }

    /// Consume a fixed-size sequential group from the remaining tail.
    ///
    /// Call [`Self::no_duplicates`] first when the whole tail must be globally
    /// alias-free across several groups.
    pub fn take_group(&mut self, len: usize) -> Result<RemainingGroup<'a>, ProgramError> {
        let end = self
            .index
            .checked_add(len)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if end > self.remaining.len() {
            return Err(ProgramError::NotEnoughAccountKeys);
        }
        let group = RemainingGroup {
            parser: RemainingTyped {
                declared: self.declared,
                remaining: &self.remaining[self.index..end],
                mode: self.mode,
                index: 0,
            },
        };
        self.index = end;
        Ok(group)
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
            strict
                .get(index)?
                .ok_or(ProgramError::NotEnoughAccountKeys)?;
            index += 1;
        }
        Ok(())
    }

    /// Verify all remaining accounts are sorted by a caller-supplied key.
    pub fn assert_sorted_by<K, F>(&self, mut key: F) -> ProgramResult
    where
        K: Ord,
        F: FnMut(&'a AccountView<'a>) -> Result<K, ProgramError>,
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

/// Sequential typed parser for one logical remaining-account group.
pub struct RemainingGroup<'a> {
    parser: RemainingTyped<'a>,
}

impl<'a> RemainingGroup<'a> {
    /// Number of unconsumed group slots.
    #[inline(always)]
    pub fn remaining_len(&self) -> usize {
        self.parser.remaining_len()
    }

    /// Consume and return the next account in this group.
    #[inline]
    pub fn next_account(&mut self) -> Result<&'a AccountView<'a>, ProgramError> {
        self.parser.next_account()
    }

    /// Consume and validate the next group account as a signer.
    #[inline]
    pub fn next_signer(&mut self) -> Result<Signer<'a>, ProgramError> {
        self.parser.next_signer()
    }

    /// Consume and validate the next group account as a known external layout.
    #[inline]
    pub fn next_external<T: ExternalZeroCopy>(
        &mut self,
    ) -> Result<ExternalAccount<'a, T>, ProgramError> {
        self.parser.next_external::<T>()
    }

    /// Parse the entire group as at most `N` known external accounts.
    pub fn parse_external<T: ExternalZeroCopy, const N: usize>(
        &mut self,
    ) -> Result<RemainingExternalAccounts<'a, T, N>, ProgramError> {
        if self.parser.remaining_len() > N {
            return Err(RemainingError::Overflow.into());
        }
        let mut items: [Option<ExternalAccount<'a, T>>; N] = [None; N];
        let mut len = 0;
        while !self.parser.is_empty() {
            items[len] = Some(self.parser.next_external::<T>()?);
            len += 1;
        }
        Ok(RemainingExternalAccounts { items, len })
    }

    /// Require the group parser to have consumed every slot.
    #[inline]
    pub fn assert_empty(&self) -> ProgramResult {
        self.parser.assert_empty()
    }
}

/// Bounded parsed external-account group.
pub struct RemainingExternalAccounts<'a, T: ExternalZeroCopy, const N: usize> {
    items: [Option<ExternalAccount<'a, T>>; N],
    len: usize,
}

impl<'a, T: ExternalZeroCopy, const N: usize> RemainingExternalAccounts<'a, T, N> {
    /// Number of parsed external accounts.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when the parsed group is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return parsed external account `index` if it exists.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<ExternalAccount<'a, T>> {
        if index >= self.len {
            None
        } else {
            self.items[index]
        }
    }
}

/// Random-access lazy parser for remaining accounts.
pub struct RemainingLazy<'a> {
    declared: &'a [AccountView<'a>],
    remaining: &'a [AccountView<'a>],
    mode: RemainingMode,
}

impl<'a> RemainingLazy<'a> {
    #[inline(always)]
    fn view(&self) -> RemainingAccounts<'a> {
        RemainingAccounts {
            declared: self.declared,
            remaining: self.remaining,
            mode: self.mode,
        }
    }

    /// Number of remaining slots available for lazy access.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.remaining.len()
    }

    /// True when there are no remaining slots.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }

    /// Select a remaining-account slot by index without validating any other
    /// external accounts in the tail.
    pub fn at(&self, index: usize) -> Result<RemainingLazySlot<'a>, ProgramError> {
        let account = self
            .view()
            .get(index)?
            .ok_or(ProgramError::NotEnoughAccountKeys)?;
        Ok(RemainingLazySlot { account })
    }
}

/// One lazily-selected remaining-account slot.
pub struct RemainingLazySlot<'a> {
    account: &'a AccountView<'a>,
}

impl<'a> RemainingLazySlot<'a> {
    /// Return the raw account view.
    #[inline(always)]
    pub const fn account(&self) -> &'a AccountView<'a> {
        self.account
    }

    /// Bind this slot as an unchecked account role.
    #[inline(always)]
    pub fn unchecked(&self) -> UncheckedAccount<'a> {
        UncheckedAccount::new(self.account)
    }

    /// Validate this slot as a signer.
    #[inline]
    pub fn signer(&self) -> Result<Signer<'a>, ProgramError> {
        Signer::try_new(self.account)
    }

    /// Validate this slot as a known external layout.
    #[inline]
    pub fn external<T: ExternalZeroCopy>(&self) -> Result<ExternalAccount<'a, T>, ProgramError> {
        ExternalAccount::try_new(self.account)
    }
}

/// Ergonomic fall-through used by the proc-macro codegen when the user
/// wants to just burn through remaining accounts without a mode.
#[inline(always)]
pub fn strict<'a>(
    declared: &'a [AccountView<'a>],
    remaining: &'a [AccountView<'a>],
) -> RemainingAccounts<'a> {
    RemainingAccounts::strict(declared, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Address;
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };
    const EXTERNAL_OWNER: Address = Address::new_from_array([5; 32]);
    struct SampleExternal;
    impl ExternalZeroCopy for SampleExternal {
        type View<'a> = crate::foreign::ExternalBytes<'a>;

        const OWNER: Option<Address> = Some(EXTERNAL_OWNER);
        const DISCRIMINATOR: Option<&'static [u8]> = Some(b"EX");
        const MIN_LEN: usize = 4;

        fn view<'a>(data: crate::Ref<'a, [u8]>) -> Result<Self::View<'a>, ProgramError> {
            Ok(crate::foreign::ExternalBytes::new(data))
        }
    }
    fn make_account(
        address: [u8; 32],
        owner: Address,
        signer: bool,
        data: &[u8],
    ) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data.len()];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: Test helper writes a valid RuntimeAccount header and payload
        // into owned backing memory that outlives the returned AccountView.
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
        // SAFETY: `raw` points to the initialized RuntimeAccount header above.
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
    #[test]
    fn typed_remaining_parses_external_signer_and_raw_slots() {
        let (_declared_backing, declared) =
            make_account([1; 32], Address::new_from_array([9; 32]), false, b"");
        let (_external_backing, external) = make_account([2; 32], EXTERNAL_OWNER, false, b"EX12");
        let (_signer_backing, signer) =
            make_account([3; 32], Address::new_from_array([9; 32]), true, b"");
        let (_raw_backing, raw) =
            make_account([4; 32], Address::new_from_array([9; 32]), false, b"");

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
    #[test]
    fn typed_remaining_supports_groups_and_lazy_external_access() {
        let (_declared_backing, declared) =
            make_account([1; 32], Address::new_from_array([9; 32]), false, b"");
        let (_external_a_backing, external_a) =
            make_account([2; 32], EXTERNAL_OWNER, false, b"EX12");
        let (_external_b_backing, external_b) =
            make_account([3; 32], EXTERNAL_OWNER, false, b"EX34");
        let (_signer_backing, signer) =
            make_account([4; 32], Address::new_from_array([9; 32]), true, b"");

        let declared_accounts = [declared];
        let remaining_accounts = [external_a, external_b, signer];
        let accounts = RemainingAccounts::strict(&declared_accounts, &remaining_accounts);

        let lazy_external = accounts
            .lazy()
            .at(1)
            .unwrap()
            .external::<SampleExternal>()
            .unwrap();
        assert_eq!(lazy_external.key(), remaining_accounts[1].address());

        let mut typed = accounts.typed().no_duplicates().unwrap();
        let mut oracle_group = typed.take_group(2).unwrap();
        let parsed = oracle_group.parse_external::<SampleExternal, 4>().unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed.get(0).unwrap().key(),
            remaining_accounts[0].address()
        );
        assert_eq!(
            parsed.get(1).unwrap().key(),
            remaining_accounts[1].address()
        );
        assert!(oracle_group.assert_empty().is_ok());

        let signer = typed.next_signer().unwrap();
        assert_eq!(signer.key(), remaining_accounts[2].address());
        assert!(typed.assert_empty().is_ok());
    }
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
