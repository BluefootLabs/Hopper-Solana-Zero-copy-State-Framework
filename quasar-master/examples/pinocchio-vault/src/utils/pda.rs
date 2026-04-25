use pinocchio::Address;

#[inline(always)]
pub fn vault_pda(user: &[u8]) -> (Address, u8) {
    Address::find_program_address(&[b"vault", user], &crate::ID)
}
