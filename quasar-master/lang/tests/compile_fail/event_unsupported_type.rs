use quasar_lang::prelude::*;

#[event(discriminator = [1])]
pub struct Bad {
    pub x: Vec<u8>,
}

fn main() {}
