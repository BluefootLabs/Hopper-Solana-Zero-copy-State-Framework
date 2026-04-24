//! Zero-copy Token-2022 extension TLV readers.
//!
//! Token-2022 stores extension data in a TLV region after a fixed
//! per-account prefix. Each TLV entry is
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
//! ## On-chain layout (authoritative)
//!
//! An extended mint is padded to the same length as an extended token
//! account so that the Token-2022 program cannot confuse the two by
//! data length alone; the one-byte `AccountType` discriminator at
//! offset `ACCOUNT_TYPE_OFFSET` (= 165) disambiguates them.
//!
//! ```text
//! Extended mint         : [0..82] Mint base | [82..165] padding | [165] AccountType=1 | [166..] TLV
//! Extended token account: [0..165] Account base                 | [165] AccountType=2 | [166..] TLV
//! ```
//!
//! Both shapes place the AccountType byte at offset 165 and begin TLV
//! data at offset 166. `BASE_MINT_LEN` (82) is the length of a *plain*,
//! non-extended mint and is used by length checks; it is **not** the
//! offset at which mint extensions live. This layout matches
//! `spl-token-2022` and the pinocchio reference implementation
//! (`validate_account_type` keys on `bytes[BASE_ACCOUNT_LENGTH]`
//! where `BASE_ACCOUNT_LENGTH = 165`).
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

/// Length of a plain (non-extended) mint's data region.
///
/// This is the stride of the `Mint` base struct; it is **not** the
/// offset at which mint extensions begin (see [`TLV_OFFSET`]). Used
/// for the length-only check that distinguishes a legacy SPL mint
/// (exactly 82 bytes) from an extended Token-2022 mint.
pub const BASE_MINT_LEN: usize = 82;
/// Length of a plain (non-extended) token-account's data region.
///
/// Also equal to [`ACCOUNT_TYPE_OFFSET`]: an extended mint is padded
/// up to this length so that the AccountType discriminator sits at
/// the same offset as on an extended token account.
pub const BASE_TOKEN_LEN: usize = 165;
/// Offset of the `AccountType` discriminator on any extended
/// Token-2022 account (mint or token account).
pub const ACCOUNT_TYPE_OFFSET: usize = BASE_TOKEN_LEN;
/// Offset at which the TLV extension region begins on any extended
/// Token-2022 account (mint or token account).
pub const TLV_OFFSET: usize = ACCOUNT_TYPE_OFFSET + 1;
/// Start of the mint's extension padding region (82..165). Bytes in
/// this range are zero-filled and exist purely to equalize the length
/// of extended mints and extended token accounts.
pub const MINT_EXTENSION_PADDING_START: usize = BASE_MINT_LEN;
/// End of the mint extension padding region (exclusive).
pub const MINT_EXTENSION_PADDING_END: usize = ACCOUNT_TYPE_OFFSET;

// ── TLV scanner ──────────────────────────────────────────────────────

/// Locate an extension in a Token-2022 account's TLV region.
///
/// Returns the slice of the extension's data bytes (not including the
/// 4-byte TLV header) or `None` if the extension is not present.
///
/// `tlv_bytes` must be the account data starting at the TLV region
/// (i.e. `&data[TLV_OFFSET..]` for both mints and token accounts).
/// Use [`mint_tlv_region`] or [`token_account_tlv_region`] to obtain
/// this slice safely. Malformed TLVs (length runs past the buffer)
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
/// Returns `None` if the account is too short to be an *extended*
/// Token-2022 mint (length must be strictly greater than
/// [`TLV_OFFSET`]; a plain 82-byte legacy mint has no TLV region).
///
/// Performs three validations:
/// 1. Account length is large enough to contain at least the TLV
///    header offset (`> TLV_OFFSET`, i.e. >= 166).
/// 2. The `AccountType` discriminator at [`ACCOUNT_TYPE_OFFSET`]
///    is either [`ACCOUNT_TYPE_MINT`] (0x01) or `0x00`. We accept
///    `0x00` for a just-reallocated mint that the Token-2022 program
///    has not yet stamped; every subsequent extension initializer
///    writes the correct byte, and the TLV scanner tolerates an
///    all-zero region by hitting `EXT_UNINITIALIZED` on the first
///    header read. This matches `spl-token-2022`'s permissive init
///    sequencing.
/// 3. Returns the tail slice beginning at [`TLV_OFFSET`] (166).
///
/// The bytes in `[BASE_MINT_LEN..ACCOUNT_TYPE_OFFSET]` (82..165) are
/// Token-2022's equalization padding and are intentionally skipped
/// over; they are not part of the TLV stream.
#[inline]
pub fn mint_tlv_region(data: &[u8]) -> Option<&[u8]> {
    if data.len() <= TLV_OFFSET {
        return None;
    }
    let kind = data[ACCOUNT_TYPE_OFFSET];
    if kind != ACCOUNT_TYPE_MINT && kind != 0 {
        return None;
    }
    Some(&data[TLV_OFFSET..])
}

/// Slice the TLV region out of a token-account's data.
///
/// Returns `None` if the account is too short to be an *extended*
/// Token-2022 token account. Same validation shape as
/// [`mint_tlv_region`] but requires the discriminator at
/// [`ACCOUNT_TYPE_OFFSET`] be [`ACCOUNT_TYPE_TOKEN`] (0x02) or
/// `0x00`. TLV data is read from [`TLV_OFFSET`] (166).
#[inline]
pub fn token_account_tlv_region(data: &[u8]) -> Option<&[u8]> {
    if data.len() <= TLV_OFFSET {
        return None;
    }
    let kind = data[ACCOUNT_TYPE_OFFSET];
    if kind != ACCOUNT_TYPE_TOKEN && kind != 0 {
        return None;
    }
    Some(&data[TLV_OFFSET..])
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
    extern crate alloc;
    use super::*;

    /// Build a mint buffer in the **real** Token-2022 on-chain layout:
    /// 82 bytes of Mint base, 83 bytes of zero padding, one AccountType
    /// byte, then a single TLV entry.
    ///
    /// A previous iteration of this helper elided the padding region
    /// and pushed the AccountType byte directly after the 82-byte
    /// base. The parser was wrong in exactly the complementary way, so
    /// the two wrongnesses aligned and the tests passed while the
    /// production code silently mis-read every real mainnet mint. This
    /// helper now matches `spl-token-2022` and pinocchio's
    /// `validate_account_type` (which keys on
    /// `bytes[BASE_ACCOUNT_LENGTH]` where `BASE_ACCOUNT_LENGTH = 165`).
    fn mint_with_exts(exts: &[(u16, &[u8])]) -> alloc::vec::Vec<u8> {
        // 82 base + 83 padding = 165 bytes, then AccountType, then TLV.
        let mut v = alloc::vec![0u8; ACCOUNT_TYPE_OFFSET];
        v.push(ACCOUNT_TYPE_MINT);
        for (ty, payload) in exts {
            v.extend_from_slice(&ty.to_le_bytes());
            v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            v.extend_from_slice(payload);
        }
        debug_assert!(v.len() > TLV_OFFSET);
        v
    }

    /// Single-extension convenience wrapper. Delegates to [`mint_with_exts`].
    fn one_ext_mint(ext_type: u16, payload: &[u8]) -> alloc::vec::Vec<u8> {
        mint_with_exts(&[(ext_type, payload)])
    }

    /// Build a token-account buffer in the real layout: 165 base bytes
    /// then AccountType then TLV.
    fn token_account_with_exts(exts: &[(u16, &[u8])]) -> alloc::vec::Vec<u8> {
        let mut v = alloc::vec![0u8; BASE_TOKEN_LEN];
        v.push(ACCOUNT_TYPE_TOKEN);
        for (ty, payload) in exts {
            v.extend_from_slice(&ty.to_le_bytes());
            v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            v.extend_from_slice(payload);
        }
        v
    }

    // ── Layout invariants (the regression suite for the offset bug) ──────

    #[test]
    fn offset_constants_match_authoritative_spec() {
        // Values anchored to spl-token-2022 and pinocchio's reference.
        assert_eq!(BASE_MINT_LEN, 82);
        assert_eq!(BASE_TOKEN_LEN, 165);
        assert_eq!(ACCOUNT_TYPE_OFFSET, 165);
        assert_eq!(TLV_OFFSET, 166);
        assert_eq!(MINT_EXTENSION_PADDING_START, 82);
        assert_eq!(MINT_EXTENSION_PADDING_END, 165);
        assert_eq!(ACCOUNT_TYPE_MINT, 0x01);
        assert_eq!(ACCOUNT_TYPE_TOKEN, 0x02);
    }

    #[test]
    fn real_layout_mint_tlv_region_starts_at_166() {
        // Build a real-layout mint whose only extension is NonTransferable,
        // placed at offset 166.
        let data = one_ext_mint(EXT_NON_TRANSFERABLE, &[]);
        let tlv = mint_tlv_region(&data).expect("extended mint must yield TLV region");
        // TLV must begin at offset 166, not at offset 83.
        assert_eq!(tlv.as_ptr() as usize - data.as_ptr() as usize, TLV_OFFSET);
        // First four bytes are type=9, length=0.
        assert_eq!(u16::from_le_bytes([tlv[0], tlv[1]]), EXT_NON_TRANSFERABLE);
        assert_eq!(u16::from_le_bytes([tlv[2], tlv[3]]), 0);
    }

    #[test]
    fn real_layout_mint_padding_is_not_treated_as_tlv() {
        // This is the exact shape that the previous implementation
        // mis-parsed: 82 base + 83 zero padding + AccountType=1 +
        // genuine TLV entry for TransferHook at offset 166. The old
        // parser read zero padding at offset 83 as type=0/length=0 and
        // short-circuited to None. The corrected parser must find the
        // real entry.
        let data = one_ext_mint(EXT_TRANSFER_HOOK, &[0u8; 64]);
        let tlv = mint_tlv_region(&data).expect("tlv region");
        assert!(find_extension(tlv, EXT_TRANSFER_HOOK).is_some());
    }

    // ── find_extension core ──────────────────────────────────────────────

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
        // Real-layout mint: 82 + 83 padding + type byte, then a
        // corrupt TLV header claiming 999 bytes of data that are not
        // present. Scanner must return None, not panic.
        let mut data = alloc::vec![0u8; ACCOUNT_TYPE_OFFSET];
        data.push(ACCOUNT_TYPE_MINT);
        data.extend_from_slice(&EXT_TRANSFER_HOOK.to_le_bytes());
        data.extend_from_slice(&999u16.to_le_bytes());
        let tlv = mint_tlv_region(&data).unwrap();
        assert!(find_extension(tlv, EXT_TRANSFER_HOOK).is_none());
    }

    #[test]
    fn find_extension_finds_second_entry() {
        let data = mint_with_exts(&[
            (EXT_METADATA_POINTER, &[1u8; 64]),
            (EXT_PERMANENT_DELEGATE, &[2u8; 32]),
        ]);
        let tlv = mint_tlv_region(&data).unwrap();
        let perm = find_extension(tlv, EXT_PERMANENT_DELEGATE).unwrap();
        assert_eq!(perm, &[2u8; 32]);
    }

    // ── Region accept / reject edges ─────────────────────────────────────

    #[test]
    fn mint_tlv_region_rejects_short_account() {
        // Anything <= TLV_OFFSET (166) has no extension region.
        let data = alloc::vec![0u8; 40];
        assert!(mint_tlv_region(&data).is_none());
        let data = alloc::vec![0u8; TLV_OFFSET];
        assert!(mint_tlv_region(&data).is_none());
    }

    #[test]
    fn mint_tlv_region_rejects_wrong_account_kind() {
        // A 166-byte buffer whose AccountType byte reads as Token
        // (0x02) must not decode as a mint.
        let mut data = alloc::vec![0u8; ACCOUNT_TYPE_OFFSET];
        data.push(ACCOUNT_TYPE_TOKEN);
        data.push(0); // make length > TLV_OFFSET
        assert!(mint_tlv_region(&data).is_none());
    }

    #[test]
    fn mint_tlv_region_accepts_zero_kind_byte() {
        // Permissive init sequencing: a freshly-reallocated mint may
        // have AccountType still zero. The scanner tolerates it.
        let mut data = alloc::vec![0u8; ACCOUNT_TYPE_OFFSET];
        data.push(0u8);
        data.push(0); // length > TLV_OFFSET
        assert!(mint_tlv_region(&data).is_some());
    }

    #[test]
    fn token_account_tlv_region_accepts_zero_kind_byte() {
        let mut data = alloc::vec![0u8; BASE_TOKEN_LEN];
        data.push(0u8);
        data.push(0); // length > TLV_OFFSET
        assert!(token_account_tlv_region(&data).is_some());
    }

    #[test]
    fn token_account_tlv_region_rejects_mint_kind() {
        let mut data = alloc::vec![0u8; BASE_TOKEN_LEN];
        data.push(ACCOUNT_TYPE_MINT);
        data.push(0);
        assert!(token_account_tlv_region(&data).is_none());
    }

    #[test]
    fn token_account_tlv_region_returns_real_tlv() {
        let data = token_account_with_exts(&[(EXT_IMMUTABLE_OWNER, &[])]);
        let tlv = token_account_tlv_region(&data).unwrap();
        assert_eq!(tlv.as_ptr() as usize - data.as_ptr() as usize, TLV_OFFSET);
        assert!(find_extension(tlv, EXT_IMMUTABLE_OWNER).is_some());
    }
}
