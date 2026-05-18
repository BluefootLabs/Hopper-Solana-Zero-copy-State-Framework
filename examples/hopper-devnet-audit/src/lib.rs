//! Devnet audit program for Hopper capability testing.
//!
//! The program is intentionally small, but it touches the Hopper surfaces that
//! need live-cluster confidence: pretty dynamic account syntax, typed context
//! validation, generated tail helpers, remaining-account signer parsing,
//! segment leases, and substrate exports.

#![cfg_attr(any(target_os = "solana", target_arch = "bpf"), no_std)]
#![allow(dead_code)]

use hopper::prelude::*;
use hopper::systems::SegmentBorrowRegistry;

#[cfg(any(target_os = "solana", target_arch = "bpf"))]
mod __hopper_sbf {
    #[cfg(not(feature = "solana-program-backend"))]
    hopper::no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    hopper::nostd_panic_handler!();
}

hopper::fast_entrypoint!(process_instruction, 8);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    hopper_devnet_audit::process_instruction(&mut ctx)
}

#[hopper::account(discriminator = 91, version = 1)]
pub struct AuditState<'a> {
    pub authority: Address,
    pub counter: u64,
    pub bump: u8,
    pub flags: u16,
    pub substrate_passes: u64,
    pub remaining_signer_checks: u64,
    pub label: String<'a, 32>,
    pub members: Vec<'a, Address, 8>,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init, payer = payer, space = AuditState::ALLOC_SPACE)]
    pub state: InitAccount<'info, AuditState>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Mutate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub state: Account<'info, AuditState>,
}

#[derive(Accounts)]
pub struct ReadAudit<'info> {
    pub authority: Signer<'info>,

    #[account(has_one = authority)]
    pub state: Account<'info, AuditState>,
}

#[program(entrypoint = false)]
mod hopper_devnet_audit {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Ctx<Initialize>, bump: u8) -> ProgramResult {
        ctx.init_state()?;
        let authority = *ctx.accounts.payer.key();

        {
            let mut state = ctx.accounts.state.get_mut_after_init()?;
            state.set_inner(authority, 0, bump, 0, 0, 0)?;
        }

        let mut tail = AuditStateTail::default();
        tail.label.set_str("devnet-audit")?;
        tail.members.push(authority)?;
        ctx.accounts.state.tail_write(&tail)?;
        Ok(())
    }

    #[instruction(1)]
    pub fn rename(ctx: Ctx<Mutate>) -> ProgramResult {
        ctx.accounts.state.set_label("hopper-live")
    }

    #[instruction(2)]
    pub fn add_member(ctx: Ctx<Mutate>) -> ProgramResult {
        let _inserted = ctx
            .accounts
            .state
            .push_unique_member(*ctx.accounts.authority.key())?;
        Ok(())
    }

    #[instruction(3)]
    pub fn increment_segment(ctx: Ctx<Mutate>) -> ProgramResult {
        let account = ctx.accounts.state.as_account();
        let mut borrows = SegmentBorrowRegistry::new();
        let mut counter = account.segment_mut::<WireU64>(
            &mut borrows,
            AuditState::COUNTER_ABS_OFFSET,
            AuditState::COUNTER_SIZE,
        )?;
        let next = counter
            .get()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *counter = WireU64::new(next);
        Ok(())
    }

    #[instruction(4)]
    pub fn substrate_probe(ctx: Ctx<Mutate>) -> ProgramResult {
        let budget = hopper::substrate::CuBudget::snapshot();
        budget.require_remaining(1)?;
        ctx.accounts.authority.as_account().check_writable()?;

        let mut state = ctx.accounts.state.get_mut()?;
        state.substrate_passes.checked_add_assign(1)?;
        budget.log_delta("hopper-devnet-audit");
        Ok(())
    }

    #[instruction(6)]
    pub fn remaining_signers(ctx: Ctx<Mutate>) -> ProgramResult {
        let signers = ctx.remaining_accounts().signers::<4>()?;
        if signers.len() < 2 {
            return Err(ProgramError::NotEnoughAccountKeys);
        }

        let passthrough = ctx.remaining_accounts_passthrough().account_views::<4>()?;
        if passthrough.len() != signers.len() {
            return Err(ProgramError::InvalidAccountData);
        }

        let mut state = ctx.accounts.state.get_mut()?;
        state
            .remaining_signer_checks
            .checked_add_assign(signers.len() as u64)?;
        Ok(())
    }

    #[instruction(5)]
    pub fn audit(ctx: Ctx<ReadAudit>) -> ProgramResult {
        let state = ctx.accounts.state.get()?;
        let authority = state.authority;
        drop(state);

        if authority != *ctx.accounts.authority.key() {
            return Err(ProgramError::InvalidAccountData);
        }

        let label = ctx.accounts.state.label()?;
        if label.as_str()?.is_empty() {
            return Err(ProgramError::InvalidAccountData);
        }

        let members = ctx.accounts.state.members()?;
        if !members
            .as_slice()
            .iter()
            .any(|member| *member == *ctx.accounts.authority.key())
        {
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(())
    }
}
