#![allow(non_camel_case_types)]

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state]
pub struct Shard {
    pub statuses: [u8; 4],
}

mod shadowed {
    use super::*;

    // A syntactic-only selector check would mistake this local alias for the
    // two-byte primitive. It is semantically `u8`, so runtime decoding would
    // consume one byte while a token-spelling manifest published two.
    type u16 = u8;

    #[derive(Accounts)]
    #[accounts(strict_writes)]
    #[instruction(slot: u16)]
    pub struct BadSelector<'info> {
        #[account(cells(slot; statuses))]
        pub shard: Account<'info, Shard>,
    }
}

fn main() {}
