//! Stack-allocated variable-length CPI builder.
//!
//! The existing `hopper_runtime::cpi::invoke_signed::<N>` family is
//! const-generic over the account count, which is perfect for CPI
//! shapes known at compile time and about ninety percent of real
//! cases. The exceptions are:
//!
//! - Aggregators that invoke the same program with a runtime-
//!   decided account count (fanout fee routers, batch settlement
//!   cranks).
//! - Forwarders that pass through the caller's remaining accounts
//!   after splicing in a known prefix.
//! - Generic instruction builders that construct the data buffer
//!   byte-by-byte from user input (priority-fee overrides, optional
//!   bump seeds) and do not know the final length until build time.
//!
//! [`DynCpi`] covers those cases. It is parameterised on two
//! compile-time capacities, `MAX_ACCTS` and `MAX_DATA`, so the whole
//! buffer lives on the stack in a single `MaybeUninit` array. No
//! heap, no `Vec`, no panic on overflow: [`DynCpi::push_account`]
//! and [`DynCpi::push_data`] return errors when the declared
//! capacity would be exceeded.
//!
//! ## Innovation vs. Quasar
//!
//! Quasar's `DynCpiCall` is conceptually the same shape but expects
//! the caller to hand-roll seed threading. Hopper's builder carries
//! a typed `Signer` slice through the invoke call so a PDA-authored
//! CPI reads like a single method chain. The overflow discipline
//! also differs: Hopper propagates `Err(ProgramError::InvalidArgument)`
//! rather than panicking, which keeps the handler's error surface
//! uniform.

use core::mem::MaybeUninit;

use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::{
    account::AccountView,
    address::{address_eq, Address},
    error::ProgramError,
    result::ProgramResult,
};

/// Variable-length CPI builder with compile-time stack capacity.
///
/// `MAX_ACCTS` is the upper bound on the number of `AccountMeta`
/// entries. `MAX_DATA` is the upper bound on the instruction data
/// byte count. Exceeding either returns an error; nothing panics.
///
/// Use when the CPI shape is not known at compile time. For
/// statically-shaped CPIs, prefer `cpi::invoke_signed::<N>` which
/// avoids the two bounds entirely.
pub struct DynCpi<'a, const MAX_ACCTS: usize, const MAX_DATA: usize> {
    program_id: &'a Address,
    // Per-push (meta) storage: one slot per `push_account`, order and
    // duplicates preserved — this is the ordered meta list the callee sees.
    accounts: [MaybeUninit<&'a AccountView<'a>>; MAX_ACCTS],
    writable: [bool; MAX_ACCTS],
    signer: [bool; MAX_ACCTS],
    account_count: usize,
    // SIMD-0339 dedup projection (by pubkey): one entry per *unique*
    // account. `info_first[k]` is the push index of the first occurrence of
    // unique account `k` (used to recover its `AccountView`); the writable /
    // signer flags are the OR across every occurrence, because an account
    // that is writable (or a signer) in *any* meta must be passed to the
    // syscall as writable (or signer). `u16` suffices: MAX_ACCTS never
    // exceeds the 255 SIMD-0339 account ceiling in practice.
    info_first: [u16; MAX_ACCTS],
    info_writable: [bool; MAX_ACCTS],
    info_signer: [bool; MAX_ACCTS],
    info_count: usize,
    data: [MaybeUninit<u8>; MAX_DATA],
    data_len: usize,
}

impl<'a, const MAX_ACCTS: usize, const MAX_DATA: usize> DynCpi<'a, MAX_ACCTS, MAX_DATA> {
    /// Start a new dynamic CPI against the given program.
    #[inline]
    pub fn new(program_id: &'a Address) -> Self {
        Self {
            program_id,
            accounts: [const { MaybeUninit::uninit() }; MAX_ACCTS],
            writable: [false; MAX_ACCTS],
            signer: [false; MAX_ACCTS],
            account_count: 0,
            info_first: [0u16; MAX_ACCTS],
            info_writable: [false; MAX_ACCTS],
            info_signer: [false; MAX_ACCTS],
            info_count: 0,
            data: [const { MaybeUninit::uninit() }; MAX_DATA],
            data_len: 0,
        }
    }

    /// Append one account meta. The `writable` and `signer` flags
    /// are carried through to the emitted CPI instruction.
    ///
    /// Every call appends one *meta* (order and duplicates preserved — the
    /// callee reads accounts positionally). In parallel the builder folds
    /// the account into a deduplicated *info* set keyed by pubkey: pushing
    /// an address already present does **not** allocate a second info slot,
    /// it reuses the existing one and OR-merges the writable/signer flags.
    /// Under SIMD-0339 each distinct account-info costs CU, so N metas of
    /// the same account collapse to a single info at submit time — a saving
    /// a one-info-per-meta builder cannot make. See [`Self::info_count`].
    ///
    /// Returns `Err(ProgramError::InvalidArgument)` when the builder
    /// is already at `MAX_ACCTS` capacity. Users pick the capacity
    /// at the type parameter; bumping it is a type-system edit, not
    /// a runtime error.
    #[inline]
    pub fn push_account(
        &mut self,
        account: &'a AccountView<'a>,
        writable: bool,
        signer: bool,
    ) -> ProgramResult {
        if self.account_count >= MAX_ACCTS {
            return Err(ProgramError::InvalidArgument);
        }
        let idx = self.account_count;
        self.accounts[idx] = MaybeUninit::new(account);
        self.writable[idx] = writable;
        self.signer[idx] = signer;

        // Fold into the deduped info projection (match by pubkey).
        let mut k = 0;
        let mut merged = false;
        while k < self.info_count {
            // SAFETY: `info_first[k] < account_count`, so that `accounts`
            // slot was initialized by an earlier `push_account`.
            let existing = unsafe { self.accounts[self.info_first[k] as usize].assume_init() };
            if address_eq(existing.address(), account.address()) {
                self.info_writable[k] |= writable;
                self.info_signer[k] |= signer;
                merged = true;
                break;
            }
            k += 1;
        }
        if !merged {
            let slot = self.info_count;
            self.info_first[slot] = idx as u16;
            self.info_writable[slot] = writable;
            self.info_signer[slot] = signer;
            self.info_count = self.info_count.wrapping_add(1);
        }

        self.account_count = self.account_count.wrapping_add(1);
        Ok(())
    }

    /// Append the given bytes to the instruction data buffer.
    ///
    /// Returns `Err(ProgramError::InvalidArgument)` when the buffer
    /// does not have room for the full slice. The append is
    /// all-or-nothing; a partial write does not happen.
    #[inline]
    pub fn push_data(&mut self, bytes: &[u8]) -> ProgramResult {
        if self.data_len.saturating_add(bytes.len()) > MAX_DATA {
            return Err(ProgramError::InvalidArgument);
        }
        let dst = &mut self.data[self.data_len..self.data_len + bytes.len()];
        for (i, b) in bytes.iter().enumerate() {
            dst[i] = MaybeUninit::new(*b);
        }
        self.data_len = self.data_len.wrapping_add(bytes.len());
        Ok(())
    }

    /// Append one byte. Sugar for programs that build instruction
    /// data one discriminator + one argument at a time.
    #[inline]
    pub fn push_byte(&mut self, byte: u8) -> ProgramResult {
        self.push_data(core::slice::from_ref(&byte))
    }

    /// Append the little-endian encoding of a `u64`. Covers the
    /// most common arg shape (lamports, timestamps, flags).
    #[inline]
    pub fn push_u64_le(&mut self, value: u64) -> ProgramResult {
        self.push_data(&value.to_le_bytes())
    }

    /// Append a 32-byte pubkey.
    #[inline]
    pub fn push_pubkey(&mut self, address: &Address) -> ProgramResult {
        self.push_data(address.as_array())
    }

    /// Current account (meta) count — one per `push_account`, including
    /// duplicates. This is the length of the ordered meta list the callee
    /// sees, *not* the deduped info count (see [`Self::info_count`]).
    #[inline(always)]
    pub const fn account_count(&self) -> usize {
        self.account_count
    }

    /// Number of *unique* account-infos after SIMD-0339 pubkey dedup.
    ///
    /// This is `<= account_count()`, and is exactly the count of
    /// account-infos handed to the syscall at submit time. Pushing the same
    /// address twice leaves this unchanged.
    #[inline(always)]
    pub const fn info_count(&self) -> usize {
        self.info_count
    }

    /// The `k`-th deduplicated account-info: its view plus the OR-merged
    /// `(writable, signer)` privilege across every occurrence. Returns
    /// `None` for `k >= info_count()`.
    ///
    /// Infos are in first-occurrence (push) order, so `dedup_info(0)` is the
    /// account whose first push came first.
    #[inline]
    pub fn dedup_info(&self, k: usize) -> Option<(&'a AccountView<'a>, bool, bool)> {
        if k >= self.info_count {
            return None;
        }
        // SAFETY: `k < info_count`, so `info_first[k] < account_count` names
        // an initialized `accounts` slot.
        let view = unsafe { self.accounts[self.info_first[k] as usize].assume_init() };
        Some((view, self.info_writable[k], self.info_signer[k]))
    }

    /// Program id this dynamic CPI targets.
    #[inline(always)]
    pub const fn program_id(&self) -> &Address {
        self.program_id
    }

    /// Current data length.
    #[inline(always)]
    pub const fn data_len(&self) -> usize {
        self.data_len
    }

    /// Borrow the finalized data buffer. Useful for tests that
    /// want to inspect the wire bytes without actually submitting
    /// the CPI.
    #[inline]
    pub fn data(&self) -> &[u8] {
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        unsafe { core::slice::from_raw_parts(self.data.as_ptr() as *const u8, self.data_len) }
    }

    /// The pushed account views, in push order.
    #[inline]
    pub fn account_views(&self) -> &[&'a AccountView<'a>] {
        // SAFETY: slots `0..account_count` were initialized by
        // `push_account`, and we expose exactly that prefix.
        unsafe {
            core::slice::from_raw_parts(
                self.accounts.as_ptr() as *const &'a AccountView<'a>,
                self.account_count,
            )
        }
    }

    /// Submit the built CPI (no PDA signers).
    ///
    /// Equivalent to [`invoke_signed`](Self::invoke_signed) with an
    /// empty signer set.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    /// Submit the built CPI with the given PDA signer seeds.
    ///
    /// Assembles the pushed `(account, writable, signer)` metas — the full
    /// ordered list, duplicates preserved — and the data buffer into an
    /// [`InstructionView`], then routes it through the **validated,
    /// dedup-aware** path
    /// ([`cpi::invoke_signed_deduped`](crate::cpi::invoke_signed_deduped)).
    /// The metas define what the callee sees positionally; the account-info
    /// list handed to the syscall is the pubkey-deduplicated set (one info
    /// per unique account, flags OR-merged), so under SIMD-0339 duplicate
    /// account-infos cost nothing. Address/flag agreement, PDA-signer
    /// resolution, live-borrow checks, and duplicate-writable rejection all
    /// run over the full meta list before the syscall — dedup never weakens
    /// validation. This is the typed signer threading the module docs
    /// promise: the builder and the submission are one method chain.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let count = self.account_count;
        let views = self.account_views();

        // Full ordered meta list — one meta per push, duplicates kept.
        let mut metas: [MaybeUninit<InstructionAccount<'a>>; MAX_ACCTS] =
            [const { MaybeUninit::uninit() }; MAX_ACCTS];
        let mut i = 0;
        while i < count {
            metas[i] = MaybeUninit::new(InstructionAccount::new(
                views[i].address(),
                self.writable[i],
                self.signer[i],
            ));
            i += 1;
        }
        // SAFETY: slots `0..count` were initialized by the loop above.
        let metas_slice = unsafe {
            core::slice::from_raw_parts(metas.as_ptr() as *const InstructionAccount<'a>, count)
        };
        let instruction = InstructionView {
            program_id: self.program_id,
            data: self.data(),
            accounts: metas_slice,
        };

        // Deduplicated account-info list — one AccountView per unique
        // address, in first-occurrence order.
        let mut infos: [MaybeUninit<&'a AccountView<'a>>; MAX_ACCTS] =
            [const { MaybeUninit::uninit() }; MAX_ACCTS];
        let mut k = 0;
        while k < self.info_count {
            // SAFETY: `info_first[k] < account_count` names an initialized
            // `accounts` slot.
            let view = unsafe { self.accounts[self.info_first[k] as usize].assume_init() };
            infos[k] = MaybeUninit::new(view);
            k += 1;
        }
        // SAFETY: slots `0..info_count` were initialized by the loop above.
        let infos_slice = unsafe {
            core::slice::from_raw_parts(
                infos.as_ptr() as *const &'a AccountView<'a>,
                self.info_count,
            )
        };

        crate::cpi::invoke_signed_deduped::<MAX_ACCTS>(&instruction, infos_slice, signers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_push_walks_the_buffer() {
        let program = Address::from([0u8; 32]);
        let mut cpi: DynCpi<4, 32> = DynCpi::new(&program);
        cpi.push_byte(0xA1).unwrap();
        cpi.push_u64_le(0xCAFEBABE_u64).unwrap();
        assert_eq!(cpi.data_len(), 1 + 8);
        assert_eq!(cpi.data()[0], 0xA1);
        assert_eq!(&cpi.data()[1..9], &0xCAFEBABE_u64.to_le_bytes());
    }

    #[test]
    fn data_overflow_rejects() {
        let program = Address::from([0u8; 32]);
        let mut cpi: DynCpi<0, 4> = DynCpi::new(&program);
        cpi.push_u64_le(1).expect_err("u64 is 8 bytes, buffer is 4");
    }

    #[test]
    fn push_pubkey_fills_32_bytes() {
        let program = Address::from([0u8; 32]);
        let mut cpi: DynCpi<0, 64> = DynCpi::new(&program);
        let pk = Address::from([0x7Au8; 32]);
        cpi.push_pubkey(&pk).unwrap();
        assert_eq!(cpi.data_len(), 32);
        assert!(cpi.data().iter().all(|b| *b == 0x7A));
    }

    mod invoke {
        use super::*;
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        fn make_account(
            address_byte: u8,
            is_signer: bool,
            is_writable: bool,
        ) -> (std::vec::Vec<u64>, AccountView<'static>) {
            let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + 8).div_ceil(8)];
            let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
            // SAFETY: backing is sized for the header plus data and
            // outlives the returned view.
            unsafe {
                raw.write(RuntimeAccount {
                    borrow_state: NOT_BORROWED,
                    is_signer: is_signer as u8,
                    is_writable: is_writable as u8,
                    executable: 0,
                    resize_delta: 0,
                    address: NativeAddress::new_from_array([address_byte; 32]),
                    owner: NativeAddress::new_from_array([2; 32]),
                    lamports: 1,
                    data_len: 8,
                });
            }
            // SAFETY: raw points at a fully initialized RuntimeAccount.
            let backend = unsafe { NativeAccountView::new_unchecked(raw) };
            (backing, AccountView::from_backend(backend))
        }

        #[test]
        fn invoke_validates_and_submits_built_metas() {
            let program = Address::from([9u8; 32]);
            let (_b1, signer_acct) = make_account(1, true, false);
            let (_b2, writable_acct) = make_account(2, false, true);

            let mut cpi: DynCpi<4, 16> = DynCpi::new(&program);
            cpi.push_account(&signer_acct, false, true).unwrap();
            cpi.push_account(&writable_acct, true, false).unwrap();
            cpi.push_byte(3).unwrap();
            cpi.push_u64_le(42).unwrap();

            // Off-chain the syscall is a no-op, but the full validation
            // pipeline (address/flag agreement, borrow checks,
            // duplicate-writable) runs against the metas this builder
            // assembled — proving the build→submit chain is wired.
            assert_eq!(cpi.invoke(), Ok(()));
        }

        #[test]
        fn invoke_surfaces_flag_mismatch_from_built_metas() {
            let program = Address::from([9u8; 32]);
            // The view is NOT a transaction signer, but the builder
            // declares it must sign (and no PDA seeds are provided):
            // the validated path must refuse.
            let (_b1, not_signer) = make_account(3, false, false);

            let mut cpi: DynCpi<2, 4> = DynCpi::new(&program);
            cpi.push_account(&not_signer, false, true).unwrap();
            assert_eq!(cpi.invoke(), Err(ProgramError::MissingRequiredSignature));
        }

        #[test]
        fn invoke_rejects_duplicate_writable_accounts() {
            let program = Address::from([9u8; 32]);
            let (_b1, first) = make_account(4, false, true);
            let (_b2, second) = make_account(4, false, true);

            let mut cpi: DynCpi<2, 4> = DynCpi::new(&program);
            cpi.push_account(&first, true, false).unwrap();
            cpi.push_account(&second, true, false).unwrap();
            assert_eq!(cpi.invoke(), Err(ProgramError::AccountBorrowFailed));
        }

        #[test]
        fn account_views_exposes_push_order_prefix() {
            let program = Address::from([9u8; 32]);
            let (_b1, a) = make_account(5, false, false);
            let (_b2, b) = make_account(6, false, false);

            let mut cpi: DynCpi<4, 4> = DynCpi::new(&program);
            cpi.push_account(&a, false, false).unwrap();
            cpi.push_account(&b, false, false).unwrap();
            let views = cpi.account_views();
            assert_eq!(views.len(), 2);
            assert_eq!(views[0].address(), &Address::from([5u8; 32]));
            assert_eq!(views[1].address(), &Address::from([6u8; 32]));
        }

        // -- SIMD-0339 account-info dedup --------------------------------

        #[test]
        fn repeated_address_collapses_to_one_info_with_or_merged_flags() {
            let program = Address::from([9u8; 32]);
            // One underlying account, pushed twice with complementary flags.
            let (_b, acct) = make_account(7, true, true);

            let mut cpi: DynCpi<4, 4> = DynCpi::new(&program);
            cpi.push_account(&acct, true, false).unwrap(); // writable, not signer
            cpi.push_account(&acct, false, true).unwrap(); // not writable, signer

            // Both pushes are kept as metas...
            assert_eq!(cpi.account_count(), 2);
            // ...but collapse to a single deduplicated account-info.
            assert_eq!(cpi.info_count(), 1);

            let (view, writable, signer) = cpi.dedup_info(0).unwrap();
            assert_eq!(view.address(), &Address::from([7u8; 32]));
            // Flags are the OR across occurrences: writable in push #1,
            // signer in push #2 => the single info is both.
            assert!(writable, "writable in any meta => info writable");
            assert!(signer, "signer in any meta => info signer");
            assert!(cpi.dedup_info(1).is_none());
        }

        #[test]
        fn distinct_addresses_stay_distinct_infos() {
            let program = Address::from([9u8; 32]);
            let (_b1, a) = make_account(5, false, false);
            let (_b2, b) = make_account(6, false, false);

            let mut cpi: DynCpi<4, 4> = DynCpi::new(&program);
            cpi.push_account(&a, false, false).unwrap();
            cpi.push_account(&b, false, false).unwrap();

            assert_eq!(cpi.account_count(), 2);
            assert_eq!(cpi.info_count(), 2);
            assert_eq!(
                cpi.dedup_info(0).unwrap().0.address(),
                &Address::from([5u8; 32])
            );
            assert_eq!(
                cpi.dedup_info(1).unwrap().0.address(),
                &Address::from([6u8; 32])
            );
        }

        #[test]
        fn metas_preserve_order_and_duplicates_while_infos_dedup() {
            let program = Address::from([9u8; 32]);
            let (_b1, a) = make_account(5, false, false);
            let (_b2, b) = make_account(6, false, false);

            // Push order a, b, a: the middle account is distinct, the third
            // repeats the first.
            let mut cpi: DynCpi<4, 4> = DynCpi::new(&program);
            cpi.push_account(&a, false, false).unwrap();
            cpi.push_account(&b, false, false).unwrap();
            cpi.push_account(&a, false, false).unwrap();

            // Metas: all three, in push order, duplicate preserved.
            let metas = cpi.account_views();
            assert_eq!(metas.len(), 3);
            assert_eq!(metas[0].address(), &Address::from([5u8; 32]));
            assert_eq!(metas[1].address(), &Address::from([6u8; 32]));
            assert_eq!(metas[2].address(), &Address::from([5u8; 32]));

            // Infos: two unique, in first-occurrence order.
            assert_eq!(cpi.info_count(), 2);
            assert_eq!(
                cpi.dedup_info(0).unwrap().0.address(),
                &Address::from([5u8; 32])
            );
            assert_eq!(
                cpi.dedup_info(1).unwrap().0.address(),
                &Address::from([6u8; 32])
            );
        }

        #[test]
        fn invoke_submits_deduped_repeated_readonly_account() {
            let program = Address::from([9u8; 32]);
            let (_b, acct) = make_account(8, false, false);

            let mut cpi: DynCpi<4, 4> = DynCpi::new(&program);
            cpi.push_account(&acct, false, false).unwrap();
            cpi.push_account(&acct, false, false).unwrap();
            cpi.push_account(&acct, false, false).unwrap();

            // Three read-only metas of one account collapse to one info.
            assert_eq!(cpi.account_count(), 3);
            assert_eq!(cpi.info_count(), 1);
            // Off-chain the syscall is a no-op; Ok proves the dedup-aware
            // validation pipeline accepted the built instruction.
            assert_eq!(cpi.invoke(), Ok(()));
        }

        #[test]
        fn wide_dyn_cpi_exceeds_legacy_64_account_ceiling() {
            let program = Address::from([9u8; 32]);
            // 65 distinct accounts — one past the pre-SIMD-0339 static
            // ceiling of 64. Keep backings and views alive for the builder.
            let mut backings: std::vec::Vec<std::vec::Vec<u64>> = std::vec::Vec::new();
            let mut views: std::vec::Vec<AccountView<'static>> = std::vec::Vec::new();
            for i in 1..=65u8 {
                let (b, v) = make_account(i, false, false);
                backings.push(b);
                views.push(v);
            }

            let mut cpi: DynCpi<70, 4> = DynCpi::new(&program);
            for v in &views {
                cpi.push_account(v, false, false).unwrap();
            }

            assert_eq!(cpi.account_count(), 65);
            // All distinct, so no dedup shrinkage here — but the shape is
            // accepted, proving >64 account CPIs build and submit.
            assert_eq!(cpi.info_count(), 65);
            assert_eq!(cpi.invoke(), Ok(()));
        }
    }
}
