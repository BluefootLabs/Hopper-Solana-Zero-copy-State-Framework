//! The mint and token account readers that hand out a reference into the
//! account's bytes: the reference points where the field is, and a buffer
//! one byte short is refused.

use hopper_runtime::ProgramError;
use hopper_solana::mint::{mint_authority, mint_freeze_authority, MINT_LEN};
use hopper_solana::token::{token_account_mint, token_account_owner, TOKEN_ACCOUNT_LEN};

/// A buffer where byte `i` holds `i`.
fn counted<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = i as u8;
    }
    bytes
}

#[test]
fn mint_authorities_follow_their_option_tags() {
    let mut mint = counted::<MINT_LEN>();
    mint[0..4].copy_from_slice(&1u32.to_le_bytes());
    mint[46..50].copy_from_slice(&1u32.to_le_bytes());

    let authority = mint_authority(&mint).unwrap().unwrap();
    assert_eq!(authority.as_array()[0], 4);
    assert_eq!(authority.as_array()[31], 35);
    let freeze = mint_freeze_authority(&mint).unwrap().unwrap();
    assert_eq!(freeze.as_array()[0], 50);
    assert_eq!(freeze.as_array()[31], 81);
    assert!(core::ptr::eq(
        freeze.as_array().as_ptr(),
        mint[50..].as_ptr()
    ));

    mint[46..50].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(mint_freeze_authority(&mint).unwrap(), None);
    assert!(mint_authority(&mint).unwrap().is_some());
    mint[0..4].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(mint_authority(&mint).unwrap(), None);
}

#[test]
fn an_option_tag_the_token_program_never_writes_is_refused() {
    let mut mint = counted::<MINT_LEN>();
    for tag in [2u32, 0x100, 0x0100_0000, u32::MAX] {
        mint[0..4].copy_from_slice(&tag.to_le_bytes());
        mint[46..50].copy_from_slice(&tag.to_le_bytes());
        assert_eq!(
            mint_authority(&mint),
            Err(ProgramError::InvalidAccountData),
            "tag {tag:#x}"
        );
        assert_eq!(
            mint_freeze_authority(&mint),
            Err(ProgramError::InvalidAccountData),
            "tag {tag:#x}"
        );
    }
}

#[test]
fn token_account_keys_are_read_in_place() {
    let account = counted::<TOKEN_ACCOUNT_LEN>();
    let mint = token_account_mint(&account).unwrap();
    assert_eq!((mint.as_array()[0], mint.as_array()[31]), (0, 31));
    let owner = token_account_owner(&account).unwrap();
    assert_eq!((owner.as_array()[0], owner.as_array()[31]), (32, 63));
    assert!(core::ptr::eq(
        owner.as_array().as_ptr(),
        account[32..].as_ptr()
    ));
}

#[test]
fn a_buffer_one_byte_short_is_refused() {
    let mut mint = counted::<MINT_LEN>();
    mint[0..4].copy_from_slice(&1u32.to_le_bytes());
    mint[46..50].copy_from_slice(&1u32.to_le_bytes());
    for len in [0, 36, 50, MINT_LEN - 1] {
        assert_eq!(
            mint_authority(&mint[..len]),
            Err(ProgramError::InvalidAccountData),
            "{len} bytes"
        );
        assert_eq!(
            mint_freeze_authority(&mint[..len]),
            Err(ProgramError::InvalidAccountData),
            "{len} bytes"
        );
    }
    let account = counted::<TOKEN_ACCOUNT_LEN>();
    for len in [0, 32, 64, TOKEN_ACCOUNT_LEN - 1] {
        assert_eq!(
            token_account_owner(&account[..len]),
            Err(ProgramError::InvalidAccountData),
            "{len} bytes"
        );
        assert_eq!(
            token_account_mint(&account[..len]),
            Err(ProgramError::InvalidAccountData),
            "{len} bytes"
        );
    }
}
