//! Byte-exact instruction-data encoders for the Token-2022 CPI builders,
//! with Kani proofs that pin the wire format.
//!
//! Token-2022 shares the SPL Token instruction-data wire format for its
//! base instructions: a single discriminator byte followed (for the
//! amount-carrying instructions) by a little-endian `u64`. Pubkeys are
//! carried in the account-meta list, never in the instruction data, so
//! the data buffers proven here are small and fixed-size.
//!
//! These encoders are the **single source of truth** for the bytes the
//! builders in this crate emit: [`super::Transfer`], [`super::MintTo`],
//! [`super::Burn`], [`super::Approve`], [`super::Revoke`],
//! [`super::CloseAccount`], and [`super::InitializeAccount`] all call the
//! `encode_*` functions below to build their `data` buffer before handing
//! it to `hopper_runtime::cpi::invoke_signed`. Because the builders route
//! through these functions, the `#[cfg(kani)]` proofs verify the *actual*
//! bytes the CPI path emits — not a parallel re-implementation.
//!
//! Reference: `<spl-token>/src/instruction.rs` `TokenInstruction` tags
//! (Transfer = 3, InitializeAccount = 1, Approve = 4, Revoke = 5,
//! MintTo = 7, Burn = 8, CloseAccount = 9). Token-2022's dispatcher
//! accepts the identical base encoding.

/// `TokenInstruction::InitializeAccount` discriminator.
pub const IX_INITIALIZE_ACCOUNT: u8 = 1;
/// `TokenInstruction::Transfer` discriminator.
pub const IX_TRANSFER: u8 = 3;
/// `TokenInstruction::Approve` discriminator.
pub const IX_APPROVE: u8 = 4;
/// `TokenInstruction::Revoke` discriminator.
pub const IX_REVOKE: u8 = 5;
/// `TokenInstruction::MintTo` discriminator.
pub const IX_MINT_TO: u8 = 7;
/// `TokenInstruction::Burn` discriminator.
pub const IX_BURN: u8 = 8;
/// `TokenInstruction::CloseAccount` discriminator.
pub const IX_CLOSE_ACCOUNT: u8 = 9;

/// Encode `TokenInstruction::Transfer { amount }`.
///
/// Layout: `[3][amount: u64 LE]` — 9 bytes.
#[inline(always)]
pub fn encode_transfer(amount: u64) -> [u8; 9] {
    let mut data = [0u8; 9];
    data[0] = IX_TRANSFER;
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data
}

/// Encode `TokenInstruction::Approve { amount }`.
///
/// Layout: `[4][amount: u64 LE]` — 9 bytes.
#[inline(always)]
pub fn encode_approve(amount: u64) -> [u8; 9] {
    let mut data = [0u8; 9];
    data[0] = IX_APPROVE;
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data
}

/// Encode `TokenInstruction::MintTo { amount }`.
///
/// Layout: `[7][amount: u64 LE]` — 9 bytes.
#[inline(always)]
pub fn encode_mint_to(amount: u64) -> [u8; 9] {
    let mut data = [0u8; 9];
    data[0] = IX_MINT_TO;
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data
}

/// Encode `TokenInstruction::Burn { amount }`.
///
/// Layout: `[8][amount: u64 LE]` — 9 bytes.
#[inline(always)]
pub fn encode_burn(amount: u64) -> [u8; 9] {
    let mut data = [0u8; 9];
    data[0] = IX_BURN;
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data
}

/// Encode `TokenInstruction::Revoke`.
///
/// Layout: `[5]` — 1 byte.
#[inline(always)]
pub fn encode_revoke() -> [u8; 1] {
    [IX_REVOKE]
}

/// Encode `TokenInstruction::CloseAccount`.
///
/// Layout: `[9]` — 1 byte.
#[inline(always)]
pub fn encode_close_account() -> [u8; 1] {
    [IX_CLOSE_ACCOUNT]
}

/// Encode `TokenInstruction::InitializeAccount`.
///
/// Layout: `[1]` — 1 byte. The owner/mint/rent are carried in the
/// account-meta list, not the instruction data.
#[inline(always)]
pub fn encode_initialize_account() -> [u8; 1] {
    [IX_INITIALIZE_ACCOUNT]
}

// =====================================================================
// Golden-vector unit tests: pin the exact byte strings against the
// SPL Token / Token-2022 program wire format for concrete inputs.
// =====================================================================
#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    // A recognizable amount whose 8 LE bytes are all distinct, so a
    // byte-order or offset bug can't hide behind repeated bytes.
    const AMOUNT: u64 = 0x0102_0304_0506_0708;
    const AMOUNT_LE: [u8; 8] = [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01];

    #[test]
    fn transfer_golden() {
        let d = encode_transfer(AMOUNT);
        assert_eq!(d[0], 3);
        assert_eq!(&d[1..9], &AMOUNT_LE);
    }

    #[test]
    fn approve_golden() {
        let d = encode_approve(AMOUNT);
        assert_eq!(d[0], 4);
        assert_eq!(&d[1..9], &AMOUNT_LE);
    }

    #[test]
    fn mint_to_golden() {
        let d = encode_mint_to(AMOUNT);
        assert_eq!(d[0], 7);
        assert_eq!(&d[1..9], &AMOUNT_LE);
    }

    #[test]
    fn burn_golden() {
        let d = encode_burn(AMOUNT);
        assert_eq!(d[0], 8);
        assert_eq!(&d[1..9], &AMOUNT_LE);
    }

    #[test]
    fn single_byte_golden() {
        assert_eq!(encode_revoke(), [5]);
        assert_eq!(encode_close_account(), [9]);
        assert_eq!(encode_initialize_account(), [1]);
    }
}

// =====================================================================
// Kani layout proofs.
//
// For each amount-carrying encoder we prove, over a fully symbolic
// `amount`:
//   (i)   the discriminator byte is exactly the SPL/Token-2022 tag,
//   (ii)  the amount lands little-endian at byte offset 1 (compared as
//         one u64 word — never a 32-byte slice `==`, per the raw_input
//         PID_WORD lesson: word/field-wise comparisons keep the harness
//         free of memcmp loops that would blow up the SAT formula),
//   (iii) the total encoded length is exact.
// The single-byte encoders are deterministic (one path) so their
// discriminator + length verdicts are universal.
// =====================================================================
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Read the 8-byte little-endian window at offset 1 as a `u64`.
    #[inline]
    fn amount_word(d: &[u8; 9]) -> u64 {
        u64::from_le_bytes([d[1], d[2], d[3], d[4], d[5], d[6], d[7], d[8]])
    }

    #[kani::proof]
    fn transfer_discriminator() {
        let d = encode_transfer(kani::any());
        assert_eq!(d[0], 3);
    }

    #[kani::proof]
    fn transfer_amount_le_at_offset_1() {
        let amount: u64 = kani::any();
        assert_eq!(amount_word(&encode_transfer(amount)), amount);
    }

    #[kani::proof]
    fn transfer_len_is_9() {
        assert_eq!(encode_transfer(kani::any()).len(), 9);
    }

    #[kani::proof]
    fn approve_discriminator() {
        let d = encode_approve(kani::any());
        assert_eq!(d[0], 4);
    }

    #[kani::proof]
    fn approve_amount_le_at_offset_1() {
        let amount: u64 = kani::any();
        assert_eq!(amount_word(&encode_approve(amount)), amount);
    }

    #[kani::proof]
    fn approve_len_is_9() {
        assert_eq!(encode_approve(kani::any()).len(), 9);
    }

    #[kani::proof]
    fn mint_to_discriminator() {
        let d = encode_mint_to(kani::any());
        assert_eq!(d[0], 7);
    }

    #[kani::proof]
    fn mint_to_amount_le_at_offset_1() {
        let amount: u64 = kani::any();
        assert_eq!(amount_word(&encode_mint_to(amount)), amount);
    }

    #[kani::proof]
    fn mint_to_len_is_9() {
        assert_eq!(encode_mint_to(kani::any()).len(), 9);
    }

    #[kani::proof]
    fn burn_discriminator() {
        let d = encode_burn(kani::any());
        assert_eq!(d[0], 8);
    }

    #[kani::proof]
    fn burn_amount_le_at_offset_1() {
        let amount: u64 = kani::any();
        assert_eq!(amount_word(&encode_burn(amount)), amount);
    }

    #[kani::proof]
    fn burn_len_is_9() {
        assert_eq!(encode_burn(kani::any()).len(), 9);
    }

    #[kani::proof]
    fn revoke_is_single_byte_5() {
        let d = encode_revoke();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], 5);
    }

    #[kani::proof]
    fn close_account_is_single_byte_9() {
        let d = encode_close_account();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], 9);
    }

    #[kani::proof]
    fn initialize_account_is_single_byte_1() {
        let d = encode_initialize_account();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], 1);
    }
}
