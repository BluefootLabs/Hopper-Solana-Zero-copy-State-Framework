//! `bump = stored` on a creation lifecycle (`init` / `init_if_needed` /
//! `zero`) can never succeed at runtime: Stage-4 PDA verification runs
//! BEFORE the init lifecycle executes, so the stored bump byte it reads
//! does not exist yet and every invocation of the creation instruction
//! would brick. Must be refused at compile time.

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Config {
    pub admin: Address,
    #[bump]
    pub bump: u8,
    pub reserved: [u8; 7],
}

#[derive(Accounts)]
pub struct CreateConfig<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        payer = payer,
        space = Config::INIT_SPACE,
        seeds = [b"cfg"],
        bump = stored
    )]
    pub config: InitAccount<'info, Config>,

    pub system_program: Program<'info, System>,
}

fn main() {}
