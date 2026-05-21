//! Tiny programs do not inject receipt/modifier scaffolding into handlers.

use hopper::prelude::*;

#[program(profile = "tiny", entrypoint = false)]
mod bad_tiny_program {
    use super::*;

    #[instruction(1)]
    #[receipt]
    pub fn run(_ctx: &mut Context<'_>) -> ProgramResult {
        Ok(())
    }
}

fn main() {}