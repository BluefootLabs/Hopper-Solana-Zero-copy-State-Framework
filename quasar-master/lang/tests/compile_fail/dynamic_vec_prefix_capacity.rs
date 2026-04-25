#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;

solana_address::declare_id!("11111111111111111111111111111112");

#[account(discriminator = [1])]
pub struct BadVecPrefix<'a> {
    pub owner: Address,
    pub values: Vec<PodU64, u16, 70000>,
}

fn main() {}
