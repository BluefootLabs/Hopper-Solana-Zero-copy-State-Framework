//! `Batch` (255): several token instructions in one CPI.
//!
//! The p-token build of SPL Token accepts one instruction whose data is
//! `[255]` followed by, per inner instruction, a two-byte header
//! (`[account count][data length]`) and that instruction's data; the
//! accounts of every inner instruction follow each other in the account
//! list. One CPI then pays one invocation overhead instead of one per
//! instruction.
//!
//! [`TokenBatch`] is a [`TokenSink`]: builders in [`crate::token`] or
//! [`crate::token_2022_ix`] are appended with [`TokenBatch::push`] through
//! the same `emit` that its `invoke()` uses, so the batched bytes and the
//! single-CPI bytes are identical by construction. The buffers are const
//! generic and live on the stack; nothing is allocated.
//! Each inner instruction must fit the wire format's 255-byte data and
//! 255-account limits as well as the batch's capacities. A larger payload
//! must be sent separately.
//!
//! One account may appear in several inner instructions (a transfer there
//! and back); the batch is sent through
//! [`crate::cpi::invoke_signed_batch_with_bounds`], which keeps every
//! per-meta check of the default tier and only waives the refusal of one
//! account behind two writable metas, since for a batch that repeat is the
//! contract rather than the footgun.
//! Within an inner instruction, repeated writable accounts are still
//! refused with `AccountBorrowFailed` (including self-transfers).
//!
//! Both programs accepted a batch of two `TransferChecked`s on devnet on
//! 2026-09-28 (SPL Token at 2,472 CU for the whole instruction, Token-2022
//! at 5,575); the Token-2022 build bundled with Mollusk 0.15 refuses
//! discriminator 255 with `InvalidInstruction`, so a local test cannot
//! stand in for the cluster on that point.

use crate::account::AccountView;
use crate::error::ProgramError;
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::token::{
    TokenInstruction, TokenProgram, TokenSink, Trailing, MAX_TOKEN_MULTISIG_SIGNERS,
};
use crate::ProgramResult;
use core::mem::MaybeUninit;

/// The `Batch` discriminator.
pub const BATCH_DISCRIMINATOR: u8 = 255;

/// Bytes of header in front of each inner instruction: its account count
/// and its data length, one byte each.
pub const BATCH_INSTRUCTION_HEADER_LEN: usize = 2;

/// A stack-resident batch of token instructions.
///
/// `DATA` bounds the instruction data (one discriminator byte plus the
/// headers and data of every pushed instruction) and `ACCOUNTS` bounds
/// the account list. A push that would overflow either is refused with
/// `InvalidArgument` and leaves the batch unchanged.
pub struct TokenBatch<'a, const DATA: usize = 256, const ACCOUNTS: usize = 16> {
    data: [MaybeUninit<u8>; DATA],
    data_len: usize,
    accounts: [MaybeUninit<InstructionAccount<'a>>; ACCOUNTS],
    views: [MaybeUninit<&'a AccountView<'a>>; ACCOUNTS],
    accounts_len: usize,
    instructions: usize,
}

impl<'a, const DATA: usize, const ACCOUNTS: usize> Default for TokenBatch<'a, DATA, ACCOUNTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, const DATA: usize, const ACCOUNTS: usize> TokenBatch<'a, DATA, ACCOUNTS> {
    /// An empty batch. `DATA` must hold at least the discriminator.
    pub const fn new() -> Self {
        const {
            assert!(DATA >= 1, "a TokenBatch needs room for its discriminator");
            assert!(
                ACCOUNTS <= crate::cpi::MAX_STATIC_CPI_ACCOUNTS,
                "a TokenBatch cannot carry more accounts than one CPI"
            );
        }
        let mut data = [MaybeUninit::uninit(); DATA];
        data[0] = MaybeUninit::new(BATCH_DISCRIMINATOR);
        Self {
            data,
            data_len: 1,
            accounts: [MaybeUninit::uninit(); ACCOUNTS],
            views: [MaybeUninit::uninit(); ACCOUNTS],
            accounts_len: 0,
            instructions: 0,
        }
    }

    /// Append an instruction whose authority signs directly or through
    /// PDA seeds given at invoke time.
    #[inline]
    pub fn push(&mut self, instruction: &impl TokenInstruction<'a>) -> ProgramResult {
        self.push_multisig(instruction, &[])
    }

    /// Append an instruction whose authority is a multisig with these
    /// signer accounts.
    #[inline]
    pub fn push_multisig(
        &mut self,
        instruction: &impl TokenInstruction<'a>,
        multisig_signers: &[&'a AccountView<'a>],
    ) -> ProgramResult {
        // TokenInstruction is open: a custom encoder can emit more than
        // once, or fail after emitting. Keep the whole push transactional.
        let checkpoint = (self.data_len, self.accounts_len, self.instructions);
        let result = instruction.emit(multisig_signers, self);
        if result.is_err() {
            (self.data_len, self.accounts_len, self.instructions) = checkpoint;
        }
        result
    }

    /// How many instructions were pushed.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.instructions
    }

    /// Whether nothing was pushed yet.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.instructions == 0
    }

    /// The instruction data as it will be sent: `[255]` then the headers
    /// and inner data.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        // SAFETY: bytes in 0..data_len were written by `new` (the
        // discriminator) and by `emit` (each header and inner data).
        unsafe { core::slice::from_raw_parts(self.data.as_ptr() as *const u8, self.data_len) }
    }

    /// The account metas as they will be sent.
    #[inline(always)]
    pub fn account_metas(&self) -> &[InstructionAccount<'a>] {
        // SAFETY: slots in 0..accounts_len were written by `emit`.
        unsafe {
            core::slice::from_raw_parts(
                self.accounts.as_ptr() as *const InstructionAccount<'a>,
                self.accounts_len,
            )
        }
    }

    /// The account views as they will be sent, in meta order.
    #[inline(always)]
    pub fn account_views(&self) -> &[&'a AccountView<'a>] {
        // SAFETY: mirrors `account_metas`; every slot in 0..accounts_len
        // was written by `emit`.
        unsafe {
            core::slice::from_raw_parts(
                self.views.as_ptr() as *const &'a AccountView<'a>,
                self.accounts_len,
            )
        }
    }

    /// Send the batch to SPL Token.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_on(TokenProgram::Legacy, &[])
    }

    /// Send the batch to SPL Token with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_on(TokenProgram::Legacy, signers)
    }

    /// Send the batch to an explicit token program. An empty batch is
    /// refused with `InvalidArgument` rather than sent.
    #[inline]
    pub fn invoke_on(&self, program: TokenProgram, signers: &[Signer<'_, '_>]) -> ProgramResult {
        if self.instructions == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        let instruction = InstructionView {
            program_id: program.address(),
            data: self.data(),
            accounts: self.account_metas(),
        };
        crate::cpi::invoke_signed_batch_with_bounds::<{ crate::cpi::MAX_STATIC_CPI_ACCOUNTS }>(
            &instruction,
            self.account_views(),
            signers,
        )
    }
}

impl<'a, const DATA: usize, const ACCOUNTS: usize> TokenSink<'a>
    for TokenBatch<'a, DATA, ACCOUNTS>
{
    #[inline]
    fn emit<const N: usize>(
        &mut self,
        data: &[u8],
        accounts: [InstructionAccount<'a>; N],
        views: [&'a AccountView<'a>; N],
        trailing: &[Trailing<'_, 'a>],
    ) -> ProgramResult {
        let mut count = N;
        for run in trailing {
            if run.signer && run.views.len() > MAX_TOKEN_MULTISIG_SIGNERS {
                return Err(ProgramError::InvalidArgument);
            }
            count = count
                .checked_add(run.views.len())
                .ok_or(ProgramError::ArithmeticOverflow)?;
        }
        let Some(new_data_len) = self
            .data_len
            .checked_add(BATCH_INSTRUCTION_HEADER_LEN)
            .and_then(|at| at.checked_add(data.len()))
        else {
            return Err(ProgramError::ArithmeticOverflow);
        };
        let Some(new_accounts_len) = self.accounts_len.checked_add(count) else {
            return Err(ProgramError::ArithmeticOverflow);
        };
        if count > u8::MAX as usize
            || data.len() > u8::MAX as usize
            || new_data_len > DATA
            || new_accounts_len > ACCOUNTS
        {
            return Err(ProgramError::InvalidArgument);
        }

        let at = self.data_len;
        self.data[at].write(count as u8);
        self.data[at + 1].write(data.len() as u8);
        for (slot, byte) in self.data[at + BATCH_INSTRUCTION_HEADER_LEN..new_data_len]
            .iter_mut()
            .zip(data)
        {
            slot.write(*byte);
        }

        let mut index = self.accounts_len;
        for i in 0..N {
            self.accounts[index].write(accounts[i]);
            self.views[index].write(views[i]);
            index += 1;
        }
        for run in trailing {
            for view in run.views {
                self.accounts[index].write(InstructionAccount::new(
                    view.address(),
                    run.writable,
                    run.signer,
                ));
                self.views[index].write(*view);
                index += 1;
            }
        }

        // All new slots are initialized, so the slice accessors may expose
        // them for validation. Compare only this inner instruction: an
        // account may legitimately occur again in the next instruction.
        let previous_accounts_len = self.accounts_len;
        self.accounts_len = new_accounts_len;
        let instruction = InstructionView {
            program_id: TokenProgram::Legacy.address(), // irrelevant to alias validation
            data,
            accounts: &self.account_metas()[previous_accounts_len..],
        };
        if let Err(error) = crate::cpi::validate_no_duplicate_writable(
            &instruction,
            &self.account_views()[previous_accounts_len..],
        ) {
            self.accounts_len = previous_accounts_len;
            return Err(error);
        }
        self.data_len = new_data_len;
        self.instructions += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;
    use crate::token::{CloseAccount, TransferChecked, TOKEN_PROGRAM_ID};
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };

    fn make_account(address: [u8; 32], signer: bool) -> (std::vec::Vec<u64>, AccountView<'static>) {
        let mut backing = std::vec![0u64; RuntimeAccount::SIZE.div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: test helper writes a valid RuntimeAccount header into owned
        // backing memory that outlives the view (leaked below).
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: u8::from(signer),
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array(address),
                owner: NativeAddress::new_from_array(TOKEN_PROGRAM_ID.to_bytes()),
                lamports: 1,
                data_len: 0,
            });
        }
        // SAFETY: `raw` points at the initialized RuntimeAccount header.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }

    #[test]
    fn a_batch_lays_out_headers_data_and_accounts_in_push_order() {
        let (_b1, from) = make_account([1; 32], false);
        let (_b2, mint) = make_account([2; 32], false);
        let (_b3, to) = make_account([3; 32], false);
        let (_b4, authority) = make_account([4; 32], true);
        let (_b5, destination) = make_account([5; 32], false);
        let from = &from;
        let mint = &mint;
        let to = &to;
        let authority = &authority;
        let destination = &destination;

        let mut batch = TokenBatch::<128, 8>::new();
        assert!(batch.is_empty());
        assert!(batch.invoke().is_err(), "an empty batch is refused");
        batch
            .push(&TransferChecked {
                from,
                mint,
                to,
                authority,
                amount: 5,
                decimals: 2,
            })
            .unwrap();
        batch
            .push(&CloseAccount {
                account: from,
                destination,
                authority,
            })
            .unwrap();
        assert_eq!(batch.len(), 2);

        let mut expected = std::vec![255u8];
        expected.extend_from_slice(&[4, 10, 12, 5, 0, 0, 0, 0, 0, 0, 0, 2]);
        expected.extend_from_slice(&[3, 1, 9]);
        assert_eq!(batch.data(), &expected[..]);

        let metas = batch.account_metas();
        assert_eq!(metas.len(), 7);
        let flags: std::vec::Vec<(u8, bool, bool)> = metas
            .iter()
            .map(|m| (m.address.as_array()[0], m.is_writable, m.is_signer))
            .collect();
        assert_eq!(
            flags,
            std::vec![
                (1, true, false),
                (2, false, false),
                (3, true, false),
                (4, false, true),
                (1, true, false),
                (5, true, false),
                (4, false, true),
            ]
        );
        assert_eq!(batch.account_views().len(), 7);
        assert_eq!(batch.account_views()[3].address(), authority.address());
    }

    #[test]
    fn a_push_that_overflows_leaves_the_batch_unchanged() {
        let (_b1, account) = make_account([1; 32], false);
        let (_b2, destination) = make_account([5; 32], false);
        let (_b3, authority) = make_account([4; 32], true);
        let close = CloseAccount {
            account: &account,
            destination: &destination,
            authority: &authority,
        };
        let mut small = TokenBatch::<3, 8>::new();
        assert_eq!(small.push(&close), Err(ProgramError::InvalidArgument));
        assert!(small.is_empty());
        assert_eq!(small.data(), &[255]);

        let mut few = TokenBatch::<64, 2>::new();
        assert_eq!(few.push(&close), Err(ProgramError::InvalidArgument));
        assert_eq!(few.account_metas().len(), 0);
    }

    #[test]
    fn a_batch_refuses_a_self_transfer_but_reuses_accounts_between_instructions() {
        let (_b1, from) = make_account([1; 32], false);
        let (_b2, mint) = make_account([2; 32], false);
        let (_b3, to) = make_account([3; 32], false);
        let (_b4, authority) = make_account([4; 32], true);
        let mut batch = TokenBatch::<64, 12>::new();
        let mut transfer = TransferChecked {
            from: &from,
            mint: &mint,
            to: &to,
            authority: &authority,
            amount: 5,
            decimals: 2,
        };
        batch.push(&transfer).unwrap();
        let before = batch.data().to_vec();
        transfer.to = &from;
        assert_eq!(
            batch.push(&transfer),
            Err(ProgramError::AccountBorrowFailed)
        );
        assert_eq!(batch.data(), before);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.account_metas().len(), 4);
        transfer.from = &to;
        batch.push(&transfer).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.account_metas().len(), 8);
    }

    #[test]
    fn custom_instruction_failure_rolls_back_all_emitted_instructions() {
        struct Partial;
        impl<'a> TokenInstruction<'a> for Partial {
            fn emit(
                &self,
                _: &[&'a AccountView<'a>],
                sink: &mut impl TokenSink<'a>,
            ) -> ProgramResult {
                sink.emit(&[17], [], [], &[])?;
                Err(ProgramError::InvalidArgument)
            }
        }
        for multisig in [false, true] {
            let mut batch = TokenBatch::<16, 0>::new();
            TokenSink::emit(&mut batch, &[20], [], [], &[]).unwrap();
            let before = batch.data().to_vec();
            let result = if multisig {
                batch.push_multisig(&Partial, &[])
            } else {
                batch.push(&Partial)
            };
            assert_eq!(result, Err(ProgramError::InvalidArgument));
            assert_eq!(batch.data(), before);
            assert_eq!(batch.len(), 1);
        }
    }

    #[test]
    fn writable_trailing_alias_is_refused_even_for_distinct_views() {
        let (_b1, from) = make_account([1; 32], false);
        let (_b2, alias) = make_account([1; 32], false);
        let mut batch = TokenBatch::<16, 2>::new();
        assert_eq!(
            TokenSink::emit(
                &mut batch,
                &[17],
                [InstructionAccount::writable(from.address())],
                [&from],
                &[Trailing::writable(&[&alias])]
            ),
            Err(ProgramError::AccountBorrowFailed)
        );
        assert!(batch.is_empty());
        assert_eq!(batch.data(), &[255]);
        assert!(batch.account_views().is_empty());
    }

    #[test]
    fn a_multisig_push_appends_the_signers_after_the_fixed_accounts() {
        let (_b1, account) = make_account([1; 32], false);
        let (_b2, destination) = make_account([5; 32], false);
        let (_b3, multisig) = make_account([6; 32], false);
        let (_b4, s1) = make_account([7; 32], true);
        let (_b5, s2) = make_account([8; 32], true);
        let close = CloseAccount {
            account: &account,
            destination: &destination,
            authority: &multisig,
        };
        let mut batch = TokenBatch::<64, 8>::new();
        batch.push_multisig(&close, &[&s1, &s2]).unwrap();
        assert_eq!(&batch.data()[1..3], &[5, 1]);
        let metas = batch.account_metas();
        assert_eq!(metas.len(), 5);
        assert!(!metas[2].is_signer, "a multisig authority is not a signer");
        assert!(metas[3].is_signer && metas[4].is_signer);
        let _ = Address::default();
    }
}
