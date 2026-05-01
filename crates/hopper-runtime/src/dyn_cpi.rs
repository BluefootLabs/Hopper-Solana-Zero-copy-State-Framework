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

use crate::{
    account::AccountView,
    address::Address,
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
    accounts: [MaybeUninit<&'a AccountView>; MAX_ACCTS],
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
        account: &'a AccountView,
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
        unsafe {
            core::slice::from_raw_parts(
                self.data.as_ptr() as *const u8,
                self.data_len,
            )
        }
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
        assert_eq!(
            &cpi.data()[1..9],
            &0xCAFEBABE_u64.to_le_bytes()
        );
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
}
