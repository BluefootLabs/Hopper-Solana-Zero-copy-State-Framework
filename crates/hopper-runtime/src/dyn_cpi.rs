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
use crate::{account::AccountView, address::Address, error::ProgramError, result::ProgramResult};

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
    accounts: [MaybeUninit<&'a AccountView<'a>>; MAX_ACCTS],
    writable: [bool; MAX_ACCTS],
    signer: [bool; MAX_ACCTS],
    account_count: usize,
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
            data: [const { MaybeUninit::uninit() }; MAX_DATA],
            data_len: 0,
        }
    }

    /// Append one account meta. The `writable` and `signer` flags
    /// are carried through to the emitted CPI instruction.
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
        self.accounts[self.account_count] = MaybeUninit::new(account);
        self.writable[self.account_count] = writable;
        self.signer[self.account_count] = signer;
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

    /// Current account count.
    #[inline(always)]
    pub const fn account_count(&self) -> usize {
        self.account_count
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
    /// Assembles the pushed `(account, writable, signer)` metas and the
    /// data buffer into an [`InstructionView`] and routes it through the
    /// **validated** dynamic-count path
    /// ([`cpi::invoke_signed_with_bounds`](crate::cpi::invoke_signed_with_bounds)):
    /// address/flag agreement, PDA-signer resolution, live-borrow
    /// checks, and duplicate-writable rejection all run before the
    /// syscall. This is the typed signer threading the module docs
    /// promise — the builder and the submission are one method chain.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        let count = self.account_count;
        let mut metas: [MaybeUninit<InstructionAccount<'a>>; MAX_ACCTS] =
            [const { MaybeUninit::uninit() }; MAX_ACCTS];
        let views = self.account_views();
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
        crate::cpi::invoke_signed_with_bounds::<MAX_ACCTS>(&instruction, views, signers)
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
        ) -> (std::vec::Vec<u8>, AccountView<'static>) {
            let mut backing = std::vec![0u8; RuntimeAccount::SIZE + 8];
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
            assert_eq!(
                cpi.invoke(),
                Err(ProgramError::MissingRequiredSignature)
            );
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
    }
}
