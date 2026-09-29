//! Token-2022 confidential-transfer instructions.
//!
//! The builders carry ciphertexts and ElGamal keys as opaque byte arrays:
//! a program that moves confidential balances receives them from its
//! caller (they are produced off chain with the account's keys) and hands
//! them to Token-2022 unchanged. Nothing here encrypts, decrypts, or
//! builds a proof.
//!
//! A zero-knowledge proof that an instruction needs lives in one of two
//! places, named by [`ProofLocation`]: another instruction of the same
//! transaction, addressed by its offset from the Token-2022 instruction
//! (the Instructions sysvar account is then part of the account list), or
//! a context-state account that the ZK ElGamal proof program verified
//! earlier. The account order follows the Token-2022 processor: the
//! instruction's own accounts, the Instructions sysvar once if any proof
//! is given by offset, each context-state account in proof order, the
//! authority, and the multisig signers.
//!
//! Instruction data is `[27][sub-discriminator][fields]` with the field
//! layouts of `spl-token-2022-interface`; the tests compare every builder
//! with that crate's constructors.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionAccount, Signer};
use crate::token::{
    authority_meta, require_authority_signed_direct, require_multisig_signers_direct, Invoke,
    TokenInstruction, TokenSink, Trailing,
};
use crate::ProgramResult;
use core::num::NonZeroI8;

/// The Token-2022 instruction that carries every confidential-transfer
/// sub-instruction.
pub const IX_CONFIDENTIAL_TRANSFER: u8 = 27;

/// An authenticated-encryption ciphertext of a balance
/// (`DecryptableBalance`, `PodAeCiphertext`).
pub const DECRYPTABLE_BALANCE_LEN: usize = 36;
/// A twisted-ElGamal ciphertext (`PodElGamalCiphertext`).
pub const ELGAMAL_CIPHERTEXT_LEN: usize = 64;
/// An ElGamal public key (`PodElGamalPubkey`).
pub const ELGAMAL_PUBKEY_LEN: usize = 32;

/// A balance ciphertext only the account's owner can decrypt.
pub type DecryptableBalance = [u8; DECRYPTABLE_BALANCE_LEN];
/// A twisted-ElGamal ciphertext.
pub type ElGamalCiphertext = [u8; ELGAMAL_CIPHERTEXT_LEN];
/// An ElGamal public key.
pub type ElGamalPubkey = [u8; ELGAMAL_PUBKEY_LEN];

/// Where a zero-knowledge proof is found.
#[derive(Clone, Copy)]
pub enum ProofLocation<'a> {
    /// In another instruction of this transaction, at this offset from
    /// the Token-2022 instruction.
    InstructionOffset(NonZeroI8),
    /// In a context-state account the proof program verified earlier.
    ContextStateAccount(&'a AccountView<'a>),
}

impl ProofLocation<'_> {
    /// The offset byte the instruction data carries: the offset, or zero
    /// for a context-state account.
    #[inline(always)]
    pub const fn offset_byte(&self) -> u8 {
        match self {
            Self::InstructionOffset(offset) => offset.get() as u8,
            Self::ContextStateAccount(_) => 0,
        }
    }

    #[inline(always)]
    const fn is_offset(&self) -> bool {
        matches!(self, Self::InstructionOffset(_))
    }
}

/// The proof accounts of one instruction, in the order the processor
/// reads them: the Instructions sysvar once if any proof is given by
/// offset, then every context-state account in proof order.
struct ProofAccounts<'a> {
    views: [Option<&'a AccountView<'a>>; 6],
    len: usize,
}

impl<'a> ProofAccounts<'a> {
    #[inline(always)]
    fn gather<'x: 'a>(
        instructions_sysvar: Option<&'x AccountView<'x>>,
        proofs: &[ProofLocation<'x>],
    ) -> Result<Self, ProgramError> {
        let mut out = Self {
            views: [None; 6],
            len: 0,
        };
        if proofs.iter().any(ProofLocation::is_offset) {
            // A proof by offset is read through the Instructions sysvar;
            // without the account the CPI could not reach it.
            let sysvar = instructions_sysvar.ok_or(ProgramError::NotEnoughAccountKeys)?;
            out.views[0] = Some(sysvar);
            out.len = 1;
        }
        for proof in proofs {
            if let ProofLocation::ContextStateAccount(account) = proof {
                out.views[out.len] = Some(*account);
                out.len += 1;
            }
        }
        Ok(out)
    }

    /// The gathered views as a dense array and its length.
    #[inline(always)]
    fn dense(&self, fallback: &'a AccountView<'a>) -> ([&'a AccountView<'a>; 6], usize) {
        let mut dense = [fallback; 6];
        let mut i = 0;
        while i < self.len {
            if let Some(view) = self.views[i] {
                dense[i] = view;
            }
            i += 1;
        }
        (dense, self.len)
    }
}

/// Byte-exact encoders, shared by the builders and the tests.
pub mod encoders {
    use super::*;

    #[inline(always)]
    fn nullable_32(out: &mut [u8], value: Option<&[u8; 32]>) -> ProgramResult {
        match value {
            Some(value) if value == &[0u8; 32] => Err(ProgramError::InvalidArgument),
            Some(value) => {
                out.copy_from_slice(value);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// `[27][0][authority: nullable 32][auto_approve][auditor key: nullable 32]`.
    #[inline(always)]
    pub fn encode_initialize_mint(
        authority: Option<&Address>,
        auto_approve_new_accounts: bool,
        auditor_elgamal_pubkey: Option<&ElGamalPubkey>,
    ) -> Result<[u8; 67], ProgramError> {
        let mut data = [0u8; 67];
        data[0] = IX_CONFIDENTIAL_TRANSFER;
        data[1] = 0;
        nullable_32(&mut data[2..34], authority.map(|a| a.as_array()))?;
        data[34] = u8::from(auto_approve_new_accounts);
        nullable_32(&mut data[35..67], auditor_elgamal_pubkey)?;
        Ok(data)
    }

    /// `[27][1][auto_approve][auditor key: nullable 32]`.
    #[inline(always)]
    pub fn encode_update_mint(
        auto_approve_new_accounts: bool,
        auditor_elgamal_pubkey: Option<&ElGamalPubkey>,
    ) -> Result<[u8; 35], ProgramError> {
        let mut data = [0u8; 35];
        data[0] = IX_CONFIDENTIAL_TRANSFER;
        data[1] = 1;
        data[2] = u8::from(auto_approve_new_accounts);
        nullable_32(&mut data[3..35], auditor_elgamal_pubkey)?;
        Ok(data)
    }

    /// `[27][2][decryptable zero balance: 36][max pending credits: u64][proof offset]`.
    #[inline(always)]
    pub fn encode_configure_account(
        decryptable_zero_balance: &DecryptableBalance,
        maximum_pending_balance_credit_counter: u64,
        proof_offset: u8,
    ) -> [u8; 47] {
        let mut data = [0u8; 47];
        data[0] = IX_CONFIDENTIAL_TRANSFER;
        data[1] = 2;
        data[2..38].copy_from_slice(decryptable_zero_balance);
        data[38..46].copy_from_slice(&maximum_pending_balance_credit_counter.to_le_bytes());
        data[46] = proof_offset;
        data
    }

    /// `[27][sub]`: approve account (3), the four credit toggles (9 to
    /// 12), configure with registry (14).
    #[inline(always)]
    pub const fn encode_bare(sub: u8) -> [u8; 2] {
        [IX_CONFIDENTIAL_TRANSFER, sub]
    }

    /// `[27][4][proof offset]`.
    #[inline(always)]
    pub const fn encode_empty_account(proof_offset: u8) -> [u8; 3] {
        [IX_CONFIDENTIAL_TRANSFER, 4, proof_offset]
    }

    /// `[27][5][amount: u64][decimals]`.
    #[inline(always)]
    pub fn encode_deposit(amount: u64, decimals: u8) -> [u8; 11] {
        let mut data = [0u8; 11];
        data[0] = IX_CONFIDENTIAL_TRANSFER;
        data[1] = 5;
        data[2..10].copy_from_slice(&amount.to_le_bytes());
        data[10] = decimals;
        data
    }

    /// `[27][6][amount: u64][decimals][new decryptable balance: 36][equality offset][range offset]`.
    #[inline(always)]
    pub fn encode_withdraw(
        amount: u64,
        decimals: u8,
        new_decryptable_available_balance: &DecryptableBalance,
        equality_proof_offset: u8,
        range_proof_offset: u8,
    ) -> [u8; 49] {
        let mut data = [0u8; 49];
        data[0] = IX_CONFIDENTIAL_TRANSFER;
        data[1] = 6;
        data[2..10].copy_from_slice(&amount.to_le_bytes());
        data[10] = decimals;
        data[11..47].copy_from_slice(new_decryptable_available_balance);
        data[47] = equality_proof_offset;
        data[48] = range_proof_offset;
        data
    }

    /// `[27][sub][new source decryptable balance: 36][auditor lo: 64][auditor hi: 64][offsets]`
    /// with three offsets for `Transfer` (7) and five for
    /// `TransferWithFee` (13).
    #[inline(always)]
    pub fn encode_transfer<const N: usize>(
        sub: u8,
        new_source_decryptable_available_balance: &DecryptableBalance,
        transfer_amount_auditor_ciphertext_lo: &ElGamalCiphertext,
        transfer_amount_auditor_ciphertext_hi: &ElGamalCiphertext,
        proof_offsets: [u8; N],
    ) -> ([u8; 171], usize) {
        let mut data = [0u8; 171];
        data[0] = IX_CONFIDENTIAL_TRANSFER;
        data[1] = sub;
        data[2..38].copy_from_slice(new_source_decryptable_available_balance);
        data[38..102].copy_from_slice(transfer_amount_auditor_ciphertext_lo);
        data[102..166].copy_from_slice(transfer_amount_auditor_ciphertext_hi);
        let mut i = 0;
        while i < N && i < 5 {
            data[166 + i] = proof_offsets[i];
            i += 1;
        }
        (data, 166 + i)
    }

    /// `[27][8][expected pending credits: u64][new decryptable balance: 36]`.
    #[inline(always)]
    pub fn encode_apply_pending_balance(
        expected_pending_balance_credit_counter: u64,
        new_decryptable_available_balance: &DecryptableBalance,
    ) -> [u8; 46] {
        let mut data = [0u8; 46];
        data[0] = IX_CONFIDENTIAL_TRANSFER;
        data[1] = 8;
        data[2..10].copy_from_slice(&expected_pending_balance_credit_counter.to_le_bytes());
        data[10..46].copy_from_slice(new_decryptable_available_balance);
        data
    }
}

use encoders::*;

/// Token-2022 entry points for a builder with an authority.
macro_rules! confidential_methods {
    ($name:ident, authority = $auth:ident) => {
        impl $name<'_> {
            /// Send to Token-2022 with the authority signed directly.
            #[inline]
            pub fn invoke(&self) -> ProgramResult {
                require_authority_signed_direct(self.$auth)?;
                self.emit(&[], &mut Invoke::token_2022(&[]))
            }

            /// Send to Token-2022 with PDA signers.
            #[inline]
            pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
                self.emit(&[], &mut Invoke::token_2022(signers))
            }

            /// Send to Token-2022 with a multisig authority whose signers
            /// signed directly.
            #[inline]
            pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
                require_multisig_signers_direct(multisig_signers)?;
                self.emit(multisig_signers, &mut Invoke::token_2022(&[]))
            }

            /// Send to Token-2022 with a multisig authority and PDA
            /// signers.
            #[inline]
            pub fn invoke_signed_multisig(
                &self,
                multisig_signers: &[&AccountView<'_>],
                signers: &[Signer<'_, '_>],
            ) -> ProgramResult {
                self.emit(multisig_signers, &mut Invoke::token_2022(signers))
            }
        }
    };
}

// ---------------------------------------------------------------------
// Mint configuration

/// `InitializeMint` (27/0): enable confidential transfers on a
/// not-yet-initialized mint. Runs before `InitializeMint2`.
pub struct InitializeConfidentialTransferMint<'a> {
    pub mint: &'a AccountView<'a>,
    pub authority: Option<&'a Address>,
    pub auto_approve_new_accounts: bool,
    pub auditor_elgamal_pubkey: Option<&'a ElGamalPubkey>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeConfidentialTransferMint<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_initialize_mint(
            self.authority,
            self.auto_approve_new_accounts,
            self.auditor_elgamal_pubkey,
        )?;
        let accounts = [InstructionAccount::writable(self.mint.address())];
        let views = [self.mint];
        sink.emit(&data, accounts, views, &[])
    }
}

impl InitializeConfidentialTransferMint<'_> {
    /// Send to Token-2022.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::token_2022(&[]))
    }
}

/// `UpdateMint` (27/1): change the auto-approve policy and the auditor.
pub struct UpdateConfidentialTransferMint<'a> {
    pub mint: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub auto_approve_new_accounts: bool,
    pub auditor_elgamal_pubkey: Option<&'a ElGamalPubkey>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for UpdateConfidentialTransferMint<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_update_mint(self.auto_approve_new_accounts, self.auditor_elgamal_pubkey)?;
        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.mint, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

confidential_methods!(UpdateConfidentialTransferMint, authority = authority);

// ---------------------------------------------------------------------
// Instructions without a proof

/// An instruction over a writable token account, optionally its mint, and
/// the authority, with fixed data and no proof.
macro_rules! account_instruction {
    ($(#[$doc:meta])* $name:ident { account = $account:ident $(, @mint $mint:ident)? $(, $field:ident : $ty:ty)* $(,)? } data = |$s:ident| $data:expr;) => {
        $(#[$doc])*
        pub struct $name<'a> {
            pub $account: &'a AccountView<'a>,
            $(pub $mint: &'a AccountView<'a>,)?
            pub authority: &'a AccountView<'a>,
            $(pub $field: $ty,)*
        }

        impl<'a, 'x: 'a> TokenInstruction<'a> for $name<'x> {
            #[inline(always)]
            fn emit(
                &self,
                multisig_signers: &[&'a AccountView<'a>],
                sink: &mut impl TokenSink<'a>,
            ) -> ProgramResult {
                let $s = self;
                let data = $data;
                let accounts = [
                    InstructionAccount::writable(self.$account.address()),
                    $(InstructionAccount::readonly(self.$mint.address()),)?
                    authority_meta(self.authority, multisig_signers),
                ];
                let views = [self.$account, $(self.$mint,)? self.authority];
                sink.emit(&data, accounts, views, &[Trailing::signers(multisig_signers)])
            }
        }

        confidential_methods!($name, authority = authority);
    };
}

account_instruction! {
    /// `ApproveAccount` (27/3): the mint's confidential-transfer authority
    /// approves a configured account.
    ApproveConfidentialAccount { account = account, @mint mint }
    data = |_s| encode_bare(3);
}

account_instruction! {
    /// `Deposit` (27/5): move `amount` from the account's public balance
    /// into its pending confidential balance.
    ConfidentialDeposit { account = account, @mint mint, amount: u64, decimals: u8 }
    data = |s| encode_deposit(s.amount, s.decimals);
}

account_instruction! {
    /// `ApplyPendingBalance` (27/8): fold the pending balance into the
    /// available balance.
    ApplyPendingConfidentialBalance {
        account = account,
        expected_pending_balance_credit_counter: u64,
        new_decryptable_available_balance: &'a DecryptableBalance,
    }
    data = |s| encode_apply_pending_balance(
        s.expected_pending_balance_credit_counter,
        s.new_decryptable_available_balance,
    );
}

account_instruction! {
    /// `EnableConfidentialCredits` (27/9).
    EnableConfidentialCredits { account = account }
    data = |_s| encode_bare(9);
}

account_instruction! {
    /// `DisableConfidentialCredits` (27/10).
    DisableConfidentialCredits { account = account }
    data = |_s| encode_bare(10);
}

account_instruction! {
    /// `EnableNonConfidentialCredits` (27/11).
    EnableNonConfidentialCredits { account = account }
    data = |_s| encode_bare(11);
}

account_instruction! {
    /// `DisableNonConfidentialCredits` (27/12).
    DisableNonConfidentialCredits { account = account }
    data = |_s| encode_bare(12);
}

/// `ConfigureAccountWithRegistry` (27/14): configure a token account from
/// an ElGamal registry account. With a `payer` (and the System program),
/// Token-2022 reallocates the account and the payer funds it.
pub struct ConfigureConfidentialAccountWithRegistry<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub elgamal_registry: &'a AccountView<'a>,
    /// The payer and the System program account, when the account must
    /// grow.
    pub payer: Option<(&'a AccountView<'a>, &'a AccountView<'a>)>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for ConfigureConfidentialAccountWithRegistry<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_bare(14);
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.elgamal_registry.address()),
        ];
        let views = [self.account, self.mint, self.elgamal_registry];
        match self.payer {
            Some((payer, system_program)) => sink.emit(
                &data,
                accounts,
                views,
                &[
                    Trailing {
                        views: &[payer],
                        writable: true,
                        signer: true,
                    },
                    Trailing::readonly(&[system_program]),
                ],
            ),
            None => sink.emit(&data, accounts, views, &[]),
        }
    }
}

impl ConfigureConfidentialAccountWithRegistry<'_> {
    /// Send to Token-2022.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::token_2022(&[]))
    }

    /// Send to Token-2022 with PDA signers (a PDA payer).
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::token_2022(signers))
    }
}

// ---------------------------------------------------------------------
// Instructions that carry proofs

/// Emit an instruction whose fixed accounts are followed by the proof
/// accounts, the authority, and the multisig signers.
// One parameter per part of the instruction; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn emit_with_proofs<'a, 'x: 'a, const N: usize>(
    sink: &mut impl TokenSink<'a>,
    data: &[u8],
    accounts: [InstructionAccount<'a>; N],
    views: [&'a AccountView<'a>; N],
    instructions_sysvar: Option<&'x AccountView<'x>>,
    proofs: &[ProofLocation<'x>],
    authority: &'x AccountView<'x>,
    multisig_signers: &[&'a AccountView<'a>],
) -> ProgramResult {
    let gathered = ProofAccounts::gather(instructions_sysvar, proofs)?;
    let (dense, len) = gathered.dense(authority);
    let authority_run: [&'a AccountView<'a>; 1] = [authority];
    sink.emit(
        data,
        accounts,
        views,
        &[
            Trailing::readonly(&dense[..len]),
            Trailing {
                views: &authority_run,
                writable: false,
                signer: multisig_signers.is_empty(),
            },
            Trailing::signers(multisig_signers),
        ],
    )
}

/// `ConfigureAccount` (27/2): set a token account up for confidential
/// transfers. The proof is a public-key validity proof.
pub struct ConfigureConfidentialAccount<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub decryptable_zero_balance: &'a DecryptableBalance,
    pub maximum_pending_balance_credit_counter: u64,
    pub proof: ProofLocation<'a>,
    /// Required when the proof is given by instruction offset.
    pub instructions_sysvar: Option<&'a AccountView<'a>>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for ConfigureConfidentialAccount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_configure_account(
            self.decryptable_zero_balance,
            self.maximum_pending_balance_credit_counter,
            self.proof.offset_byte(),
        );
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
        ];
        emit_with_proofs(
            sink,
            &data,
            accounts,
            [self.account, self.mint],
            self.instructions_sysvar,
            &[self.proof],
            self.authority,
            multisig_signers,
        )
    }
}

confidential_methods!(ConfigureConfidentialAccount, authority = authority);

/// `EmptyAccount` (27/4): prove the confidential balance is zero so the
/// account can be closed. The proof is a zero-ciphertext proof.
pub struct EmptyConfidentialAccount<'a> {
    pub account: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub proof: ProofLocation<'a>,
    /// Required when the proof is given by instruction offset.
    pub instructions_sysvar: Option<&'a AccountView<'a>>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for EmptyConfidentialAccount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_empty_account(self.proof.offset_byte());
        let accounts = [InstructionAccount::writable(self.account.address())];
        emit_with_proofs(
            sink,
            &data,
            accounts,
            [self.account],
            self.instructions_sysvar,
            &[self.proof],
            self.authority,
            multisig_signers,
        )
    }
}

confidential_methods!(EmptyConfidentialAccount, authority = authority);

/// `Withdraw` (27/6): move `amount` from the confidential balance to the
/// public balance. Proofs: ciphertext-commitment equality, then range.
pub struct ConfidentialWithdraw<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: u64,
    pub decimals: u8,
    pub new_decryptable_available_balance: &'a DecryptableBalance,
    pub equality_proof: ProofLocation<'a>,
    pub range_proof: ProofLocation<'a>,
    /// Required when any proof is given by instruction offset.
    pub instructions_sysvar: Option<&'a AccountView<'a>>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for ConfidentialWithdraw<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_withdraw(
            self.amount,
            self.decimals,
            self.new_decryptable_available_balance,
            self.equality_proof.offset_byte(),
            self.range_proof.offset_byte(),
        );
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
        ];
        emit_with_proofs(
            sink,
            &data,
            accounts,
            [self.account, self.mint],
            self.instructions_sysvar,
            &[self.equality_proof, self.range_proof],
            self.authority,
            multisig_signers,
        )
    }
}

confidential_methods!(ConfidentialWithdraw, authority = authority);

/// `Transfer` (27/7): a confidential transfer. Proofs: equality,
/// ciphertext validity, range.
pub struct ConfidentialTransfer<'a> {
    pub source: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub new_source_decryptable_available_balance: &'a DecryptableBalance,
    pub transfer_amount_auditor_ciphertext_lo: &'a ElGamalCiphertext,
    pub transfer_amount_auditor_ciphertext_hi: &'a ElGamalCiphertext,
    pub equality_proof: ProofLocation<'a>,
    pub ciphertext_validity_proof: ProofLocation<'a>,
    pub range_proof: ProofLocation<'a>,
    /// Required when any proof is given by instruction offset.
    pub instructions_sysvar: Option<&'a AccountView<'a>>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for ConfidentialTransfer<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) = encode_transfer(
            7,
            self.new_source_decryptable_available_balance,
            self.transfer_amount_auditor_ciphertext_lo,
            self.transfer_amount_auditor_ciphertext_hi,
            [
                self.equality_proof.offset_byte(),
                self.ciphertext_validity_proof.offset_byte(),
                self.range_proof.offset_byte(),
            ],
        );
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::writable(self.destination.address()),
        ];
        emit_with_proofs(
            sink,
            &data[..len],
            accounts,
            [self.source, self.mint, self.destination],
            self.instructions_sysvar,
            &[
                self.equality_proof,
                self.ciphertext_validity_proof,
                self.range_proof,
            ],
            self.authority,
            multisig_signers,
        )
    }
}

confidential_methods!(ConfidentialTransfer, authority = authority);

/// `TransferWithFee` (27/13): a confidential transfer on a mint with a
/// transfer fee. Proofs: equality, transfer-amount ciphertext validity,
/// fee sigma, fee ciphertext validity, range.
pub struct ConfidentialTransferWithFee<'a> {
    pub source: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub new_source_decryptable_available_balance: &'a DecryptableBalance,
    pub transfer_amount_auditor_ciphertext_lo: &'a ElGamalCiphertext,
    pub transfer_amount_auditor_ciphertext_hi: &'a ElGamalCiphertext,
    pub equality_proof: ProofLocation<'a>,
    pub transfer_amount_ciphertext_validity_proof: ProofLocation<'a>,
    pub fee_sigma_proof: ProofLocation<'a>,
    pub fee_ciphertext_validity_proof: ProofLocation<'a>,
    pub range_proof: ProofLocation<'a>,
    /// Required when any proof is given by instruction offset.
    pub instructions_sysvar: Option<&'a AccountView<'a>>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for ConfidentialTransferWithFee<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) = encode_transfer(
            13,
            self.new_source_decryptable_available_balance,
            self.transfer_amount_auditor_ciphertext_lo,
            self.transfer_amount_auditor_ciphertext_hi,
            [
                self.equality_proof.offset_byte(),
                self.transfer_amount_ciphertext_validity_proof.offset_byte(),
                self.fee_sigma_proof.offset_byte(),
                self.fee_ciphertext_validity_proof.offset_byte(),
                self.range_proof.offset_byte(),
            ],
        );
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::writable(self.destination.address()),
        ];
        emit_with_proofs(
            sink,
            &data[..len],
            accounts,
            [self.source, self.mint, self.destination],
            self.instructions_sysvar,
            &[
                self.equality_proof,
                self.transfer_amount_ciphertext_validity_proof,
                self.fee_sigma_proof,
                self.fee_ciphertext_validity_proof,
                self.range_proof,
            ],
            self.authority,
            multisig_signers,
        )
    }
}

confidential_methods!(ConfidentialTransferWithFee, authority = authority);

#[cfg(test)]
mod tests {
    use super::encoders::*;
    use super::*;

    #[test]
    fn fixed_layouts_have_the_interface_sizes() {
        assert_eq!(encode_bare(9), [27, 9]);
        assert_eq!(encode_empty_account(3), [27, 4, 3]);
        assert_eq!(encode_deposit(1, 6), [27, 5, 1, 0, 0, 0, 0, 0, 0, 0, 6]);
        let balance = [9u8; DECRYPTABLE_BALANCE_LEN];
        let configure = encode_configure_account(&balance, 65_536, 1);
        assert_eq!(&configure[..2], &[27, 2]);
        assert_eq!(&configure[2..38], &balance);
        assert_eq!(&configure[38..46], &65_536u64.to_le_bytes());
        assert_eq!(configure[46], 1);
        let withdraw = encode_withdraw(5, 2, &balance, 0, 0xff);
        assert_eq!(withdraw.len(), 49);
        assert_eq!(&withdraw[47..], &[0, 0xff]);
        let apply = encode_apply_pending_balance(4, &balance);
        assert_eq!(&apply[2..10], &4u64.to_le_bytes());
        let lo = [1u8; ELGAMAL_CIPHERTEXT_LEN];
        let hi = [2u8; ELGAMAL_CIPHERTEXT_LEN];
        let (transfer, len) = encode_transfer(7, &balance, &lo, &hi, [1, 2, 3]);
        assert_eq!(len, 169);
        assert_eq!(&transfer[166..169], &[1, 2, 3]);
        let (with_fee, len) = encode_transfer(13, &balance, &lo, &hi, [1, 2, 3, 4, 5]);
        assert_eq!(len, 171);
        assert_eq!(&with_fee[166..171], &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_present_zero_key_is_refused_and_an_absent_one_is_zero() {
        let key = [4u8; ELGAMAL_PUBKEY_LEN];
        let authority = Address::new_from_array([3; 32]);
        let data = encode_initialize_mint(Some(&authority), true, Some(&key)).unwrap();
        assert_eq!(&data[2..34], &[3u8; 32]);
        assert_eq!(data[34], 1);
        assert_eq!(&data[35..], &key);
        let data = encode_initialize_mint(None, false, None).unwrap();
        assert_eq!(&data[2..], &[0u8; 65]);
        assert!(encode_update_mint(true, Some(&[0u8; 32])).is_err());
        assert!(
            encode_initialize_mint(Some(&Address::new_from_array([0; 32])), true, None).is_err()
        );
    }

    #[test]
    fn a_negative_offset_is_its_twos_complement_byte() {
        let back = ProofLocation::InstructionOffset(NonZeroI8::new(-1).unwrap());
        assert_eq!(back.offset_byte(), 0xff);
        let next = ProofLocation::InstructionOffset(NonZeroI8::new(2).unwrap());
        assert_eq!(next.offset_byte(), 2);
    }
}
