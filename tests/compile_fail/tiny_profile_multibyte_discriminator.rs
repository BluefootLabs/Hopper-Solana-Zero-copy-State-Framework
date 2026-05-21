//! Tiny programs keep Hopper's densest one-byte dispatch shape.

use hopper::prelude::*;

#[program(profile = "tiny", entrypoint = false)]
mod bad_tiny_program {
    use super::*;

    #[instruction(discriminator = [1, 2])]
    pub fn run(_ctx: &mut Context<'_>) -> ProgramResult {
        Ok(())
    }
}

fn main() {}