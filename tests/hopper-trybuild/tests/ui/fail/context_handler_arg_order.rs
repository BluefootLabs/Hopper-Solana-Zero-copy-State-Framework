use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state]
pub struct Shard {
    pub statuses: [u8; 4],
}

#[derive(Accounts)]
#[accounts(strict_writes)]
#[instruction(slot: u16, nonce: u16)]
pub struct Ordered<'info> {
    #[account(cells(slot; statuses))]
    pub shard: Account<'info, Shard>,
}

#[hopper::program(entrypoint = false)]
mod bad_order {
    use super::*;

    #[instruction(0, ctx_args = 2)]
    pub fn mutate(_ctx: Ctx<Ordered>, nonce: u16, slot: u16) -> ProgramResult {
        let _ = (nonce, slot);
        Ok(())
    }
}

fn main() {}
