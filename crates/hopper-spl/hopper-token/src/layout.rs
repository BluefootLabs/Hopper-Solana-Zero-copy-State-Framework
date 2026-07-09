//! Byte-exact instruction-data encoders for the SPL Token and System
//! program CPI wire formats, with Kani proofs pinning every field's
//! canonical byte offset, discriminator, and total length.
//!
//! ## Why these live here
//!
//! The SPL Token `*Checked` builders and the entire System-program
//! builder family ([`hopper_runtime::token`], [`hopper_runtime::system`])
//! build their instruction-data buffer *inline* inside private
//! `invoke_signed*` methods and pass it straight to
//! `hopper_runtime::cpi::invoke_signed` — the bytes are never returned,
//! so they cannot be observed (and therefore cannot be model-checked)
//! from outside that crate. `hopper-runtime` is not editable in this
//! change, so the encoders cannot be refactored to expose their output.
//!
//! This module reproduces those wire formats as small, pure, **callable**
//! encoders that `hopper-token` owns, and proves them with Kani. Two
//! independent anchors keep the reproduction honest:
//!
//! 1. **Golden-vector unit tests** (`#[cfg(test)]`) pin the exact byte
//!    strings against the SPL Token / System program specification for
//!    concrete inputs — the same discriminators and field order the
//!    on-chain programs dispatch on.
//! 2. Each encoder's doc comment cites the exact `hopper-runtime` source
//!    line that emits the identical bytes, so the mirror can be checked
//!    against the shipped builder by inspection.
//!
//! ### Provenance (runtime source lines at repo HEAD)
//!
//! SPL Token — `crates/hopper-runtime/src/token.rs`:
//! Transfer disc 3 (L401), Approve disc 4 (L599), MintTo disc 7 (L450),
//! Burn disc 8 (L498), TransferChecked disc 12 (L768), ApproveChecked
//! disc 13 (L1010), MintToChecked disc 14 (L836), BurnChecked disc 15
//! (L923), Revoke disc 5 (L659), CloseAccount disc 9 (L559),
//! FreezeAccount disc 10 (L1131), ThawAccount disc 11 (L1182),
//! SyncNative disc 17 (L1203), SetAuthority disc 6 (`encode_set_authority_data`,
//! L72), InitializeAccount2 disc 16 / InitializeAccount3 disc 18
//! (`encode_initialize_account_with_owner`, L89).
//!
//! System — `crates/hopper-runtime/src/system.rs`:
//! CreateAccount disc 0 (L45), Transfer disc 2 (L83), Assign disc 1
//! (L118), Allocate disc 8 (L150). System discriminators are 4-byte
//! `u32` LE (only the low byte is nonzero for these tags).
//!
//! ## Followup (requires editing the currently-locked hopper-runtime)
//!
//! The runtime builders should delegate to these functions so there is a
//! single proven source of truth and the proof covers the exact bytes the
//! CPI path emits. Until then, the "real-path" proofs (where the emitting
//! builder actually calls the proven encoder) live in the editable
//! `hopper-token-2022` and `hopper-associated-token` crates; the SPL
//! Token base wire format (Transfer/MintTo/Burn/Approve) is identical
//! between those crates and this module.

// =====================================================================
// SPL Token wire format.
// =====================================================================

/// SPL Token instruction discriminators (`TokenInstruction` tags).
pub mod spl_token {
    /// `InitializeAccount2` discriminator.
    pub const IX_INITIALIZE_ACCOUNT2: u8 = 16;
    /// `InitializeAccount3` discriminator.
    pub const IX_INITIALIZE_ACCOUNT3: u8 = 18;
    /// `Transfer` discriminator.
    pub const IX_TRANSFER: u8 = 3;
    /// `Approve` discriminator.
    pub const IX_APPROVE: u8 = 4;
    /// `Revoke` discriminator.
    pub const IX_REVOKE: u8 = 5;
    /// `SetAuthority` discriminator.
    pub const IX_SET_AUTHORITY: u8 = 6;
    /// `MintTo` discriminator.
    pub const IX_MINT_TO: u8 = 7;
    /// `Burn` discriminator.
    pub const IX_BURN: u8 = 8;
    /// `CloseAccount` discriminator.
    pub const IX_CLOSE_ACCOUNT: u8 = 9;
    /// `FreezeAccount` discriminator.
    pub const IX_FREEZE_ACCOUNT: u8 = 10;
    /// `ThawAccount` discriminator.
    pub const IX_THAW_ACCOUNT: u8 = 11;
    /// `TransferChecked` discriminator.
    pub const IX_TRANSFER_CHECKED: u8 = 12;
    /// `ApproveChecked` discriminator.
    pub const IX_APPROVE_CHECKED: u8 = 13;
    /// `MintToChecked` discriminator.
    pub const IX_MINT_TO_CHECKED: u8 = 14;
    /// `BurnChecked` discriminator.
    pub const IX_BURN_CHECKED: u8 = 15;
    /// `SyncNative` discriminator.
    pub const IX_SYNC_NATIVE: u8 = 17;

    #[inline(always)]
    fn amount_ix(disc: u8, amount: u64) -> [u8; 9] {
        let mut data = [0u8; 9];
        data[0] = disc;
        data[1..9].copy_from_slice(&amount.to_le_bytes());
        data
    }

    #[inline(always)]
    fn amount_checked_ix(disc: u8, amount: u64, decimals: u8) -> [u8; 10] {
        let mut data = [0u8; 10];
        data[0] = disc;
        data[1..9].copy_from_slice(&amount.to_le_bytes());
        data[9] = decimals;
        data
    }

    /// `Transfer { amount }` — `[3][amount: u64 LE]` (9 bytes).
    /// Mirrors `token.rs` L401-403.
    #[inline(always)]
    pub fn encode_transfer(amount: u64) -> [u8; 9] {
        amount_ix(IX_TRANSFER, amount)
    }

    /// `Approve { amount }` — `[4][amount: u64 LE]` (9 bytes).
    /// Mirrors `token.rs` L598-600.
    #[inline(always)]
    pub fn encode_approve(amount: u64) -> [u8; 9] {
        amount_ix(IX_APPROVE, amount)
    }

    /// `MintTo { amount }` — `[7][amount: u64 LE]` (9 bytes).
    /// Mirrors `token.rs` L449-451.
    #[inline(always)]
    pub fn encode_mint_to(amount: u64) -> [u8; 9] {
        amount_ix(IX_MINT_TO, amount)
    }

    /// `Burn { amount }` — `[8][amount: u64 LE]` (9 bytes).
    /// Mirrors `token.rs` L497-499.
    #[inline(always)]
    pub fn encode_burn(amount: u64) -> [u8; 9] {
        amount_ix(IX_BURN, amount)
    }

    /// `TransferChecked { amount, decimals }` —
    /// `[12][amount: u64 LE][decimals: u8]` (10 bytes).
    /// Mirrors `token.rs` L767-770.
    #[inline(always)]
    pub fn encode_transfer_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_TRANSFER_CHECKED, amount, decimals)
    }

    /// `ApproveChecked { amount, decimals }` —
    /// `[13][amount: u64 LE][decimals: u8]` (10 bytes).
    /// Mirrors `token.rs` L1009-1012.
    #[inline(always)]
    pub fn encode_approve_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_APPROVE_CHECKED, amount, decimals)
    }

    /// `MintToChecked { amount, decimals }` —
    /// `[14][amount: u64 LE][decimals: u8]` (10 bytes).
    /// Mirrors `token.rs` L835-838.
    #[inline(always)]
    pub fn encode_mint_to_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_MINT_TO_CHECKED, amount, decimals)
    }

    /// `BurnChecked { amount, decimals }` —
    /// `[15][amount: u64 LE][decimals: u8]` (10 bytes).
    /// Mirrors `token.rs` L922-925.
    #[inline(always)]
    pub fn encode_burn_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_BURN_CHECKED, amount, decimals)
    }

    /// `Revoke` — `[5]` (1 byte). Mirrors `token.rs` L659.
    #[inline(always)]
    pub fn encode_revoke() -> [u8; 1] {
        [IX_REVOKE]
    }

    /// `CloseAccount` — `[9]` (1 byte). Mirrors `token.rs` L559.
    #[inline(always)]
    pub fn encode_close_account() -> [u8; 1] {
        [IX_CLOSE_ACCOUNT]
    }

    /// `FreezeAccount` — `[10]` (1 byte). Mirrors `token.rs` L1131.
    #[inline(always)]
    pub fn encode_freeze_account() -> [u8; 1] {
        [IX_FREEZE_ACCOUNT]
    }

    /// `ThawAccount` — `[11]` (1 byte). Mirrors `token.rs` L1182.
    #[inline(always)]
    pub fn encode_thaw_account() -> [u8; 1] {
        [IX_THAW_ACCOUNT]
    }

    /// `SyncNative` — `[17]` (1 byte). Mirrors `token.rs` L1203.
    #[inline(always)]
    pub fn encode_sync_native() -> [u8; 1] {
        [IX_SYNC_NATIVE]
    }

    /// `InitializeAccount2 { owner }` — `[16][owner: 32 bytes]` (33 bytes).
    /// Mirrors `encode_initialize_account_with_owner(16, owner)`
    /// (`token.rs` L89-95, L1252).
    #[inline(always)]
    pub fn encode_initialize_account2(owner: &[u8; 32]) -> [u8; 33] {
        let mut data = [0u8; 33];
        data[0] = IX_INITIALIZE_ACCOUNT2;
        data[1..33].copy_from_slice(owner);
        data
    }

    /// `InitializeAccount3 { owner }` — `[18][owner: 32 bytes]` (33 bytes).
    /// Mirrors `encode_initialize_account_with_owner(18, owner)`
    /// (`token.rs` L89-95, L1273).
    #[inline(always)]
    pub fn encode_initialize_account3(owner: &[u8; 32]) -> [u8; 33] {
        let mut data = [0u8; 33];
        data[0] = IX_INITIALIZE_ACCOUNT3;
        data[1..33].copy_from_slice(owner);
        data
    }

    /// `SetAuthority { authority_type, new_authority }`.
    ///
    /// Layout: `[6][authority_type: u8][COption tag: u8]` and, when
    /// `new_authority` is `Some`, `[new_authority: 32 bytes]`. The tag
    /// byte is 1 (Some) or 0 (None). Returns the fixed 35-byte buffer
    /// and the number of meaningful bytes (35 for Some, 3 for None).
    /// Mirrors `encode_set_authority_data` (`token.rs` L71-87).
    #[inline(always)]
    pub fn encode_set_authority(
        authority_type: u8,
        new_authority: Option<&[u8; 32]>,
    ) -> ([u8; 35], usize) {
        let mut data = [0u8; 35];
        data[0] = IX_SET_AUTHORITY;
        data[1] = authority_type;
        match new_authority {
            Some(key) => {
                data[2] = 1;
                data[3..35].copy_from_slice(key);
                (data, 35)
            }
            None => {
                data[2] = 0;
                (data, 3)
            }
        }
    }
}

// =====================================================================
// System program wire format. Discriminators are 4-byte u32 LE.
// =====================================================================

/// System program instruction encoders.
pub mod system {
    /// `CreateAccount` discriminator (u32 LE).
    pub const IX_CREATE_ACCOUNT: u32 = 0;
    /// `Assign` discriminator (u32 LE).
    pub const IX_ASSIGN: u32 = 1;
    /// `Transfer` discriminator (u32 LE).
    pub const IX_TRANSFER: u32 = 2;
    /// `Allocate` discriminator (u32 LE).
    pub const IX_ALLOCATE: u32 = 8;

    /// `CreateAccount { lamports, space, owner }`.
    ///
    /// Layout: `[disc: u32 LE = 0][lamports: u64 LE][space: u64 LE]
    /// [owner: 32 bytes]` — 52 bytes. Mirrors `system.rs` L45-49.
    #[inline(always)]
    pub fn encode_create_account(lamports: u64, space: u64, owner: &[u8; 32]) -> [u8; 52] {
        let mut data = [0u8; 52];
        data[0..4].copy_from_slice(&IX_CREATE_ACCOUNT.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data[12..20].copy_from_slice(&space.to_le_bytes());
        data[20..52].copy_from_slice(owner);
        data
    }

    /// `Transfer { lamports }`.
    ///
    /// Layout: `[disc: u32 LE = 2][lamports: u64 LE]` — 12 bytes.
    /// Mirrors `system.rs` L83-85.
    #[inline(always)]
    pub fn encode_transfer(lamports: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&IX_TRANSFER.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data
    }

    /// `Assign { owner }`.
    ///
    /// Layout: `[disc: u32 LE = 1][owner: 32 bytes]` — 36 bytes.
    /// Mirrors `system.rs` L118-120.
    #[inline(always)]
    pub fn encode_assign(owner: &[u8; 32]) -> [u8; 36] {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&IX_ASSIGN.to_le_bytes());
        data[4..36].copy_from_slice(owner);
        data
    }

    /// `Allocate { space }`.
    ///
    /// Layout: `[disc: u32 LE = 8][space: u64 LE]` — 12 bytes.
    /// Mirrors `system.rs` L150-152.
    #[inline(always)]
    pub fn encode_allocate(space: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&IX_ALLOCATE.to_le_bytes());
        data[4..12].copy_from_slice(&space.to_le_bytes());
        data
    }
}

// =====================================================================
// Golden-vector unit tests: pin exact byte strings against the SPL
// Token / System program specification.
// =====================================================================
#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    const AMOUNT: u64 = 0x0102_0304_0506_0708;
    const AMOUNT_LE: [u8; 8] = [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01];
    // Distinct per-byte pattern so an offset/order bug is visible.
    const OWNER: [u8; 32] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31,
    ];

    #[test]
    fn spl_amount_instructions_golden() {
        for (bytes, disc) in [
            (spl_token::encode_transfer(AMOUNT), 3u8),
            (spl_token::encode_approve(AMOUNT), 4),
            (spl_token::encode_mint_to(AMOUNT), 7),
            (spl_token::encode_burn(AMOUNT), 8),
        ] {
            assert_eq!(bytes[0], disc);
            assert_eq!(&bytes[1..9], &AMOUNT_LE);
        }
    }

    #[test]
    fn spl_checked_instructions_golden() {
        for (bytes, disc) in [
            (spl_token::encode_transfer_checked(AMOUNT, 9), 12u8),
            (spl_token::encode_approve_checked(AMOUNT, 9), 13),
            (spl_token::encode_mint_to_checked(AMOUNT, 9), 14),
            (spl_token::encode_burn_checked(AMOUNT, 9), 15),
        ] {
            assert_eq!(bytes[0], disc);
            assert_eq!(&bytes[1..9], &AMOUNT_LE);
            assert_eq!(bytes[9], 9);
        }
    }

    #[test]
    fn spl_single_byte_instructions_golden() {
        assert_eq!(spl_token::encode_revoke(), [5]);
        assert_eq!(spl_token::encode_close_account(), [9]);
        assert_eq!(spl_token::encode_freeze_account(), [10]);
        assert_eq!(spl_token::encode_thaw_account(), [11]);
        assert_eq!(spl_token::encode_sync_native(), [17]);
    }

    #[test]
    fn spl_initialize_account_golden() {
        let a2 = spl_token::encode_initialize_account2(&OWNER);
        assert_eq!(a2[0], 16);
        assert_eq!(&a2[1..33], &OWNER);
        let a3 = spl_token::encode_initialize_account3(&OWNER);
        assert_eq!(a3[0], 18);
        assert_eq!(&a3[1..33], &OWNER);
    }

    #[test]
    fn spl_set_authority_golden() {
        let (some, some_len) = spl_token::encode_set_authority(2, Some(&OWNER));
        assert_eq!(some_len, 35);
        assert_eq!(some[0], 6);
        assert_eq!(some[1], 2);
        assert_eq!(some[2], 1);
        assert_eq!(&some[3..35], &OWNER);

        let (none, none_len) = spl_token::encode_set_authority(3, None);
        assert_eq!(none_len, 3);
        assert_eq!(none[0], 6);
        assert_eq!(none[1], 3);
        assert_eq!(none[2], 0);
    }

    #[test]
    fn system_create_account_golden() {
        let d = system::encode_create_account(AMOUNT, 165, &OWNER);
        assert_eq!(&d[0..4], &[0, 0, 0, 0]);
        assert_eq!(&d[4..12], &AMOUNT_LE);
        assert_eq!(&d[12..20], &165u64.to_le_bytes());
        assert_eq!(&d[20..52], &OWNER);
    }

    #[test]
    fn system_transfer_golden() {
        let d = system::encode_transfer(AMOUNT);
        assert_eq!(&d[0..4], &[2, 0, 0, 0]);
        assert_eq!(&d[4..12], &AMOUNT_LE);
    }

    #[test]
    fn system_assign_golden() {
        let d = system::encode_assign(&OWNER);
        assert_eq!(&d[0..4], &[1, 0, 0, 0]);
        assert_eq!(&d[4..36], &OWNER);
    }

    #[test]
    fn system_allocate_golden() {
        let d = system::encode_allocate(AMOUNT);
        assert_eq!(&d[0..4], &[8, 0, 0, 0]);
        assert_eq!(&d[4..12], &AMOUNT_LE);
    }
}

// =====================================================================
// Kani layout proofs.
//
// Conventions (per the raw_input PID_WORD lesson):
//   * scalars are compared as one word (`u64::from_le_bytes(...)`),
//     never a slice `==`;
//   * a 32-byte pubkey region is proven with a single symbolic index
//     `i in 0..32` and one assertion `data[off + i] == owner[i]`, which
//     universally covers all 32 offsets without a 32-iteration loop.
//
// Run by `scripts/kani-spl-layouts.{sh,ps1}` (CI lane
// `kani-spl-layout-proofs`).
// =====================================================================
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[inline]
    fn word_at(d: &[u8], off: usize) -> u64 {
        u64::from_le_bytes([
            d[off],
            d[off + 1],
            d[off + 2],
            d[off + 3],
            d[off + 4],
            d[off + 5],
            d[off + 6],
            d[off + 7],
        ])
    }

    /// Symbolic-index equality over a 32-byte region: universal over all
    /// offsets, no loop.
    #[inline]
    fn assert_pubkey_region(d: &[u8], off: usize, owner: &[u8; 32]) {
        let i: usize = kani::any();
        kani::assume(i < 32);
        assert_eq!(d[off + i], owner[i]);
    }

    // ── SPL Token: amount instructions ──────────────────────────────

    #[kani::proof]
    fn spl_transfer_layout() {
        let amount: u64 = kani::any();
        let d = spl_token::encode_transfer(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 3);
        assert_eq!(word_at(&d, 1), amount);
    }

    #[kani::proof]
    fn spl_approve_layout() {
        let amount: u64 = kani::any();
        let d = spl_token::encode_approve(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 4);
        assert_eq!(word_at(&d, 1), amount);
    }

    #[kani::proof]
    fn spl_mint_to_layout() {
        let amount: u64 = kani::any();
        let d = spl_token::encode_mint_to(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 7);
        assert_eq!(word_at(&d, 1), amount);
    }

    #[kani::proof]
    fn spl_burn_layout() {
        let amount: u64 = kani::any();
        let d = spl_token::encode_burn(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 8);
        assert_eq!(word_at(&d, 1), amount);
    }

    // ── SPL Token: checked instructions ─────────────────────────────

    #[kani::proof]
    fn spl_transfer_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = spl_token::encode_transfer_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 12);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    #[kani::proof]
    fn spl_approve_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = spl_token::encode_approve_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 13);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    #[kani::proof]
    fn spl_mint_to_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = spl_token::encode_mint_to_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 14);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    #[kani::proof]
    fn spl_burn_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = spl_token::encode_burn_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 15);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    // ── SPL Token: single-byte instructions (deterministic) ─────────

    #[kani::proof]
    fn spl_single_byte_discriminators() {
        assert_eq!(spl_token::encode_revoke(), [5]);
        assert_eq!(spl_token::encode_close_account(), [9]);
        assert_eq!(spl_token::encode_freeze_account(), [10]);
        assert_eq!(spl_token::encode_thaw_account(), [11]);
        assert_eq!(spl_token::encode_sync_native(), [17]);
    }

    // ── SPL Token: initialize-account-with-owner ────────────────────

    #[kani::proof]
    fn spl_initialize_account2_layout() {
        let owner: [u8; 32] = kani::any();
        let d = spl_token::encode_initialize_account2(&owner);
        assert_eq!(d.len(), 33);
        assert_eq!(d[0], 16);
        assert_pubkey_region(&d, 1, &owner);
    }

    #[kani::proof]
    fn spl_initialize_account3_layout() {
        let owner: [u8; 32] = kani::any();
        let d = spl_token::encode_initialize_account3(&owner);
        assert_eq!(d.len(), 33);
        assert_eq!(d[0], 18);
        assert_pubkey_region(&d, 1, &owner);
    }

    // ── SPL Token: SetAuthority (both COption branches) ─────────────

    #[kani::proof]
    fn spl_set_authority_some_layout() {
        let authority_type: u8 = kani::any();
        let owner: [u8; 32] = kani::any();
        let (d, len) = spl_token::encode_set_authority(authority_type, Some(&owner));
        assert_eq!(len, 35);
        assert_eq!(d[0], 6);
        assert_eq!(d[1], authority_type);
        assert_eq!(d[2], 1);
        assert_pubkey_region(&d, 3, &owner);
    }

    #[kani::proof]
    fn spl_set_authority_none_layout() {
        let authority_type: u8 = kani::any();
        let (d, len) = spl_token::encode_set_authority(authority_type, None);
        assert_eq!(len, 3);
        assert_eq!(d[0], 6);
        assert_eq!(d[1], authority_type);
        assert_eq!(d[2], 0);
    }

    // ── System program ──────────────────────────────────────────────

    /// The 4-byte u32 discriminator: low byte = tag, high 3 bytes = 0.
    #[inline]
    fn assert_system_disc(d: &[u8], tag: u8) {
        assert_eq!(d[0], tag);
        assert_eq!(d[1], 0);
        assert_eq!(d[2], 0);
        assert_eq!(d[3], 0);
    }

    #[kani::proof]
    fn system_transfer_layout() {
        let lamports: u64 = kani::any();
        let d = system::encode_transfer(lamports);
        assert_eq!(d.len(), 12);
        assert_system_disc(&d, 2);
        assert_eq!(word_at(&d, 4), lamports);
    }

    #[kani::proof]
    fn system_allocate_layout() {
        let space: u64 = kani::any();
        let d = system::encode_allocate(space);
        assert_eq!(d.len(), 12);
        assert_system_disc(&d, 8);
        assert_eq!(word_at(&d, 4), space);
    }

    #[kani::proof]
    fn system_assign_layout() {
        let owner: [u8; 32] = kani::any();
        let d = system::encode_assign(&owner);
        assert_eq!(d.len(), 36);
        assert_system_disc(&d, 1);
        assert_pubkey_region(&d, 4, &owner);
    }

    #[kani::proof]
    fn system_create_account_layout() {
        let lamports: u64 = kani::any();
        let space: u64 = kani::any();
        let owner: [u8; 32] = kani::any();
        let d = system::encode_create_account(lamports, space, &owner);
        assert_eq!(d.len(), 52);
        assert_system_disc(&d, 0);
        assert_eq!(word_at(&d, 4), lamports);
        assert_eq!(word_at(&d, 12), space);
        assert_pubkey_region(&d, 20, &owner);
    }
}
