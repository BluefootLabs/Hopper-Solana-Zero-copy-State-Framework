use hopper::prelude::*;

#[hopper::program]
mod bad {
    use super::*;

    // Cranks must be zero-arg. Adding `amount: u64` should fail at
    // macro expansion time with a clear diagnostic.
    #[hopper::crank]
    #[instruction(0)]
    fn settle(_ctx: &mut Context<'_>, amount: u64) -> ProgramResult {
        let _ = amount;
        Ok(())
    }
}

fn main() {}
