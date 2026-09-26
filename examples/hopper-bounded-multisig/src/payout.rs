//! Threshold-approved, single-use SOL payouts with on-chain execution windows.
use super::*;

#[derive(Accounts)]
pub struct ApprovePayout<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    #[account(mut)]
    pub payout: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub destination: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ExecutePayout<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    #[account(mut)]
    pub payout: Account<'info, Payout>,
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct RevokePayout<'info> {
    #[account(mut)]
    pub multisig: Account<'info, Multisig>,
    #[account(mut)]
    pub payout: Account<'info, Payout>,
}

pub fn approve(
    ctx: ApprovePayoutCtx<'_, '_>,
    amount: u64,
    not_before: u64,
    expires: u64,
) -> ProgramResult {
    let approvals = ctx.remaining_accounts().signers::<10>()?;
    let a = &ctx.accounts;
    let mut keys = [Address::new([0; 32]); 10];
    for (i, signer) in approvals.iter().enumerate() {
        keys[i] = *signer.key();
    }
    let mut count = approvals.len();
    // A member paying rent can approve without duplicating that account role.
    if Multisig::signers(&a.multisig.as_account().try_borrow()?)?.contains(a.payer.key()) {
        if count == keys.len() {
            return Err(ProgramError::InvalidArgument);
        }
        keys[count] = *a.payer.key();
        count += 1;
    }
    authorize(a.multisig.as_account(), &keys[..count])?;
    if amount == 0 || not_before > expires || expires < now()? {
        return Err(ProgramError::InvalidArgument);
    }
    let roles = [
        a.multisig.key(),
        a.payout.key(),
        a.payer.key(),
        a.destination.key(),
    ];
    for (i, key) in roles.iter().enumerate() {
        if roles[..i].contains(key) {
            return Err(ProgramError::InvalidArgument);
        }
    }
    hopper::system::CreateAccount {
        from: a.payer.as_account(),
        to: a.payout.as_account(),
        lamports: hopper::hopper_runtime::rent::minimum_balance_live(Payout::INIT_SPACE)?,
        space: Payout::INIT_SPACE as u64,
        owner: ctx.program_id(),
    }
    .invoke()?;
    let mut data = a.payout.as_account().try_borrow_mut()?;
    hopper::systems::init_header::<Payout>(&mut data)?;
    *Payout::overlay_mut(&mut data[16..])? = Payout {
        multisig: *a.multisig.key(),
        destination: *a.destination.key(),
        policy_epoch: WireU64::new(a.multisig.get()?.policy_epoch()),
        amount: WireU64::new(amount),
        not_before: WireU64::new(not_before),
        expires: WireU64::new(expires),
        executed: WireBool::new(false),
    };
    Ok(())
}

pub fn execute(ctx: ExecutePayoutCtx<'_, '_>) -> ProgramResult {
    if ctx.instruction_data().len() != 1 || !ctx.remaining_accounts().is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let a = &ctx.accounts;
    let payout = *a.payout.get()?;
    if payout.multisig != *a.multisig.key()
        || payout.destination != *a.destination.key()
        || payout.policy_epoch.get() != a.multisig.get()?.policy_epoch()
        || payout.executed.get()
        || a.multisig.key() == a.destination.key()
        || a.payout.key() == a.destination.key()
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let clock = now()?;
    if clock < payout.not_before.get() || clock > payout.expires.get() {
        return Err(ProgramError::InvalidArgument);
    }
    let amount = payout.amount.get();
    let rent =
        hopper::hopper_runtime::rent::minimum_balance_live(a.multisig.as_account().data_len())?;
    if amount == 0 || amount > a.multisig.as_account().lamports().saturating_sub(rent) {
        return Err(ProgramError::InsufficientFunds);
    }
    transfer_lamports(a.multisig.as_account(), a.destination.as_account(), amount)?;
    // Keep a receipt and make re-execution fail. A threshold can reclaim its
    // rent later through revoke; the rent always returns to this multisig.
    ctx.accounts.payout.get_mut()?.executed = WireBool::new(true);
    Ok(())
}

pub fn revoke(ctx: RevokePayoutCtx<'_, '_>) -> ProgramResult {
    if ctx.instruction_data().len() != 1 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let approvals = ctx.remaining_accounts().signers::<10>()?;
    let mut keys = [Address::new([0; 32]); 10];
    for (i, signer) in approvals.iter().enumerate() {
        keys[i] = *signer.key();
    }
    let a = &ctx.accounts;
    authorize(a.multisig.as_account(), &keys[..approvals.len()])?;
    let payout = *a.payout.get()?;
    if payout.multisig != *a.multisig.key() {
        return Err(ProgramError::InvalidAccountData);
    }
    a.payout
        .as_account()
        .close_to(a.multisig.as_account(), ctx.program_id())
}

fn now() -> Result<u64, ProgramError> {
    u64::try_from(hopper::sysvar::Clock::get()?.unix_timestamp)
        .map_err(|_| ProgramError::InvalidArgument)
}
