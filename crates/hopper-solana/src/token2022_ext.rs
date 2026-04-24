//! Token-2022 extension screening.
//!
//! Parse the TLV (Type-Length-Value) extension area on Token-2022 mints
//! and token accounts. Provides both individual extension readers and
//! blanket safety checks aimed at DeFi programs (AMMs, lending, staking,
//! escrow) that need to reject exotic extensions that violate their
//! assumptions.
//!
//! ## Token-2022 on-chain layout (authoritative)
//!
//! The base `Mint` struct is 82 bytes; the base `Account` struct is 165
//! bytes. An extended mint is padded up to 165 bytes so that it matches
//! the length of an extended token account and the `AccountType`
//! discriminator falls at the same offset on both shapes:
//!
//! ```text
//! Extended mint         : [0..82] Mint base | [82..165] padding | [165] AccountType = 1 | [166..] TLV
//! Extended token account: [0..165] Account base                 | [165] AccountType = 2 | [166..] TLV
//! ```
//!
//! TLV data always begins at byte 166 for both shapes. Each TLV entry is:
//!
//! ```text
//!   [u16 LE type] [u16 LE length] [length bytes value]
//! ```
//!
//! Extensions are concatenated. The type determines which extension the
//! TLV entry represents. This matches `spl-token-2022` and the pinocchio
//! reference implementation (`validate_account_type` keys on
//! `bytes[BASE_ACCOUNT_LENGTH]` with `BASE_ACCOUNT_LENGTH = 165`).

use hopper_runtime::error::ProgramError;

// ── Extension Type Discriminators (Token-2022 ExtensionType u16 values) ──────

/// Transfer Fee Config extension (mint).
pub const EXT_TRANSFER_FEE_CONFIG: u16 = 1;
/// Transfer Fee Amount extension (token account).
pub const EXT_TRANSFER_FEE_AMOUNT: u16 = 2;
/// Mint Close Authority extension.
pub const EXT_MINT_CLOSE_AUTHORITY: u16 = 3;
/// Confidential Transfer Mint extension.
pub const EXT_CONFIDENTIAL_TRANSFER_MINT: u16 = 4;
/// Confidential Transfer Account extension.
pub const EXT_CONFIDENTIAL_TRANSFER_ACCOUNT: u16 = 5;
/// Default Account State extension (mint).
pub const EXT_DEFAULT_ACCOUNT_STATE: u16 = 6;
/// Immutable Owner extension (token account).
pub const EXT_IMMUTABLE_OWNER: u16 = 7;
/// Memo Transfer extension.
pub const EXT_MEMO_TRANSFER: u16 = 8;
/// Non-Transferable extension (mint).
pub const EXT_NON_TRANSFERABLE: u16 = 9;
/// Interest-Bearing Mint extension.
pub const EXT_INTEREST_BEARING: u16 = 10;
/// CPI Guard extension (token account).
pub const EXT_CPI_GUARD: u16 = 11;
/// Permanent Delegate extension (mint).
pub const EXT_PERMANENT_DELEGATE: u16 = 12;
/// Transfer Hook extension (mint).
pub const EXT_TRANSFER_HOOK: u16 = 14;
/// Metadata Pointer extension (mint).
pub const EXT_METADATA_POINTER: u16 = 18;
/// Token Metadata extension (mint).
pub const EXT_TOKEN_METADATA: u16 = 19;
/// Group Pointer extension (mint).
pub const EXT_GROUP_POINTER: u16 = 20;
/// Group Member Pointer extension (mint).
pub const EXT_GROUP_MEMBER_POINTER: u16 = 22;

/// Base mint account data size (before extensions).
pub const MINT_BASE_SIZE: usize = 82;

/// Base token account data size (before extensions). Also equal to
/// [`ACCOUNT_TYPE_OFFSET`]: an extended mint is padded up to this
/// length so its AccountType discriminator lives at the same offset
/// as on an extended token account.
pub const TOKEN_ACCOUNT_BASE_SIZE: usize = 165;

/// Offset of the `AccountType` discriminator on any extended
/// Token-2022 account (mint or token account).
pub const ACCOUNT_TYPE_OFFSET: usize = TOKEN_ACCOUNT_BASE_SIZE;

/// Offset at which the TLV extension region begins on any extended
/// Token-2022 account (mint or token account).
pub const TLV_OFFSET: usize = ACCOUNT_TYPE_OFFSET + 1;

/// Account-type discriminator byte: Mint.
pub const ACCOUNT_TYPE_MINT: u8 = 1;
/// Account-type discriminator byte: Token Account.
pub const ACCOUNT_TYPE_TOKEN: u8 = 2;

// ── TLV Parsing ──────────────────────────────────────────────────────────────

/// Find the first TLV entry of `ext_type` in a Token-2022 account's data.
///
/// Returns the byte slice of the extension value, or `None` if not found.
/// Works for both mint and token accounts: the TLV region begins at a
/// fixed offset (166) on both shapes because extended mints carry 83
/// bytes of padding that equalize them to the token-account length.
///
/// The `base_size` parameter is kept for API compatibility; callers
/// typically pass [`MINT_BASE_SIZE`] or [`TOKEN_ACCOUNT_BASE_SIZE`].
/// It is used to verify the expected `AccountType` discriminator: a
/// mint-shaped base is only allowed when the byte at offset 165 is
/// [`ACCOUNT_TYPE_MINT`] or `0`, and likewise for token accounts.
/// This rejects mint extensions read out of a token-account buffer
/// (and vice versa) instead of returning silently wrong answers.
///
/// Returns `None` on any of:
/// - account shorter than [`TLV_OFFSET`] + 1 (plain non-extended account)
/// - `AccountType` byte does not match `base_size`'s expected shape
/// - malformed TLV (declared length runs past the end of the buffer)
#[inline(always)]
pub fn find_extension_data(data: &[u8], base_size: usize, ext_type: u16) -> Option<&[u8]> {
    // Must be long enough to hold at least the AccountType byte and
    // the start of the TLV region.
    if data.len() <= TLV_OFFSET {
        return None;
    }

    // Validate the AccountType discriminator against the caller's
    // declared shape. `0` is permissive for mid-init accounts.
    let expected = match base_size {
        MINT_BASE_SIZE => ACCOUNT_TYPE_MINT,
        TOKEN_ACCOUNT_BASE_SIZE => ACCOUNT_TYPE_TOKEN,
        // Unknown base_size: refuse to guess. Historically this
        // function happily walked any pointer arithmetic the caller
        // supplied, which is how the offset bug went undetected.
        _ => return None,
    };
    let kind = data[ACCOUNT_TYPE_OFFSET];
    if kind != expected && kind != 0 {
        return None;
    }

    let mut offset = TLV_OFFSET;
    while offset + 4 <= data.len() {
        let ty = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let len = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as usize;
        let value_start = offset + 4;
        let value_end = value_start.checked_add(len)?;

        if value_end > data.len() {
            return None; // Truncated TLV
        }

        if ty == ext_type {
            return Some(&data[value_start..value_end]);
        }

        // `Uninitialized` (type 0) with zero length is a valid stop
        // marker on the Token-2022 wire; treating stray zero padding
        // as an endless sequence of empty TLVs masks real bugs in
        // producer code.
        if ty == 0 && len == 0 {
            return None;
        }

        offset = value_end;
    }
    None
}

/// Check if a Token-2022 mint account has a specific extension.
#[inline(always)]
pub fn mint_has_extension(mint_data: &[u8], ext_type: u16) -> bool {
    find_extension_data(mint_data, MINT_BASE_SIZE, ext_type).is_some()
}

/// Check if a Token-2022 token account has a specific extension.
#[inline(always)]
pub fn token_has_extension(token_data: &[u8], ext_type: u16) -> bool {
    find_extension_data(token_data, TOKEN_ACCOUNT_BASE_SIZE, ext_type).is_some()
}

// ── Safety Checks ────────────────────────────────────────────────────────────

/// Reject mints that have a Transfer Fee Config extension.
///
/// Transfer fees silently reduce the amount received, which can break AMM
/// invariants, lending health checks, and distribution math.
#[inline(always)]
pub fn check_no_transfer_fee(mint_data: &[u8]) -> Result<(), ProgramError> {
    if mint_has_extension(mint_data, EXT_TRANSFER_FEE_CONFIG) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Reject mints with a Permanent Delegate.
///
/// A permanent delegate can burn or transfer tokens from ANY holder at
/// any time, making escrow and collateral positions unsafe.
#[inline(always)]
pub fn check_no_permanent_delegate(mint_data: &[u8]) -> Result<(), ProgramError> {
    if mint_has_extension(mint_data, EXT_PERMANENT_DELEGATE) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Reject mints with Confidential Transfer.
///
/// Encrypted balances prevent on-chain verification of collateral ratios,
/// AMM invariants, and distribution correctness.
#[inline(always)]
pub fn check_no_confidential_transfer(mint_data: &[u8]) -> Result<(), ProgramError> {
    if mint_has_extension(mint_data, EXT_CONFIDENTIAL_TRANSFER_MINT) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Reject non-transferable (soul-bound) mints.
#[inline(always)]
pub fn check_transferable(mint_data: &[u8]) -> Result<(), ProgramError> {
    if mint_has_extension(mint_data, EXT_NON_TRANSFERABLE) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Reject mints with a Transfer Hook.
///
/// Transfer hooks invoke arbitrary programs on every transfer, which
/// may re-enter or add unbounded CU cost.
#[inline(always)]
pub fn check_no_transfer_hook(mint_data: &[u8]) -> Result<(), ProgramError> {
    if mint_has_extension(mint_data, EXT_TRANSFER_HOOK) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Blanket safety check: reject mints with any DeFi-unsafe extension.
///
/// Rejects: transfer fee, permanent delegate, confidential transfer,
/// non-transferable, transfer hook.
///
/// This is the recommended default for AMM pools, lending markets, and
/// staking programs.
#[inline(always)]
pub fn check_safe_token_2022_mint(mint_data: &[u8]) -> Result<(), ProgramError> {
    check_no_transfer_fee(mint_data)?;
    check_no_permanent_delegate(mint_data)?;
    check_no_confidential_transfer(mint_data)?;
    check_transferable(mint_data)?;
    check_no_transfer_hook(mint_data)?;
    Ok(())
}

// ── Transfer Fee Reader ──────────────────────────────────────────────────────

/// Transfer fee configuration extracted from a Token-2022 mint.
///
/// Layout of the TransferFeeConfig extension value (108 bytes):
/// ```text
///   0..32   transfer_fee_config_authority
///  32..64   withdraw_withheld_authority
///  64..72   withheld_amount (u64 LE)
///  72..74   older_epoch (u16 LE)
///  74..82   older_maximum_fee (u64 LE)
///  82..84   older_transfer_fee_bps (u16 LE)
///  84..86   newer_epoch (u16 LE)
///  86..94   newer_maximum_fee (u64 LE)
///  94..96   newer_transfer_fee_bps (u16 LE)
/// ```
pub struct TransferFeeConfig {
    /// Current epoch's transfer fee in basis points.
    pub fee_bps: u16,
    /// Maximum fee amount for transfers in the current epoch.
    pub maximum_fee: u64,
}

/// Read the active transfer fee config from a Token-2022 mint.
///
/// Returns the `newer` fee schedule. Protocols should also compare
/// `current_epoch` against the epoch boundaries if they need the
/// exact fee for the current slot.
#[inline(always)]
pub fn read_transfer_fee_config(mint_data: &[u8]) -> Result<TransferFeeConfig, ProgramError> {
    let ext = find_extension_data(mint_data, MINT_BASE_SIZE, EXT_TRANSFER_FEE_CONFIG)
        .ok_or(ProgramError::InvalidAccountData)?;

    // Newer schedule starts at offset 84 within the extension value.
    if ext.len() < 96 {
        return Err(ProgramError::InvalidAccountData);
    }

    let newer_max_fee = u64::from_le_bytes([
        ext[86], ext[87], ext[88], ext[89],
        ext[90], ext[91], ext[92], ext[93],
    ]);
    let newer_fee_bps = u16::from_le_bytes([ext[94], ext[95]]);

    Ok(TransferFeeConfig {
        fee_bps: newer_fee_bps,
        maximum_fee: newer_max_fee,
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::vec;
    use alloc::vec::Vec;
    use super::*;

    /// Build a mint buffer in the **real** Token-2022 on-chain layout:
    /// 82 bytes of `Mint` base, 83 bytes of zero padding equalizing it
    /// to the token-account length, one `AccountType` byte, then TLV
    /// entries. An earlier iteration of this helper elided the 83-byte
    /// padding region, and the parser was wrong in exactly the
    /// complementary way, so tests agreed with buggy code. This helper
    /// now matches `spl-token-2022` and pinocchio's
    /// `validate_account_type` (AccountType at offset 165, TLV at 166).
    fn sample_mint_with_extensions(exts: &[(u16, &[u8])]) -> Vec<u8> {
        let mut data = vec![0u8; ACCOUNT_TYPE_OFFSET]; // 82 base + 83 padding
        data.push(ACCOUNT_TYPE_MINT);
        for (ext_type, ext_value) in exts {
            data.extend_from_slice(&ext_type.to_le_bytes());
            data.extend_from_slice(&(ext_value.len() as u16).to_le_bytes());
            data.extend_from_slice(ext_value);
        }
        debug_assert!(data.len() > TLV_OFFSET);
        data
    }

    /// Single-extension convenience wrapper.
    fn sample_mint_with_extension(ext_type: u16, ext_value: &[u8]) -> Vec<u8> {
        sample_mint_with_extensions(&[(ext_type, ext_value)])
    }

    // ── Layout invariants (regression for the offset bug) ────────────────

    #[test]
    fn offset_constants_match_authoritative_spec() {
        assert_eq!(MINT_BASE_SIZE, 82);
        assert_eq!(TOKEN_ACCOUNT_BASE_SIZE, 165);
        assert_eq!(ACCOUNT_TYPE_OFFSET, 165);
        assert_eq!(TLV_OFFSET, 166);
        assert_eq!(ACCOUNT_TYPE_MINT, 1);
        assert_eq!(ACCOUNT_TYPE_TOKEN, 2);
    }

    #[test]
    fn tlv_payload_lives_at_byte_166() {
        // Construct a real-layout mint with a single NonTransferable
        // extension. The first byte of the TLV header must sit at
        // offset 166, not offset 84 (the prior buggy offset).
        let data = sample_mint_with_extension(EXT_NON_TRANSFERABLE, &[]);
        assert_eq!(
            u16::from_le_bytes([data[TLV_OFFSET], data[TLV_OFFSET + 1]]),
            EXT_NON_TRANSFERABLE,
        );
        // And the previously-expected offset is pure zero padding.
        assert_eq!(data[84], 0);
        assert_eq!(data[85], 0);
    }

    // ── Screening checks ─────────────────────────────────────────────────

    #[test]
    fn no_extensions_passes_all_checks() {
        // Plain, non-extended mint (exactly 82 bytes).
        let data = vec![0u8; MINT_BASE_SIZE];
        assert!(check_safe_token_2022_mint(&data).is_ok());
    }

    #[test]
    fn detects_transfer_fee() {
        let data = sample_mint_with_extension(EXT_TRANSFER_FEE_CONFIG, &[0u8; 108]);
        assert!(mint_has_extension(&data, EXT_TRANSFER_FEE_CONFIG));
        assert!(check_no_transfer_fee(&data).is_err());
        assert!(check_safe_token_2022_mint(&data).is_err());
    }

    #[test]
    fn detects_permanent_delegate() {
        let data = sample_mint_with_extension(EXT_PERMANENT_DELEGATE, &[0u8; 32]);
        assert!(check_no_permanent_delegate(&data).is_err());
        assert!(check_safe_token_2022_mint(&data).is_err());
    }

    #[test]
    fn detects_confidential_transfer() {
        let data = sample_mint_with_extension(EXT_CONFIDENTIAL_TRANSFER_MINT, &[0u8; 64]);
        assert!(check_no_confidential_transfer(&data).is_err());
    }

    #[test]
    fn detects_non_transferable() {
        let data = sample_mint_with_extension(EXT_NON_TRANSFERABLE, &[]);
        assert!(check_transferable(&data).is_err());
    }

    #[test]
    fn detects_transfer_hook() {
        let data = sample_mint_with_extension(EXT_TRANSFER_HOOK, &[0u8; 64]);
        assert!(check_no_transfer_hook(&data).is_err());
    }

    #[test]
    fn safe_with_benign_extensions_only() {
        // Metadata pointer + token metadata = benign, should pass.
        let data = sample_mint_with_extensions(&[
            (EXT_METADATA_POINTER, &[0u8; 64]),
            (EXT_TOKEN_METADATA, &[0u8; 100]),
        ]);
        assert!(check_safe_token_2022_mint(&data).is_ok());
    }

    #[test]
    fn finds_second_extension() {
        let data = sample_mint_with_extensions(&[
            (EXT_METADATA_POINTER, &[0u8; 64]),
            (EXT_PERMANENT_DELEGATE, &[0u8; 32]),
        ]);
        assert!(mint_has_extension(&data, EXT_PERMANENT_DELEGATE));
        assert!(check_no_permanent_delegate(&data).is_err());
    }

    #[test]
    fn read_transfer_fee_config_parses_correctly() {
        let mut ext_value = vec![0u8; 96];
        // newer_maximum_fee at offset 86..94 = 1_000_000
        let max_fee = 1_000_000u64;
        ext_value[86..94].copy_from_slice(&max_fee.to_le_bytes());
        // newer_transfer_fee_bps at offset 94..96 = 250 (2.5%)
        ext_value[94..96].copy_from_slice(&250u16.to_le_bytes());

        let data = sample_mint_with_extension(EXT_TRANSFER_FEE_CONFIG, &ext_value);
        let fee = read_transfer_fee_config(&data).unwrap();
        assert_eq!(fee.fee_bps, 250);
        assert_eq!(fee.maximum_fee, 1_000_000);
    }

    #[test]
    fn read_transfer_fee_config_rejects_missing() {
        let data = vec![0u8; MINT_BASE_SIZE];
        assert!(read_transfer_fee_config(&data).is_err());
    }

    #[test]
    fn truncated_tlv_returns_none() {
        let mut data = vec![0u8; ACCOUNT_TYPE_OFFSET];
        data.push(ACCOUNT_TYPE_MINT);
        // Write type but length points past end.
        data.extend_from_slice(&EXT_TRANSFER_FEE_CONFIG.to_le_bytes());
        data.extend_from_slice(&200u16.to_le_bytes()); // claims 200 bytes
        data.extend_from_slice(&[0u8; 10]);            // only add 10
        assert!(!mint_has_extension(&data, EXT_TRANSFER_FEE_CONFIG));
    }

    #[test]
    fn rejects_reading_mint_extension_out_of_token_account() {
        // A real extended token account should not be treated as a
        // mint just because the caller passed the wrong base_size.
        // Historically this was silently wrong; now find_extension_data
        // must refuse the AccountType mismatch.
        let mut data = vec![0u8; ACCOUNT_TYPE_OFFSET];
        data.push(ACCOUNT_TYPE_TOKEN);
        data.extend_from_slice(&EXT_TRANSFER_FEE_AMOUNT.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        // Calling with MINT_BASE_SIZE must fail regardless of contents.
        assert!(find_extension_data(&data, MINT_BASE_SIZE, EXT_TRANSFER_FEE_AMOUNT).is_none());
        // And the token-account path must find it.
        assert!(
            find_extension_data(&data, TOKEN_ACCOUNT_BASE_SIZE, EXT_TRANSFER_FEE_AMOUNT).is_some()
        );
    }

    #[test]
    fn unknown_base_size_is_rejected() {
        // The old implementation accepted arbitrary base_size values
        // and walked attacker-controlled pointer arithmetic. The new
        // implementation refuses anything that is not one of the two
        // canonical shapes.
        let data = sample_mint_with_extension(EXT_NON_TRANSFERABLE, &[]);
        assert!(find_extension_data(&data, 42, EXT_NON_TRANSFERABLE).is_none());
        assert!(find_extension_data(&data, 0, EXT_NON_TRANSFERABLE).is_none());
    }
}
