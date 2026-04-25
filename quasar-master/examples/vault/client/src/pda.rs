use solana_address::Address;

/// Seeds: [b"vault", user]
pub fn find_vault_address(user: &Address, program_id: &Address) -> (Address, u8) {
    Address::find_program_address(&[b"vault", user.as_ref()], program_id)
}
