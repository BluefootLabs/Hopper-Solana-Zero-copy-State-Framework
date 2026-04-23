//! Zero-copy Token-2022 extension TLV readers.
//!
//! Token-2022 stores extension data in a TLV region after the base
//! account bytes. Mints have extensions after byte 82; token accounts
//! have extensions after byte 165. Each TLV entry is
//! `[type: u16 LE][length: u16 LE][data: length bytes]`.
//!
//! Anchor routes every extension constraint through
//! `InterfaceAccount<Mint>`, which Borsh-deserializes the whole
//! account. Quasar has a zero-copy base-layout reader but no TLV
//! helpers. Pinocchio has nothing. This module fills the gap.
//!
//! Every reader here validates only the bytes it reads. No heap
//! allocation, no full-account decode, no version coupling to
//! `spl-token-2022`. A program that needs to enforce
//! `transfer_hook::authority = X` on a mint calls [`require_transfer_hook_authority`]
//! and pays the cost of a TLV scan plus a 32-byte compare. That is the
//! cost floor for a correct check.
//!
//! ## Account type byte
//!
//! Between the base layout and the TLV region sits a one-byte
//! discriminator: `0x01` for a Mint, `0x02` for a Token Account.
//! The TLV region begins immediately after that byte.
//!
//! Base layout offset table:
//! - Mint:          base 0..82,   account type at 82,   TLV at 83
//! - Token account: base 0..165,  account type at 165,  TLV at 166
//!
//! ## Extension type constants
//!
//! Values are the on-chain `u16` encoding from
//! `spl-token-2022::extension::ExtensionType`. They are stable wire
//! values and safe to hard-code. The full set is listed below so
//! tooling can surface the name for any TLV it encounters.

use crate::{error::ProgramError, result::ProgramResult, address::Address, account::AccountView};

// ── Extension type codes (stable wire values) ────────────────────────

pub const EXT_UNINITIALIZED: u16 = 0;
pub const EXT_TRANSFER_FEE_CONFIG: u16 = 1;
pub const EXT_TRANSFER_FEE_AMOUNT: u16 = 2;
pub const EXT_MINT_CLOSE_AUTHORITY: u16 = 3;
pub const EXT_CONFIDENTIAL_TRANSFER_MINT: u16 = 4;
pub const EXT_CONFIDENTIAL_TRANSFER_ACCOUNT: u16 = 5;
pub const EXT_DEFAULT_ACCOUNT_STATE: u16 = 6;
pub const EXT_IMMUTABLE_OWNER: u16 = 7;
pub const EXT_MEMO_TRANSFER: u16 = 8;
pub const EXT_NON_TRANSFERABLE: u16 = 9;
pub const EXT_INTEREST_BEARING_CONFIG: u16 = 10;
pub const EXT_CPI_GUARD: u16 = 11;
pub const EXT_PERMANENT_DELEGATE: u16 = 12;
pub const EXT_NON_TRANSFERABLE_ACCOUNT: u16 = 13;
pub const EXT_TRANSFER_HOOK: u16 = 14;
pub const EXT_TRANSFER_HOOK_ACCOUNT: u16 = 15;
pub const EXT_CONFIDENTIAL_TRANSFER_FEE_CONFIG: u16 = 16;
pub const EXT_CONFIDENTIAL_TRANSFER_FEE_AMOUNT: u16 = 17;
pub const EXT_METADATA_POINTER: u16 = 18;
pub const EXT_TOKEN_METADATA: u16 = 19;
pub const EXT_GROUP_POINTER: u16 = 20;
pub const EXT_TOKEN_GROUP: u16 = 21;
pub const EXT_GROUP_MEMBER_POINTER: u16 = 22;
pub const EXT_TOKEN_GROUP_MEMBER: u16 = 23;
pub const EXT_SCALED_UI_AMOUNT_CONFIG: u16 = 24;
pub const EXT_PAUSABLE_CONFIG: u16 = 25;
pub const EXT_PAUSABLE_ACCOUNT: u16 = 26;

/// Account-type byte: Mint.
pub const ACCOUNT_TYPE_MINT: u8 = 0x01;
/// Account-type byte: Token Account.
pub const ACCOUNT_TYPE_TOKEN: u8 = 0x02;

/// Base mint size. Extensions start at byte `BASE_MINT_LEN + 1`.
pub const BASE_MINT_LEN: usize = 82;
/// Base token account size. Extensions start at byte `BASE_TOKEN_LEN + 1`.
pub const BASE_TOKEN_LEN: usize = 165;

// ── TLV scanner ──────────────────────────────────────────────────────

/// Locate an extension in a Token-2022 account's TLV region.
///
/// Returns the slice of the extension's data bytes (not including the
/// 4-byte TLV header) or `None` if the extension is not present.
///
/// `tlv_bytes` must be the account data starting at the TLV region
/// (i.e. `&data[BASE_MINT_LEN + 1..]` for mints or `&data[BASE_TOKEN_LEN + 1..]`
/// for token accounts). Malformed TLVs (length runs past the buffer)
/// return `None` rather than panic.
///
/// One pass, O(n) in the TLV count. No allocation. The caller is
/// expected to amortize calls by grouping checks.
#[inline]
pub fn find_extension<'a>(tlv_bytes: &'a [u8], ext_type: u16) -> Option<&'a [u8]> {
    let mut cursor = 0usize;
    while cursor + 4 <= tlv_bytes.len() {
        let t = u16::from_le_bytes([tlv_bytes[cursor], tlv_bytes[cursor + 1]]);
        let len = u16::from_le_bytes([tlv_bytes[cursor + 2], tlv_bytes[cursor + 3]]) as usize;
        let data_start = cursor + 4;
        let data_end = data_start + len;
        if data_end > tlv_bytes.len() {
            return None;
        }
        if t == ext_type {
            return Some(&tlv_bytes[data_start..data_end]);
        }
        if t == EXT_UNINITIALIZED {
            // Uninitialized marker with zero length is a valid
            // stopping condition. Anything else with type 0 is
            // malformed padding; we stop scanning to avoid running
            // off the end through stray bytes.
            return None;
        }
        cursor = data_end;
    }
    None
}

/// Slice the TLV region out of a mint account's data.
///
/// Returns `None` if the account is too short to be a Token-2022
/// mint (i.e. it is a legacy SPL mint with no extensions).
#[inline]
pub fn mint_tlv_region(data: &[u8]) -> Option<&[u8]> {
    if data.len() <= BASE_MINT_LEN {
        return None;
    }
    // The account-type byte sits at index BASE_MINT_LEN. We accept
    // either 0x01 (Mint) or 0x00 on a just-extended mint that the
    // runtime has not yet stamped.
    let kind = data[BASE_MINT_LEN];
    if kind != ACCOUNT_TYPE_MINT && kind != 0 {
        return None;
    }
    Some(&data[BASE_MINT_LEN + 1..])
}

/// Slice the TLV region out of a token-account's data.
#[inline]
pub fn token_account_tlv_region(data: &[u8]) -> Option<&[u8]> {
    if data.len() <= BASE_TOKEN_LEN {
        return None;
    }
    let kind = data[BASE_TOKEN_LEN];
    if kind != ACCOUNT_TYPE_TOKEN && kind != 0 {
        return None;
    }
    Some(&data[BASE_TOKEN_LEN + 1..])
}

// ── Declarative require_* helpers for the common cases ────────────────

/// Require a mint to carry the `NonTransferable` extension.
///
/// Use when a program is designed to only ever mint soulbound tokens.
#[inline]
pub fn require_non_transferable(mint: &AccountView) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    if find_extension(tlv, EXT_NON_TRANSFERABLE).is_some() {
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}

/// Require a mint's `MintCloseAuthority` extension to equal `expected`.
#[inline]
pub fn require_mint_close_authority(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_MINT_CLOSE_AUTHORITY).ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 32 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[..32] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Require a mint's `PermanentDelegate` extension to equal `expected`.
#[inline]
pub fn require_permanent_delegate(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_PERMANENT_DELEGATE).ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 32 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[..32] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Require a mint's `TransferHook` program id to equal `expected`.
///
/// `TransferHook` layout: `[authority: 32][program_id: 32]`. This
/// validates the second field. Use [`require_transfer_hook_authority`]
/// for the first.
#[inline]
pub fn require_transfer_hook_program(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_TRANSFER_HOOK).ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 64 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[32..64] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectProgramId)
    }
}

/// Require a mint's `TransferHook` authority to equal `expected`.
#[inline]
pub fn require_transfer_hook_authority(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_TRANSFER_HOOK).ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 32 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[..32] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Require a mint's `MetadataPointer` metadata-address to equal `expected`.
///
/// `MetadataPointer` layout: `[authority: 32][metadata_address: 32]`.
#[inline]
pub fn require_metadata_pointer_address(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_METADATA_POINTER).ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 64 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[32..64] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}

/// Require a mint's `MetadataPointer` authority to equal `expected`.
#[inline]
pub fn require_metadata_pointer_authority(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_METADATA_POINTER).ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 32 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[..32] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Require a token account to carry the `ImmutableOwner` extension.
#[inline]
pub fn require_immutable_owner(token_account: &AccountView) -> ProgramResult {
    let data = token_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = token_account_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    if find_extension(tlv, EXT_IMMUTABLE_OWNER).is_some() {
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}

/// Require a mint's `DefaultAccountState` byte to equal `expected`.
///
/// Values: `0` Uninitialized, `1` Initialized, `2` Frozen.
#[inline]
pub fn require_default_account_state(mint: &AccountView, expected: u8) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_DEFAULT_ACCOUNT_STATE)
        .ok_or(ProgramError::InvalidAccountData)?;
    if ext.is_empty() {
        return Err(ProgramError::InvalidAccountData);
    }
    if ext[0] == expected {
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}

/// Require a mint's `InterestBearingConfig` rate-authority to equal `expected`.
///
/// Layout: `[rate_authority: 32][initialization_timestamp: 8][pre_update_average_rate: 2][last_update_timestamp: 8][current_rate: 2]`.
#[inline]
pub fn require_interest_bearing_authority(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_INTEREST_BEARING_CONFIG)
        .ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 32 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[..32] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Require a mint's `TransferFeeConfig` transfer-fee-config authority to equal `expected`.
///
/// Layout prefix: `[transfer_fee_config_authority: 32][withdraw_withheld_authority: 32]...`.
#[inline]
pub fn require_transfer_fee_config_authority(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_TRANSFER_FEE_CONFIG)
        .ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 32 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[..32] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Require a mint's `TransferFeeConfig` withdraw-withheld-authority to equal `expected`.
#[inline]
pub fn require_transfer_fee_withdraw_authority(
    mint: &AccountView,
    expected: &Address,
) -> ProgramResult {
    let data = mint.try_borrow().map_err(|_| ProgramError::AccountBorrowFailed)?;
    let tlv = mint_tlv_region(&data).ok_or(ProgramError::InvalidAccountData)?;
    let ext = find_extension(tlv, EXT_TRANSFER_FEE_CONFIG)
        .ok_or(ProgramError::InvalidAccountData)?;
    if ext.len() < 64 {
        return Err(ProgramError::InvalidAccountData);
    }
    if &ext[32..64] == expected.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic mint TLV buffer with a single extension.
    fn one_ext_mint(ext_type: u16, payload: &[u8]) -> alloc::vec::Vec<u8> {
        // base 82 bytes + 1 account-type byte + 4 TLV header + payload
        let mut v = alloc::vec![0u8; BASE_MINT_LEN];
        v.push(ACCOUNT_TYPE_MINT);
        v.extend_from_slice(&ext_type.to_le_bytes());
        v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn find_extension_returns_payload_slice() {
        let data = one_ext_mint(EXT_NON_TRANSFERABLE, &[]);
        let tlv = mint_tlv_region(&data).unwrap();
        assert!(find_extension(tlv, EXT_NON_TRANSFERABLE).is_some());
    }

    #[test]
    fn find_extension_returns_none_when_absent() {
        let data = one_ext_mint(EXT_NON_TRANSFERABLE, &[]);
        let tlv = mint_tlv_region(&data).unwrap();
        assert!(find_extension(tlv, EXT_TRANSFER_HOOK).is_none());
    }

    #[test]
    fn find_extension_bails_on_malformed_length() {
        let mut data = alloc::vec![0u8; BASE_MINT_LEN];
        data.push(ACCOUNT_TYPE_MINT);
        // type + over-long length, but no data
        data.extend_from_slice(&EXT_TRANSFER_HOOK.to_le_bytes());
        data.extend_from_slice(&999u16.to_le_bytes());
        let tlv = mint_tlv_region(&data).unwrap();
        assert!(find_extension(tlv, EXT_TRANSFER_HOOK).is_none());
    }

    #[test]
    fn mint_tlv_region_rejects_short_account() {
        let data = alloc::vec![0u8; 40];
        assert!(mint_tlv_region(&data).is_none());
    }

    #[test]
    fn mint_tlv_region_rejects_wrong_account_kind() {
        let mut data = alloc::vec![0u8; BASE_MINT_LEN];
        data.push(ACCOUNT_TYPE_TOKEN);
        assert!(mint_tlv_region(&data).is_none());
    }

    #[test]
    fn token_account_tlv_region_accepts_zero_kind_byte() {
        // A just-extended token account whose kind byte was not yet
        // stamped should still read as a token account. Extension
        // helpers rely on this so init sequencing stays permissive.
        let mut data = alloc::vec![0u8; BASE_TOKEN_LEN];
        data.push(0u8);
        assert!(token_account_tlv_region(&data).is_some());
    }

    extern crate alloc;
}
