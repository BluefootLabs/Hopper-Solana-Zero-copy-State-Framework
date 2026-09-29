//! Confidential lab: the Token-2022 confidential-transfer instructions
//! Hopper builds, one per instruction of this program, so a test can run the
//! whole flow against the real Token-2022 program and the ZK ElGamal proof
//! program: create a confidential mint, configure accounts with a public-key
//! validity proof, approve, deposit, apply the pending balance, withdraw
//! with an equality and a range proof, transfer with three proofs, toggle
//! credits, and empty an account with a zero-ciphertext proof. Two more
//! cover the rest: configuring an account from an ElGamal registry, and a
//! transfer on a mint that charges a fee, which takes five proofs.
//!
//! The program holds no key and builds no proof. Ciphertexts, keys, and
//! proofs come from the caller, who produced them off chain with the crates
//! a wallet uses. A proof is read either from a context-state account the
//! proof program wrote earlier, or from another instruction of the same
//! transaction: a nonzero `*_offset` argument selects the second, and the
//! proof account passed is then the Instructions sysvar.
#![cfg_attr(target_os = "solana", no_std)]
#![cfg_attr(not(target_os = "solana"), allow(dead_code))]

use core::num::NonZeroI8;
use hopper::prelude::*;
use hopper::token::{
    GetAccountDataSize, InitializeAccount3, InitializeMint2, MintConfig, TokenProgram,
};
use hopper::token_2022::confidential_instructions::{
    ApplyPendingConfidentialBalance, ApproveConfidentialAccount, ConfidentialDeposit,
    ConfidentialTransfer, ConfidentialTransferWithFee, ConfidentialWithdraw,
    ConfigureConfidentialAccount, ConfigureConfidentialAccountWithRegistry,
    DisableConfidentialCredits, DisableNonConfidentialCredits, EmptyConfidentialAccount,
    EnableConfidentialCredits, EnableNonConfidentialCredits, InitializeConfidentialTransferMint,
    ProofLocation, UpdateConfidentialTransferMint,
};

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

hopper::hopper_error! {
    base = 6900;
    NotToken2022,
    UnknownCreditToggle,
}

/// A mint with the confidential-transfer extension and nothing else: the
/// 82-byte base, padding to 165, the account-type byte, and the extension's
/// 4-byte TLV header and 65 bytes.
pub const CONFIDENTIAL_MINT_LEN: usize = 165 + 1 + 4 + 65;
/// Token-2022's TLV type of the account-side confidential-transfer state.
pub const EXT_CONFIDENTIAL_TRANSFER_ACCOUNT: u16 = 5;
pub const DECIMALS: u8 = 2;

/// The four credit toggles `credits` accepts.
pub const ENABLE_CONFIDENTIAL: u8 = 0;
pub const DISABLE_CONFIDENTIAL: u8 = 1;
pub const ENABLE_NON_CONFIDENTIAL: u8 = 2;
pub const DISABLE_NON_CONFIDENTIAL: u8 = 3;

fn token_2022(account: &AccountView<'_>) -> ProgramResult {
    account.check_executable()?;
    hopper::hopper_require!(
        TokenProgram::from_program_account(account)? == TokenProgram::Token2022,
        NotToken2022
    );
    Ok(())
}

/// Where a proof is: in the transaction at `offset` (the account is then
/// the Instructions sysvar), or in the context-state account.
fn locate<'a>(
    offset: i8,
    account: &'a AccountView<'a>,
) -> (ProofLocation<'a>, Option<&'a AccountView<'a>>) {
    match NonZeroI8::new(offset) {
        Some(offset) => (ProofLocation::InstructionOffset(offset), Some(account)),
        None => (ProofLocation::ContextStateAccount(account), None),
    }
}

#[derive(Accounts)]
pub struct CreateMint<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub mint: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MintAuthority<'info> {
    #[account(mut)]
    pub mint: UncheckedAccount<'info>,
    pub authority: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct OpenAccount<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub account: Signer<'info>,
    pub mint: UncheckedAccount<'info>,
    pub owner: Signer<'info>,
    pub proof: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct OpenFromRegistry<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub account: Signer<'info>,
    pub mint: UncheckedAccount<'info>,
    pub owner: UncheckedAccount<'info>,
    pub registry: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct TransferWithFee<'info> {
    #[account(mut)]
    pub source: UncheckedAccount<'info>,
    pub mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,
    pub equality_proof: UncheckedAccount<'info>,
    pub amount_validity_proof: UncheckedAccount<'info>,
    pub fee_sigma_proof: UncheckedAccount<'info>,
    pub fee_validity_proof: UncheckedAccount<'info>,
    pub range_proof: UncheckedAccount<'info>,
    pub owner: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Approve<'info> {
    #[account(mut)]
    pub account: UncheckedAccount<'info>,
    pub mint: UncheckedAccount<'info>,
    pub authority: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct WithMint<'info> {
    #[account(mut)]
    pub account: UncheckedAccount<'info>,
    pub mint: UncheckedAccount<'info>,
    pub owner: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Owned<'info> {
    #[account(mut)]
    pub account: UncheckedAccount<'info>,
    pub owner: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub account: UncheckedAccount<'info>,
    pub mint: UncheckedAccount<'info>,
    pub equality_proof: UncheckedAccount<'info>,
    pub range_proof: UncheckedAccount<'info>,
    pub owner: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Transfer<'info> {
    #[account(mut)]
    pub source: UncheckedAccount<'info>,
    pub mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,
    pub equality_proof: UncheckedAccount<'info>,
    pub validity_proof: UncheckedAccount<'info>,
    pub range_proof: UncheckedAccount<'info>,
    pub owner: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Empty<'info> {
    #[account(mut)]
    pub account: UncheckedAccount<'info>,
    pub proof: UncheckedAccount<'info>,
    pub owner: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[program]
mod confidential_lab {
    use super::*;

    /// A mint with confidential transfers enabled: the payer is the mint
    /// authority and the confidential-transfer authority. `auditor` is an
    /// ElGamal public key, or zero for none.
    #[instruction(0)]
    pub fn create_mint(ctx: Ctx<CreateMint>, auto_approve: u8, auditor: [u8; 32]) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let program = TokenProgram::Token2022;
        hopper::system::CreateAccount {
            from: a.payer.as_account(),
            to: a.mint.as_account(),
            lamports: hopper::hopper_runtime::rent::minimum_balance_live(CONFIDENTIAL_MINT_LEN)?,
            space: CONFIDENTIAL_MINT_LEN as u64,
            owner: program.address(),
        }
        .invoke()?;
        InitializeConfidentialTransferMint {
            mint: a.mint.as_account(),
            authority: Some(a.payer.key()),
            auto_approve_new_accounts: auto_approve != 0,
            auditor_elgamal_pubkey: (auditor != [0u8; 32]).then_some(&auditor),
        }
        .invoke()?;
        InitializeMint2 {
            mint: a.mint.as_account(),
            program,
            config: MintConfig {
                decimals: DECIMALS,
                mint_authority: a.payer.key(),
                freeze_authority: None,
            },
        }
        .invoke()
    }

    /// Change the auto-approve policy and the auditor key.
    #[instruction(1)]
    pub fn update_mint(
        ctx: Ctx<MintAuthority>,
        auto_approve: u8,
        auditor: [u8; 32],
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        UpdateConfidentialTransferMint {
            mint: a.mint.as_account(),
            authority: a.authority.as_account(),
            auto_approve_new_accounts: auto_approve != 0,
            auditor_elgamal_pubkey: (auditor != [0u8; 32]).then_some(&auditor),
        }
        .invoke()
    }

    /// A token account sized for the confidential extension (and
    /// `extra_extension`, when nonzero), initialized for `owner`, then
    /// configured: `zero_balance` is the owner's authenticated encryption
    /// of zero, the proof a public-key validity proof of the owner's
    /// ElGamal key.
    #[instruction(2)]
    pub fn open_account(
        ctx: Ctx<OpenAccount>,
        maximum_pending_credits: u64,
        zero_balance: [u8; 36],
        proof_offset: i8,
        extra_extension: u16,
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let program = TokenProgram::Token2022;
        let with_extra = [EXT_CONFIDENTIAL_TRANSFER_ACCOUNT, extra_extension];
        let size = GetAccountDataSize {
            mint: a.mint.as_account(),
            extension_types: if extra_extension == 0 {
                &with_extra[..1]
            } else {
                &with_extra
            },
        }
        .query(program)?;
        let space = usize::try_from(size).map_err(|_| ProgramError::InvalidAccountData)?;
        hopper::system::CreateAccount {
            from: a.payer.as_account(),
            to: a.account.as_account(),
            lamports: hopper::hopper_runtime::rent::minimum_balance_live(space)?,
            space: size,
            owner: program.address(),
        }
        .invoke()?;
        InitializeAccount3 {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            owner: a.owner.key(),
        }
        .invoke_on(program, &[], &[])?;
        let (proof, instructions_sysvar) = locate(proof_offset, a.proof.as_account());
        ConfigureConfidentialAccount {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            authority: a.owner.as_account(),
            decryptable_zero_balance: &zero_balance,
            maximum_pending_balance_credit_counter: maximum_pending_credits,
            proof,
            instructions_sysvar,
        }
        .invoke()
    }

    /// The confidential-transfer authority approves a configured account.
    #[instruction(3)]
    pub fn approve(ctx: Ctx<Approve>) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        ApproveConfidentialAccount {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            authority: a.authority.as_account(),
        }
        .invoke()
    }

    /// Move `amount` of the public balance into the pending confidential
    /// balance.
    #[instruction(4)]
    pub fn deposit(ctx: Ctx<WithMint>, amount: u64, decimals: u8) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        ConfidentialDeposit {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            authority: a.owner.as_account(),
            amount,
            decimals,
        }
        .invoke()
    }

    /// Fold the pending balance into the available balance. The caller
    /// sends the counter it expects and the new available balance under
    /// its authenticated-encryption key.
    #[instruction(5)]
    pub fn apply_pending(
        ctx: Ctx<Owned>,
        expected_pending_credits: u64,
        new_balance: [u8; 36],
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        ApplyPendingConfidentialBalance {
            account: a.account.as_account(),
            authority: a.owner.as_account(),
            expected_pending_balance_credit_counter: expected_pending_credits,
            new_decryptable_available_balance: &new_balance,
        }
        .invoke()
    }

    /// Move `amount` of the available confidential balance to the public
    /// balance.
    #[instruction(6)]
    pub fn withdraw(
        ctx: Ctx<Withdraw>,
        amount: u64,
        decimals: u8,
        new_balance: [u8; 36],
        equality_offset: i8,
        range_offset: i8,
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let (equality_proof, equality_sysvar) =
            locate(equality_offset, a.equality_proof.as_account());
        let (range_proof, range_sysvar) = locate(range_offset, a.range_proof.as_account());
        ConfidentialWithdraw {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            authority: a.owner.as_account(),
            amount,
            decimals,
            new_decryptable_available_balance: &new_balance,
            equality_proof,
            range_proof,
            instructions_sysvar: equality_sysvar.or(range_sysvar),
        }
        .invoke()
    }

    /// A confidential transfer. `auditor_lo` and `auditor_hi` are the
    /// auditor's ciphertexts of the amount's low 16 and high 32 bits.
    #[instruction(7)]
    pub fn transfer(
        ctx: Ctx<Transfer>,
        new_balance: [u8; 36],
        auditor_lo: [u8; 64],
        auditor_hi: [u8; 64],
        equality_offset: i8,
        validity_offset: i8,
        range_offset: i8,
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let (equality_proof, s1) = locate(equality_offset, a.equality_proof.as_account());
        let (ciphertext_validity_proof, s2) =
            locate(validity_offset, a.validity_proof.as_account());
        let (range_proof, s3) = locate(range_offset, a.range_proof.as_account());
        ConfidentialTransfer {
            source: a.source.as_account(),
            mint: a.mint.as_account(),
            destination: a.destination.as_account(),
            authority: a.owner.as_account(),
            new_source_decryptable_available_balance: &new_balance,
            transfer_amount_auditor_ciphertext_lo: &auditor_lo,
            transfer_amount_auditor_ciphertext_hi: &auditor_hi,
            equality_proof,
            ciphertext_validity_proof,
            range_proof,
            instructions_sysvar: s1.or(s2).or(s3),
        }
        .invoke()
    }

    /// One of the four credit toggles, by `which`.
    #[instruction(8)]
    pub fn credits(ctx: Ctx<Owned>, which: u8) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let (account, authority) = (a.account.as_account(), a.owner.as_account());
        match which {
            ENABLE_CONFIDENTIAL => EnableConfidentialCredits { account, authority }.invoke(),
            DISABLE_CONFIDENTIAL => DisableConfidentialCredits { account, authority }.invoke(),
            ENABLE_NON_CONFIDENTIAL => EnableNonConfidentialCredits { account, authority }.invoke(),
            DISABLE_NON_CONFIDENTIAL => {
                DisableNonConfidentialCredits { account, authority }.invoke()
            }
            _ => Err(UnknownCreditToggle.into()),
        }
    }

    /// Prove the available balance is zero so the account can be closed.
    #[instruction(9)]
    pub fn empty(ctx: Ctx<Empty>, proof_offset: i8) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let (proof, instructions_sysvar) = locate(proof_offset, a.proof.as_account());
        EmptyConfidentialAccount {
            account: a.account.as_account(),
            authority: a.owner.as_account(),
            proof,
            instructions_sysvar,
        }
        .invoke()
    }

    /// A token account at its base size, configured from `owner`'s ElGamal
    /// registry. Token-2022 grows the account for the extension and the
    /// payer funds the growth; the owner does not sign.
    #[instruction(10)]
    pub fn open_from_registry(ctx: Ctx<OpenFromRegistry>) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let program = TokenProgram::Token2022;
        let size = GetAccountDataSize {
            mint: a.mint.as_account(),
            extension_types: &[],
        }
        .query(program)?;
        let space = usize::try_from(size).map_err(|_| ProgramError::InvalidAccountData)?;
        hopper::system::CreateAccount {
            from: a.payer.as_account(),
            to: a.account.as_account(),
            lamports: hopper::hopper_runtime::rent::minimum_balance_live(space)?,
            space: size,
            owner: program.address(),
        }
        .invoke()?;
        InitializeAccount3 {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            owner: a.owner.key(),
        }
        .invoke_on(program, &[], &[])?;
        ConfigureConfidentialAccountWithRegistry {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            elgamal_registry: a.registry.as_account(),
            payer: Some((a.payer.as_account(), a.system_program.as_account())),
        }
        .invoke()
    }

    /// A confidential transfer on a mint with a transfer fee: equality,
    /// amount validity, fee sigma, fee validity, and range proofs.
    #[instruction(11)]
    pub fn transfer_with_fee(
        ctx: Ctx<TransferWithFee>,
        new_balance: [u8; 36],
        auditor_lo: [u8; 64],
        auditor_hi: [u8; 64],
        offsets: [u8; 5],
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_2022(a.token_program.as_account())?;
        let (equality_proof, s1) = locate(offsets[0] as i8, a.equality_proof.as_account());
        let (transfer_amount_ciphertext_validity_proof, s2) =
            locate(offsets[1] as i8, a.amount_validity_proof.as_account());
        let (fee_sigma_proof, s3) = locate(offsets[2] as i8, a.fee_sigma_proof.as_account());
        let (fee_ciphertext_validity_proof, s4) =
            locate(offsets[3] as i8, a.fee_validity_proof.as_account());
        let (range_proof, s5) = locate(offsets[4] as i8, a.range_proof.as_account());
        ConfidentialTransferWithFee {
            source: a.source.as_account(),
            mint: a.mint.as_account(),
            destination: a.destination.as_account(),
            authority: a.owner.as_account(),
            new_source_decryptable_available_balance: &new_balance,
            transfer_amount_auditor_ciphertext_lo: &auditor_lo,
            transfer_amount_auditor_ciphertext_hi: &auditor_hi,
            equality_proof,
            transfer_amount_ciphertext_validity_proof,
            fee_sigma_proof,
            fee_ciphertext_validity_proof,
            range_proof,
            instructions_sysvar: s1.or(s2).or(s3).or(s4).or(s5),
        }
        .invoke()
    }
}
