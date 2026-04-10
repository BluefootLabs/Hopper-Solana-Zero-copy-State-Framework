//! Token-2022 extension screening.
//!
//! Parse the TLV (Type-Length-Value) extension area that follows the base
//! 165-byte mint or 82-byte token account layout. Provides both individual
//! extension readers and blanket safety checks.
//!
//! ## Token-2022 TLV Format
//!
//! After the base account data (82 bytes for token, 165 bytes for mint),
//! there is an optional padding byte, then a discriminator byte (`0x01` for
//! account extensions, `0x02` for mint extensions), followed by TLV entries:
//!
//! ```text
//!   [u16 LE type] [u16 LE length] [length bytes value]
//! ```
//!
//! Extensions are concatenated. The type determines which extension the
//! TLV entry represents.

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

/// Base token account data size (before extensions).
pub const TOKEN_ACCOUNT_BASE_SIZE: usize = 165;

/// Account type discriminator byte for mint accounts.
#[cfg(test)]
const ACCOUNT_TYPE_MINT: u8 = 2;

// ── TLV Parsing ──────────────────────────────────────────────────────────────

/// Find the first TLV entry of `ext_type` in a Token-2022 account's data.
///
/// Returns the byte slice of the extension value, or `None` if not found.
/// Works for both mint (base 82 bytes) and token (base 165 bytes) accounts.
#[inline(always)]
pub fn find_extension_data(data: &[u8], base_size: usize, ext_type: u16) -> Option<&[u8]> {
    // After base data: 1 padding byte + 1 account type byte = +2 bytes overhead
    let tlv_start = base_size + 1 + 1;
    if data.len() < tlv_start {
        return None;
    }

    let mut offset = tlv_start;
    while offset + 4 <= data.len() {
        let ty = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let len = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as usize;
        let value_start = offset + 4;
        let value_end = value_start + len;

        if value_end > data.len() {
            return None; // Truncated TLV
        }

        if ty == ext_type {
            return Some(&data[value_start..value_end]);
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

    /// Build sample mint data with one TLV extension.
    fn sample_mint_with_extension(ext_type: u16, ext_value: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; MINT_BASE_SIZE]; // base mint data
        data.push(0); // padding
        data.push(ACCOUNT_TYPE_MINT); // account type
        // TLV: type (2) + length (2) + value
        data.extend_from_slice(&ext_type.to_le_bytes());
        data.extend_from_slice(&(ext_value.len() as u16).to_le_bytes());
        data.extend_from_slice(ext_value);
        data
    }

    /// Build sample mint data with multiple TLV extensions.
    fn sample_mint_with_extensions(exts: &[(u16, &[u8])]) -> Vec<u8> {
        let mut data = vec![0u8; MINT_BASE_SIZE];
        data.push(0);
        data.push(ACCOUNT_TYPE_MINT);
        for (ext_type, ext_value) in exts {
            data.extend_from_slice(&ext_type.to_le_bytes());
            data.extend_from_slice(&(ext_value.len() as u16).to_le_bytes());
            data.extend_from_slice(ext_value);
        }
        data
    }

    #[test]
    fn no_extensions_passes_all_checks() {
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
        let mut data = vec![0u8; MINT_BASE_SIZE];
        data.push(0);
        data.push(ACCOUNT_TYPE_MINT);
        // Write type but length points past end
        data.extend_from_slice(&EXT_TRANSFER_FEE_CONFIG.to_le_bytes());
        data.extend_from_slice(&200u16.to_le_bytes()); // claims 200 bytes
        // But only add 10 bytes
        data.extend_from_slice(&[0u8; 10]);

        assert!(!mint_has_extension(&data, EXT_TRANSFER_FEE_CONFIG));
    }
}
