#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();

    hopper::nostd_panic_handler!();
}

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Config {
    pub authority: Address,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = Config::INIT_SPACE)]
    pub config: InitAccount<'info, Config>,

    pub system_program: Program<'info, System>,
}

#[program]
mod app {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Ctx<Initialize>) -> ProgramResult {
        ctx.init_config()?;
        ctx.accounts.initialize()
    }
}

impl<'info> Initialize<'info> {
    pub fn initialize(&self) -> ProgramResult {
        let mut config = self.config.get_mut_after_init()?;
        config.set_inner(*self.authority.key(), 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_constants_are_stable() {
        assert_eq!(Config::DISC, 1);
        assert_eq!(Config::VERSION, 1);
        assert!(Config::INIT_SPACE >= 33);
    }
}
