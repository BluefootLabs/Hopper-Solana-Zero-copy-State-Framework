#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;

solana_address::declare_id!("11111111111111111111111111111112");

#[account(discriminator = [1])]
pub struct BadDynamic<'a> {
    pub vals: Vec<u64>,
}

fn main() {}
