use {
    solana_address::Address,
    solana_instruction::{AccountMeta, Instruction},
    std::vec,
};

pub const ID: Address = solana_address::address!("33333333333333333333333333333333333333333333");

pub struct DepositInstruction {
    pub user: Address,
    pub vault: Address,
    pub system_program: Address,
    pub amount: u64,
}

impl From<DepositInstruction> for Instruction {
    fn from(ix: DepositInstruction) -> Instruction {
        let accounts = vec![
            AccountMeta::new(ix.user, true),
            AccountMeta::new(ix.vault, false),
            AccountMeta::new_readonly(ix.system_program, false),
        ];
        let mut data = vec![0];
        data.extend_from_slice(&wincode::serialize(&ix.amount).unwrap());
        Instruction {
            program_id: ID,
            accounts,
            data,
        }
    }
}

pub struct WithdrawInstruction {
    pub user: Address,
    pub vault: Address,
    pub amount: u64,
}

impl From<WithdrawInstruction> for Instruction {
    fn from(ix: WithdrawInstruction) -> Instruction {
        let accounts = vec![
            AccountMeta::new(ix.user, true),
            AccountMeta::new(ix.vault, false),
        ];
        let mut data = vec![1];
        data.extend_from_slice(&wincode::serialize(&ix.amount).unwrap());
        Instruction {
            program_id: ID,
            accounts,
            data,
        }
    }
}
