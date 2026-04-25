use {
    crate::ID,
    solana_address::Address,
    solana_instruction::{AccountMeta, Instruction},
};

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
        wincode::serialize_into(&mut data, &ix.amount).unwrap();
        Instruction {
            program_id: ID,
            accounts,
            data,
        }
    }
}
