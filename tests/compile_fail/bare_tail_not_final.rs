//! Bare final tails must consume the remaining dynamic-tail payload.
//! Any fixed field after them would make the wire contract ambiguous.

#[hopper::account(discriminator = 31, version = 1)]
pub struct BadBareTail<'a> {
    pub author: hopper::prelude::Address,
    pub content: hopper::prelude::TailStr<'a>,
    pub bump: u8,
}

fn main() {}