//! Bounded member storage with authenticated multisig administration and SOL custody.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

mod state;
pub use state::*;
mod payout;
pub use payout::*;

#[derive(Accounts)]
pub struct Manage<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
}
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub multisig: Signer<'info>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,
}

#[program]
mod bounded_multisig {
    use super::*;

    #[instruction(0)]
    pub fn rename(ctx: Ctx<Manage>, label: HopperString<32>) -> ProgramResult {
        let approvals = ctx.remaining_accounts().signers::<10>()?;
        let mut keys = [Address::new([0; 32]); 10];
        for (i, signer) in approvals.iter().enumerate() {
            keys[i] = *signer.key();
        }
        authorize(ctx.accounts.multisig.as_account(), &keys[..approvals.len()])?;
        ctx.accounts.multisig.set_label(label.as_str()?)
    }
    #[instruction(1)]
    pub fn add_signer(ctx: Ctx<Manage>, signer: Address) -> ProgramResult {
        let approvals = ctx.remaining_accounts().signers::<10>()?;
        let mut keys = [Address::new([0; 32]); 10];
        for (i, approval) in approvals.iter().enumerate() {
            keys[i] = *approval.key();
        }
        authorize(ctx.accounts.multisig.as_account(), &keys[..approvals.len()])?;
        if signer == Address::new([0; 32]) {
            return Err(ProgramError::InvalidArgument);
        }
        let epoch = ctx
            .accounts
            .multisig
            .get()?
            .policy_epoch()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        ctx.accounts.multisig.push_unique_signer(signer)?;
        ctx.accounts.multisig.get_mut()?.policy_epoch = WireU64::new(epoch);
        Ok(())
    }
    #[instruction(2)]
    pub fn initialize(
        ctx: Ctx<Initialize>,
        threshold: u64,
        label: HopperString<32>,
        members: HopperVec<Address, 10>,
    ) -> ProgramResult {
        validate_members(threshold, members.as_slice())?;
        label.as_str()?;
        let a = &ctx.accounts;
        if a.payer.key() == a.multisig.key() {
            return Err(ProgramError::InvalidArgument);
        }
        hopper::system::CreateAccount {
            from: a.payer.as_account(),
            to: a.multisig.as_account(),
            lamports: hopper::hopper_runtime::rent::minimum_balance_live(Multisig::ALLOC_SPACE)?,
            space: Multisig::ALLOC_SPACE as u64,
            owner: ctx.program_id(),
        }
        .invoke()?;
        initialize_multisig_data(
            &mut a.multisig.as_account().try_borrow_mut()?,
            threshold,
            label.as_str()?,
            members.as_slice(),
        )
    }
    #[instruction(3)]
    pub fn withdraw(ctx: Ctx<Withdraw>, amount: u64) -> ProgramResult {
        let approvals = ctx.remaining_accounts().signers::<10>()?;
        let mut keys = [Address::new([0; 32]); 10];
        for (i, signer) in approvals.iter().enumerate() {
            keys[i] = *signer.key();
        }
        let a = &ctx.accounts;
        // A receiving member can approve through the destination role itself;
        // it must not be repeated in the strict remaining-account tail.
        let mut count = approvals.len();
        let destination_is_member = {
            let data = a.multisig.as_account().try_borrow()?;
            Multisig::signers(&data)?.contains(a.destination.key())
        };
        if a.destination.as_account().is_signer() && destination_is_member {
            if count == keys.len() {
                return Err(ProgramError::InvalidArgument);
            }
            keys[count] = *a.destination.key();
            count += 1;
        }
        authorize(a.multisig.as_account(), &keys[..count])?;
        if amount == 0 || a.multisig.key() == a.destination.key() {
            return Err(ProgramError::InvalidArgument);
        }
        let rent =
            hopper::hopper_runtime::rent::minimum_balance_live(a.multisig.as_account().data_len())?;
        if amount > a.multisig.as_account().lamports().saturating_sub(rent) {
            return Err(ProgramError::InsufficientFunds);
        }
        transfer_lamports(a.multisig.as_account(), a.destination.as_account(), amount)
    }
    #[instruction(4)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        let a = &ctx.accounts;
        if amount == 0 || a.depositor.key() == a.multisig.key() {
            return Err(ProgramError::InvalidArgument);
        }
        hopper::system::Transfer {
            from: a.depositor.as_account(),
            to: a.multisig.as_account(),
            lamports: amount,
        }
        .invoke()
    }

    #[instruction(5)]
    pub fn approve_payout(
        ctx: Ctx<ApprovePayout>,
        amount: u64,
        not_before: u64,
        expires: u64,
    ) -> ProgramResult {
        payout::approve(ctx, amount, not_before, expires)
    }
    #[instruction(6)]
    pub fn execute_payout(ctx: Ctx<ExecutePayout>) -> ProgramResult {
        payout::execute(ctx)
    }
    #[instruction(7)]
    pub fn revoke_payout(ctx: Ctx<RevokePayout>) -> ProgramResult {
        payout::revoke(ctx)
    }
    #[instruction(8)]
    pub fn invalidate_payouts(ctx: Ctx<Manage>) -> ProgramResult {
        if ctx.instruction_data().len() != 1 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let approvals = ctx.remaining_accounts().signers::<10>()?;
        let mut keys = [Address::new([0; 32]); 10];
        for (i, signer) in approvals.iter().enumerate() {
            keys[i] = *signer.key();
        }
        authorize(ctx.accounts.multisig.as_account(), &keys[..approvals.len()])?;
        let next = ctx
            .accounts
            .multisig
            .get()?
            .policy_epoch()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        ctx.accounts.multisig.get_mut()?.policy_epoch = WireU64::new(next);
        Ok(())
    }

    #[instruction(9)]
    pub fn remove_member(ctx: Ctx<Manage>, member: Address) -> ProgramResult {
        let approvals = ctx.remaining_accounts().signers::<10>()?;
        let mut keys = [Address::new([0; 32]); 10];
        for (i, signer) in approvals.iter().enumerate() {
            keys[i] = *signer.key();
        }
        authorize(ctx.accounts.multisig.as_account(), &keys[..approvals.len()])?;
        let next = ctx
            .accounts
            .multisig
            .get()?
            .policy_epoch()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if !remove_signer_data(
            &mut ctx.accounts.multisig.as_account().try_borrow_mut()?,
            &member,
        )? {
            return Err(ProgramError::InvalidArgument);
        }
        ctx.accounts.multisig.get_mut()?.policy_epoch = WireU64::new(next);
        Ok(())
    }

    #[instruction(10)]
    pub fn change_threshold(ctx: Ctx<Manage>, threshold: u64) -> ProgramResult {
        let approvals = ctx.remaining_accounts().signers::<10>()?;
        let mut keys = [Address::new([0; 32]); 10];
        for (i, signer) in approvals.iter().enumerate() {
            keys[i] = *signer.key();
        }
        authorize(ctx.accounts.multisig.as_account(), &keys[..approvals.len()])?;
        let data = ctx.accounts.multisig.as_account().try_borrow()?;
        validate_members(threshold, Multisig::signers(&data)?)?;
        drop(data);
        let mut state = ctx.accounts.multisig.get_mut()?;
        let next = state
            .policy_epoch()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        state.threshold = WireU64::new(threshold);
        state.policy_epoch = WireU64::new(next);
        Ok(())
    }
}

fn authorize(account: &AccountView, approvals: &[Address]) -> ProgramResult {
    let data = account.try_borrow()?;
    let members = Multisig::signers(&data)?;
    if approvals.iter().any(|key| !members.contains(key)) {
        return Err(ProgramError::InvalidArgument);
    }
    if !threshold_met(&data, approvals)? {
        return Err(ProgramError::MissingRequiredSignature);
    }
    Ok(())
}
