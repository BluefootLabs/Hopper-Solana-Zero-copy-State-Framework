//! Account fixtures for token programs: base-layout mints and token
//! accounts owned by SPL Token or Token-2022, and the associated token
//! address. Pure builders over [`Account`]; add the program itself with
//! `mollusk_svm_programs_token` when the harness must execute it.

use solana_account::Account;
use solana_pubkey::Pubkey;

/// SPL Token program id.
pub const TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
/// Token-2022 program id.
pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
/// Associated Token Account program id.
pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
/// The System program id.
pub const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");

/// Lamports that keep a base mint or token account rent exempt under the
/// harness's rent (generous; the exact live minimum is not needed here).
pub const RENT_EXEMPT: u64 = 10_000_000;

/// An initialized base mint (82 bytes): `mint_authority` set, no freeze
/// authority unless given, zero supply, `decimals`, owned by `program`.
pub fn mint(
    program: Pubkey,
    mint_authority: &Pubkey,
    freeze_authority: Option<&Pubkey>,
    decimals: u8,
) -> Account {
    let mut data = vec![0u8; 82];
    data[..4].copy_from_slice(&1u32.to_le_bytes());
    data[4..36].copy_from_slice(mint_authority.as_ref());
    data[44] = decimals;
    data[45] = 1;
    if let Some(freeze) = freeze_authority {
        data[46..50].copy_from_slice(&1u32.to_le_bytes());
        data[50..82].copy_from_slice(freeze.as_ref());
    }
    Account {
        lamports: RENT_EXEMPT,
        data,
        owner: program,
        executable: false,
        rent_epoch: 0,
    }
}

/// An initialized base token account (165 bytes) for `mint`, owned by
/// `owner`, holding `amount`, owned by `program`.
pub fn token_account(program: Pubkey, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Account {
    let mut data = vec![0u8; 165];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    Account {
        lamports: RENT_EXEMPT,
        data,
        owner: program,
        executable: false,
        rent_epoch: 0,
    }
}

/// The associated token address of `wallet` for `mint` under `program`.
pub fn associated_token_address(wallet: &Pubkey, mint: &Pubkey, program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

/// A funded System account with no data.
pub fn wallet(lamports: u64) -> Account {
    Account::new(lamports, 0, &SYSTEM_PROGRAM_ID)
}

/// The `amount` field of a base token account, when `account` has one.
pub fn token_amount(account: &Account) -> Option<u64> {
    if account.data.len() < 72 {
        return None;
    }
    let mut amount = [0u8; 8];
    amount.copy_from_slice(&account.data[64..72]);
    Some(u64::from_le_bytes(amount))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_have_the_base_layouts() {
        let authority = Pubkey::new_unique();
        let freeze = Pubkey::new_unique();
        let m = mint(TOKEN_2022_PROGRAM_ID, &authority, Some(&freeze), 6);
        assert_eq!(m.data.len(), 82);
        assert_eq!(&m.data[4..36], authority.as_ref());
        assert_eq!(m.data[44], 6);
        assert_eq!(&m.data[50..82], freeze.as_ref());
        assert_eq!(m.owner, TOKEN_2022_PROGRAM_ID);

        let mint_key = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let t = token_account(TOKEN_PROGRAM_ID, &mint_key, &owner, 42);
        assert_eq!(t.data.len(), 165);
        assert_eq!(token_amount(&t), Some(42));
        assert_eq!(&t.data[32..64], owner.as_ref());

        let ata = associated_token_address(&owner, &mint_key, &TOKEN_PROGRAM_ID);
        assert_ne!(ata, owner);
        assert_eq!(token_amount(&wallet(1)), None);
    }
}
