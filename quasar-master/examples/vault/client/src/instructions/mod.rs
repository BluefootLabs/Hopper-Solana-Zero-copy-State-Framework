pub mod deposit;
pub mod withdraw;

pub use {deposit::*, withdraw::*};

pub enum ProgramInstruction {
    Deposit { amount: u64 },
    Withdraw { amount: u64 },
}

pub fn decode_instruction(data: &[u8]) -> Option<ProgramInstruction> {
    let disc = *data.first()?;
    match disc {
        0 => {
            let payload = &data[1..];
            let amount: u64 = wincode::deserialize(payload).ok()?;
            Some(ProgramInstruction::Deposit { amount })
        }
        1 => {
            let payload = &data[1..];
            let amount: u64 = wincode::deserialize(payload).ok()?;
            Some(ProgramInstruction::Withdraw { amount })
        }
        _ => None,
    }
}
