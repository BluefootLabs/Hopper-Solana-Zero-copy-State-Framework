use super::*;

pub(super) fn verify_program(account: &AccountView<'_>) -> ProgramResult {
    account
        .check_address(&TOKEN_PROGRAM_ID)?
        .check_executable()?;
    Ok(())
}
pub(super) fn distinct(keys: &[&Address]) -> ProgramResult {
    for (i, key) in keys.iter().enumerate() {
        hopper::hopper_require!(!keys[..i].contains(key), AliasedAccount);
    }
    Ok(())
}
pub(super) fn mint_decimals(account: &AccountView<'_>) -> Result<u8, ProgramError> {
    account.check_owned_by(&TOKEN_PROGRAM_ID)?;
    let data = account.try_borrow()?;
    // No extensions or freeze authority; native accounts are rejected below.
    hopper::hopper_require!(
        data.len() == 82 && data[45] == 1 && data[46..50] == [0; 4],
        UnsupportedMint
    );
    Ok(data[44])
}
pub(super) fn token_balance(
    account: &AccountView<'_>,
    mint: &Address,
    authority: &Address,
    custody: bool,
) -> Result<u64, ProgramError> {
    account.check_owned_by(&TOKEN_PROGRAM_ID)?;
    let data = account.try_borrow()?;
    hopper::hopper_require!(data.len() == 165, InvalidTokenAccount);
    hopper::hopper_require!(data[..32] == mint.as_array()[..], MintMismatch);
    hopper::hopper_require!(data[32..64] == authority.as_array()[..], EscrowUnauthorized);
    hopper::hopper_require!(
        data[108] == 1 && data[109..113] == [0; 4],
        InvalidTokenAccount
    );
    if custody {
        hopper::hopper_require!(
            data[72..76] == [0; 4] && data[129..133] == [0; 4],
            InvalidTokenAccount
        );
    }
    let mut amount = [0; 8];
    amount.copy_from_slice(&data[64..72]);
    Ok(u64::from_le_bytes(amount))
}
