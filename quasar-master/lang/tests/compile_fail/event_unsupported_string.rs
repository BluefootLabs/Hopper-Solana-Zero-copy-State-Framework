use quasar_lang::prelude::*;

#[event(discriminator = [1])]
pub struct Bad {
    pub x: String,
}

fn main() {}
