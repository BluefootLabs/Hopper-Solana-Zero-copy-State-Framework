//! Kani proofs and golden-vector tests for the **shipped** SPL Token and
//! System-program CPI instruction-data encoders.
//!
//! ## What is proven, and where the bytes come from
//!
//! The SPL Token builders and the System-program builders in
//! [`hopper_runtime::token`] / [`hopper_runtime::system`] now construct every
//! instruction-data buffer by calling a small, pure, `#[inline(always)]`
//! encoder in `hopper_runtime::{token,system}::encoders`. Those encoders
//! return the exact bytes that leave the program on a CPI. Because a builder's
//! `invoke_signed*` path delegates to one of them (instead of building the
//! buffer inline and dropping it straight into the syscall), the shipped bytes
//! are directly callable — and therefore directly model-checkable.
//!
//! The `#[cfg(kani)]` proofs below call the shipped
//! `hopper_runtime::{token,system}::encoders::*` functions **directly** and
//! pin, over fully symbolic inputs, each field's discriminator, byte offset,
//! endianness, and the total encoded length. This is the change that retires
//! the previous batch's caveat: those encoders used to build their buffer
//! inline inside a private `invoke_signed*` method and never return it, so
//! from outside `hopper-runtime` they were unobservable and this module could
//! only prove a hand-written *mirror* of the wire format. The mirror is gone
//! as the proof target; the proofs now verify the code that actually runs.
//!
//! ## The reference encoders are now a differential oracle
//!
//! The [`spl_token`] and [`system`] modules in this file retain an
//! independent, second implementation of the same wire formats. They are no
//! longer the proof target — they serve as a **differential oracle**. The
//! `*_matches_reference` proofs assert, byte-for-byte over symbolic inputs,
//! that the shipped encoder equals this independent reference, so the shipped
//! bytes are pinned from two directions at once: a direct wire-format proof
//! *and* N-version agreement with a separately-authored encoder. A divergence
//! in either implementation fails a proof.
//!
//! ## CBMC tractability
//!
//! Per the `raw_input` `PID_WORD` lesson, scalars are compared as one word
//! (`u64::from_le_bytes(..)`), never a slice `==`, and a 32-byte pubkey region
//! is proven with a single symbolic index `i in 0..32` and one assertion —
//! universal over all offsets, with no 32-iteration `memcmp` loop that would
//! blow up the SAT formula. The differential proofs use the same discipline
//! (field-wise / symbolic-index equality between the two fixed-size buffers),
//! so every harness stays free of large comparison loops.
//!
//! ## Provenance of the shipped encoders (runtime source)
//!
//! SPL Token — `crates/hopper-runtime/src/token.rs`, `pub mod encoders`:
//! Transfer 3, Approve 4, MintTo 7, Burn 8, TransferChecked 12,
//! ApproveChecked 13, MintToChecked 14, BurnChecked 15, Revoke 5,
//! CloseAccount 9, FreezeAccount 10, ThawAccount 11, SyncNative 17,
//! InitializeAccount 1, InitializeAccount2 16 / InitializeAccount3 18
//! (`encode_initialize_account_with_owner`), SetAuthority 6
//! (`encode_set_authority`).
//!
//! System — `crates/hopper-runtime/src/system.rs`, `pub mod encoders`:
//! CreateAccount 0, Transfer 2, Assign 1, Allocate 8. System discriminators
//! are 4-byte `u32` LE (only the low byte is nonzero for these tags).

// =====================================================================
// Differential-oracle reference encoders (SPL Token wire format).
//
// These are an independent second implementation kept only as a proof
// oracle: the `#[cfg(kani)]` `*_matches_reference` harnesses prove the
// shipped `hopper_runtime::token::encoders` byte-equal to these for all
// symbolic inputs. Do not call these from production code — use the
// builders in `hopper_runtime::token`.
// =====================================================================

/// SPL Token instruction discriminators (`TokenInstruction` tags), plus an
/// independent reference encoder per instruction used as a differential
/// oracle for the shipped `hopper_runtime::token::encoders`.
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

    /// Reference `Transfer { amount }` — `[3][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_transfer(amount: u64) -> [u8; 9] {
        amount_ix(IX_TRANSFER, amount)
    }

    /// Reference `Approve { amount }` — `[4][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_approve(amount: u64) -> [u8; 9] {
        amount_ix(IX_APPROVE, amount)
    }

    /// Reference `MintTo { amount }` — `[7][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_mint_to(amount: u64) -> [u8; 9] {
        amount_ix(IX_MINT_TO, amount)
    }

    /// Reference `Burn { amount }` — `[8][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_burn(amount: u64) -> [u8; 9] {
        amount_ix(IX_BURN, amount)
    }

    /// Reference `TransferChecked { amount, decimals }` —
    /// `[12][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_transfer_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_TRANSFER_CHECKED, amount, decimals)
    }

    /// Reference `ApproveChecked { amount, decimals }` —
    /// `[13][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_approve_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_APPROVE_CHECKED, amount, decimals)
    }

    /// Reference `MintToChecked { amount, decimals }` —
    /// `[14][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_mint_to_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_MINT_TO_CHECKED, amount, decimals)
    }

    /// Reference `BurnChecked { amount, decimals }` —
    /// `[15][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_burn_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(IX_BURN_CHECKED, amount, decimals)
    }

    /// Reference `Revoke` — `[5]` (1 byte).
    #[inline(always)]
    pub fn encode_revoke() -> [u8; 1] {
        [IX_REVOKE]
    }

    /// Reference `CloseAccount` — `[9]` (1 byte).
    #[inline(always)]
    pub fn encode_close_account() -> [u8; 1] {
        [IX_CLOSE_ACCOUNT]
    }

    /// Reference `FreezeAccount` — `[10]` (1 byte).
    #[inline(always)]
    pub fn encode_freeze_account() -> [u8; 1] {
        [IX_FREEZE_ACCOUNT]
    }

    /// Reference `ThawAccount` — `[11]` (1 byte).
    #[inline(always)]
    pub fn encode_thaw_account() -> [u8; 1] {
        [IX_THAW_ACCOUNT]
    }

    /// Reference `SyncNative` — `[17]` (1 byte).
    #[inline(always)]
    pub fn encode_sync_native() -> [u8; 1] {
        [IX_SYNC_NATIVE]
    }

    /// Reference `InitializeAccount2 { owner }` — `[16][owner: 32 bytes]`
    /// (33 bytes).
    #[inline(always)]
    pub fn encode_initialize_account2(owner: &[u8; 32]) -> [u8; 33] {
        let mut data = [0u8; 33];
        data[0] = IX_INITIALIZE_ACCOUNT2;
        data[1..33].copy_from_slice(owner);
        data
    }

    /// Reference `InitializeAccount3 { owner }` — `[18][owner: 32 bytes]`
    /// (33 bytes).
    #[inline(always)]
    pub fn encode_initialize_account3(owner: &[u8; 32]) -> [u8; 33] {
        let mut data = [0u8; 33];
        data[0] = IX_INITIALIZE_ACCOUNT3;
        data[1..33].copy_from_slice(owner);
        data
    }

    /// Reference `SetAuthority { authority_type, new_authority }`.
    ///
    /// Layout: `[6][authority_type: u8][COption tag: u8]` and, when
    /// `new_authority` is `Some`, `[new_authority: 32 bytes]`. The tag byte
    /// is 1 (Some) or 0 (None). Returns the fixed 35-byte buffer and the
    /// number of meaningful bytes (35 for Some, 3 for None).
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
// Differential-oracle reference encoders (System wire format).
// Discriminators are 4-byte u32 LE.
// =====================================================================

/// System program instruction reference encoders, used as a differential
/// oracle for the shipped `hopper_runtime::system::encoders`.
pub mod system {
    /// `CreateAccount` discriminator (u32 LE).
    pub const IX_CREATE_ACCOUNT: u32 = 0;
    /// `Assign` discriminator (u32 LE).
    pub const IX_ASSIGN: u32 = 1;
    /// `Transfer` discriminator (u32 LE).
    pub const IX_TRANSFER: u32 = 2;
    /// `Allocate` discriminator (u32 LE).
    pub const IX_ALLOCATE: u32 = 8;

    /// Reference `CreateAccount { lamports, space, owner }`.
    ///
    /// Layout: `[disc: u32 LE = 0][lamports: u64 LE][space: u64 LE]
    /// [owner: 32 bytes]` — 52 bytes.
    #[inline(always)]
    pub fn encode_create_account(lamports: u64, space: u64, owner: &[u8; 32]) -> [u8; 52] {
        let mut data = [0u8; 52];
        data[0..4].copy_from_slice(&IX_CREATE_ACCOUNT.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data[12..20].copy_from_slice(&space.to_le_bytes());
        data[20..52].copy_from_slice(owner);
        data
    }

    /// Reference `Transfer { lamports }`.
    ///
    /// Layout: `[disc: u32 LE = 2][lamports: u64 LE]` — 12 bytes.
    #[inline(always)]
    pub fn encode_transfer(lamports: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&IX_TRANSFER.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data
    }

    /// Reference `Assign { owner }`.
    ///
    /// Layout: `[disc: u32 LE = 1][owner: 32 bytes]` — 36 bytes.
    #[inline(always)]
    pub fn encode_assign(owner: &[u8; 32]) -> [u8; 36] {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&IX_ASSIGN.to_le_bytes());
        data[4..36].copy_from_slice(owner);
        data
    }

    /// Reference `Allocate { space }`.
    ///
    /// Layout: `[disc: u32 LE = 8][space: u64 LE]` — 12 bytes.
    #[inline(always)]
    pub fn encode_allocate(space: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&IX_ALLOCATE.to_le_bytes());
        data[4..12].copy_from_slice(&space.to_le_bytes());
        data
    }

    // ── Durable-nonce family (fixed-size, seed-free) ────────────────

    /// `AdvanceNonceAccount` discriminator (u32 LE).
    pub const IX_ADVANCE_NONCE: u32 = 4;
    /// `WithdrawNonceAccount` discriminator (u32 LE).
    pub const IX_WITHDRAW_NONCE: u32 = 5;
    /// `InitializeNonceAccount` discriminator (u32 LE).
    pub const IX_INITIALIZE_NONCE: u32 = 6;
    /// `AuthorizeNonceAccount` discriminator (u32 LE).
    pub const IX_AUTHORIZE_NONCE: u32 = 7;
    /// `UpgradeNonceAccount` discriminator (u32 LE).
    pub const IX_UPGRADE_NONCE: u32 = 12;

    /// Reference `AdvanceNonceAccount` — `[4u32 LE]` — 4 bytes.
    #[inline(always)]
    pub fn encode_advance_nonce_account() -> [u8; 4] {
        IX_ADVANCE_NONCE.to_le_bytes()
    }

    /// Reference `WithdrawNonceAccount { lamports }` —
    /// `[5u32 LE][lamports: u64 LE]` — 12 bytes.
    #[inline(always)]
    pub fn encode_withdraw_nonce_account(lamports: u64) -> [u8; 12] {
        let mut data = [0u8; 12];
        data[0..4].copy_from_slice(&IX_WITHDRAW_NONCE.to_le_bytes());
        data[4..12].copy_from_slice(&lamports.to_le_bytes());
        data
    }

    /// Reference `InitializeNonceAccount { authority }` —
    /// `[6u32 LE][authority: 32 bytes]` — 36 bytes.
    #[inline(always)]
    pub fn encode_initialize_nonce_account(authority: &[u8; 32]) -> [u8; 36] {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&IX_INITIALIZE_NONCE.to_le_bytes());
        data[4..36].copy_from_slice(authority);
        data
    }

    /// Reference `AuthorizeNonceAccount { new_authority }` —
    /// `[7u32 LE][new_authority: 32 bytes]` — 36 bytes.
    #[inline(always)]
    pub fn encode_authorize_nonce_account(new_authority: &[u8; 32]) -> [u8; 36] {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&IX_AUTHORIZE_NONCE.to_le_bytes());
        data[4..36].copy_from_slice(new_authority);
        data
    }

    /// Reference `UpgradeNonceAccount` — `[12u32 LE]` — 4 bytes.
    #[inline(always)]
    pub fn encode_upgrade_nonce_account() -> [u8; 4] {
        IX_UPGRADE_NONCE.to_le_bytes()
    }
}

// =====================================================================
// Golden-vector unit tests.
//
// These pin the exact byte strings the *shipped* encoders emit against the
// SPL Token / System program specification for concrete inputs, and anchor
// the differential oracle by asserting the reference encoders agree with the
// shipped bytes at the same vectors.
// =====================================================================
#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use hopper_runtime::system::encoders as rt_system;
    use hopper_runtime::token::encoders as rt_token;

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
            (rt_token::encode_transfer(AMOUNT), 3u8),
            (rt_token::encode_approve(AMOUNT), 4),
            (rt_token::encode_mint_to(AMOUNT), 7),
            (rt_token::encode_burn(AMOUNT), 8),
        ] {
            assert_eq!(bytes[0], disc);
            assert_eq!(&bytes[1..9], &AMOUNT_LE);
        }
    }

    #[test]
    fn spl_checked_instructions_golden() {
        for (bytes, disc) in [
            (rt_token::encode_transfer_checked(AMOUNT, 9), 12u8),
            (rt_token::encode_approve_checked(AMOUNT, 9), 13),
            (rt_token::encode_mint_to_checked(AMOUNT, 9), 14),
            (rt_token::encode_burn_checked(AMOUNT, 9), 15),
        ] {
            assert_eq!(bytes[0], disc);
            assert_eq!(&bytes[1..9], &AMOUNT_LE);
            assert_eq!(bytes[9], 9);
        }
    }

    #[test]
    fn spl_single_byte_instructions_golden() {
        assert_eq!(rt_token::encode_revoke(), [5]);
        assert_eq!(rt_token::encode_close_account(), [9]);
        assert_eq!(rt_token::encode_freeze_account(), [10]);
        assert_eq!(rt_token::encode_thaw_account(), [11]);
        assert_eq!(rt_token::encode_sync_native(), [17]);
        assert_eq!(rt_token::encode_initialize_account(), [1]);
    }

    #[test]
    fn spl_initialize_account_golden() {
        let a2 = rt_token::encode_initialize_account_with_owner(16, &OWNER);
        assert_eq!(a2[0], 16);
        assert_eq!(&a2[1..33], &OWNER);
        let a3 = rt_token::encode_initialize_account_with_owner(18, &OWNER);
        assert_eq!(a3[0], 18);
        assert_eq!(&a3[1..33], &OWNER);
    }

    #[test]
    fn spl_set_authority_golden() {
        let (some, some_len) = rt_token::encode_set_authority(2, Some(&OWNER));
        assert_eq!(some_len, 35);
        assert_eq!(some[0], 6);
        assert_eq!(some[1], 2);
        assert_eq!(some[2], 1);
        assert_eq!(&some[3..35], &OWNER);

        let (none, none_len) = rt_token::encode_set_authority(3, None);
        assert_eq!(none_len, 3);
        assert_eq!(none[0], 6);
        assert_eq!(none[1], 3);
        assert_eq!(none[2], 0);
    }

    #[test]
    fn system_create_account_golden() {
        let d = rt_system::encode_create_account(AMOUNT, 165, &OWNER);
        assert_eq!(&d[0..4], &[0, 0, 0, 0]);
        assert_eq!(&d[4..12], &AMOUNT_LE);
        assert_eq!(&d[12..20], &165u64.to_le_bytes());
        assert_eq!(&d[20..52], &OWNER);
    }

    #[test]
    fn system_transfer_golden() {
        let d = rt_system::encode_transfer(AMOUNT);
        assert_eq!(&d[0..4], &[2, 0, 0, 0]);
        assert_eq!(&d[4..12], &AMOUNT_LE);
    }

    #[test]
    fn system_assign_golden() {
        let d = rt_system::encode_assign(&OWNER);
        assert_eq!(&d[0..4], &[1, 0, 0, 0]);
        assert_eq!(&d[4..36], &OWNER);
    }

    #[test]
    fn system_allocate_golden() {
        let d = rt_system::encode_allocate(AMOUNT);
        assert_eq!(&d[0..4], &[8, 0, 0, 0]);
        assert_eq!(&d[4..12], &AMOUNT_LE);
    }

    #[test]
    fn system_nonce_family_golden() {
        assert_eq!(rt_system::encode_advance_nonce_account(), [4, 0, 0, 0]);
        let w = rt_system::encode_withdraw_nonce_account(AMOUNT);
        assert_eq!(&w[0..4], &[5, 0, 0, 0]);
        assert_eq!(&w[4..12], &AMOUNT_LE);
        let init = rt_system::encode_initialize_nonce_account(&OWNER);
        assert_eq!(&init[0..4], &[6, 0, 0, 0]);
        assert_eq!(&init[4..36], &OWNER);
        let auth = rt_system::encode_authorize_nonce_account(&OWNER);
        assert_eq!(&auth[0..4], &[7, 0, 0, 0]);
        assert_eq!(&auth[4..36], &OWNER);
        assert_eq!(rt_system::encode_upgrade_nonce_account(), [12, 0, 0, 0]);
    }

    /// Anchor the differential oracle: the independent reference encoders must
    /// reproduce the shipped bytes at the golden vectors. Combined with the
    /// symbolic `*_matches_reference` Kani proofs, this pins the oracle both
    /// concretely and universally.
    #[test]
    fn reference_oracle_agrees_with_shipped_at_golden_vectors() {
        use super::{spl_token, system};

        assert_eq!(
            spl_token::encode_transfer(AMOUNT),
            rt_token::encode_transfer(AMOUNT)
        );
        assert_eq!(
            spl_token::encode_approve(AMOUNT),
            rt_token::encode_approve(AMOUNT)
        );
        assert_eq!(
            spl_token::encode_mint_to(AMOUNT),
            rt_token::encode_mint_to(AMOUNT)
        );
        assert_eq!(
            spl_token::encode_burn(AMOUNT),
            rt_token::encode_burn(AMOUNT)
        );
        assert_eq!(
            spl_token::encode_transfer_checked(AMOUNT, 9),
            rt_token::encode_transfer_checked(AMOUNT, 9)
        );
        assert_eq!(
            spl_token::encode_approve_checked(AMOUNT, 9),
            rt_token::encode_approve_checked(AMOUNT, 9)
        );
        assert_eq!(
            spl_token::encode_mint_to_checked(AMOUNT, 9),
            rt_token::encode_mint_to_checked(AMOUNT, 9)
        );
        assert_eq!(
            spl_token::encode_burn_checked(AMOUNT, 9),
            rt_token::encode_burn_checked(AMOUNT, 9)
        );
        assert_eq!(spl_token::encode_revoke(), rt_token::encode_revoke());
        assert_eq!(
            spl_token::encode_close_account(),
            rt_token::encode_close_account()
        );
        assert_eq!(
            spl_token::encode_freeze_account(),
            rt_token::encode_freeze_account()
        );
        assert_eq!(
            spl_token::encode_thaw_account(),
            rt_token::encode_thaw_account()
        );
        assert_eq!(
            spl_token::encode_sync_native(),
            rt_token::encode_sync_native()
        );
        assert_eq!(
            spl_token::encode_initialize_account2(&OWNER),
            rt_token::encode_initialize_account_with_owner(16, &OWNER)
        );
        assert_eq!(
            spl_token::encode_initialize_account3(&OWNER),
            rt_token::encode_initialize_account_with_owner(18, &OWNER)
        );
        assert_eq!(
            spl_token::encode_set_authority(2, Some(&OWNER)),
            rt_token::encode_set_authority(2, Some(&OWNER))
        );
        assert_eq!(
            spl_token::encode_set_authority(3, None),
            rt_token::encode_set_authority(3, None)
        );
        assert_eq!(
            system::encode_create_account(AMOUNT, 165, &OWNER),
            rt_system::encode_create_account(AMOUNT, 165, &OWNER)
        );
        assert_eq!(
            system::encode_transfer(AMOUNT),
            rt_system::encode_transfer(AMOUNT)
        );
        assert_eq!(
            system::encode_assign(&OWNER),
            rt_system::encode_assign(&OWNER)
        );
        assert_eq!(
            system::encode_allocate(AMOUNT),
            rt_system::encode_allocate(AMOUNT)
        );
        assert_eq!(
            system::encode_advance_nonce_account(),
            rt_system::encode_advance_nonce_account()
        );
        assert_eq!(
            system::encode_withdraw_nonce_account(AMOUNT),
            rt_system::encode_withdraw_nonce_account(AMOUNT)
        );
        assert_eq!(
            system::encode_initialize_nonce_account(&OWNER),
            rt_system::encode_initialize_nonce_account(&OWNER)
        );
        assert_eq!(
            system::encode_authorize_nonce_account(&OWNER),
            rt_system::encode_authorize_nonce_account(&OWNER)
        );
        assert_eq!(
            system::encode_upgrade_nonce_account(),
            rt_system::encode_upgrade_nonce_account()
        );
    }
}

// =====================================================================
// Kani layout proofs.
//
// Two families, both driven by the `scripts/kani-spl-layouts.{sh,ps1}` lane
// (CI lane `kani-spl-layout-proofs`):
//
//   * `*_layout` — prove the SHIPPED `hopper_runtime::{token,system}::encoders`
//     directly: correct discriminator, little-endian field offsets, exact
//     total length, over fully symbolic inputs.
//   * `*_matches_reference` — differential-oracle proofs: the shipped encoder
//     is byte-equal to the independent reference encoder in this file for all
//     symbolic inputs.
//
// Conventions (per the raw_input `PID_WORD` lesson):
//   * scalars are compared as one word (`u64::from_le_bytes(...)`), never a
//     slice `==`;
//   * a 32-byte pubkey region is compared with a single symbolic index
//     `i in 0..32` and one assertion, which universally covers all 32 offsets
//     without a 32-iteration loop.
// =====================================================================
#[cfg(kani)]
mod kani_proofs {
    use super::{spl_token, system};
    use hopper_runtime::system::encoders as rt_system;
    use hopper_runtime::token::encoders as rt_token;

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

    /// Symbolic-index equality of a byte slice against a 32-byte pubkey:
    /// universal over all offsets, no loop.
    #[inline]
    fn assert_pubkey_region(d: &[u8], off: usize, owner: &[u8; 32]) {
        let i: usize = kani::any();
        kani::assume(i < 32);
        assert_eq!(d[off + i], owner[i]);
    }

    /// Symbolic-index equality of the 32-byte region at `off` between two
    /// buffers: universal over all offsets, no loop. Used by the differential
    /// proofs so shipped == reference is checked without a 32-byte `memcmp`.
    #[inline]
    fn assert_regions_eq(a: &[u8], b: &[u8], off: usize) {
        let i: usize = kani::any();
        kani::assume(i < 32);
        assert_eq!(a[off + i], b[off + i]);
    }

    // ── SPL Token: amount instructions — shipped-encoder layout ─────

    #[kani::proof]
    fn spl_transfer_layout() {
        let amount: u64 = kani::any();
        let d = rt_token::encode_transfer(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 3);
        assert_eq!(word_at(&d, 1), amount);
    }

    #[kani::proof]
    fn spl_approve_layout() {
        let amount: u64 = kani::any();
        let d = rt_token::encode_approve(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 4);
        assert_eq!(word_at(&d, 1), amount);
    }

    #[kani::proof]
    fn spl_mint_to_layout() {
        let amount: u64 = kani::any();
        let d = rt_token::encode_mint_to(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 7);
        assert_eq!(word_at(&d, 1), amount);
    }

    #[kani::proof]
    fn spl_burn_layout() {
        let amount: u64 = kani::any();
        let d = rt_token::encode_burn(amount);
        assert_eq!(d.len(), 9);
        assert_eq!(d[0], 8);
        assert_eq!(word_at(&d, 1), amount);
    }

    // ── SPL Token: checked instructions — shipped-encoder layout ────

    #[kani::proof]
    fn spl_transfer_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = rt_token::encode_transfer_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 12);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    #[kani::proof]
    fn spl_approve_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = rt_token::encode_approve_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 13);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    #[kani::proof]
    fn spl_mint_to_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = rt_token::encode_mint_to_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 14);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    #[kani::proof]
    fn spl_burn_checked_layout() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let d = rt_token::encode_burn_checked(amount, decimals);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], 15);
        assert_eq!(word_at(&d, 1), amount);
        assert_eq!(d[9], decimals);
    }

    // ── SPL Token: single-byte instructions (deterministic) ─────────

    #[kani::proof]
    fn spl_single_byte_discriminators() {
        assert_eq!(rt_token::encode_revoke(), [5]);
        assert_eq!(rt_token::encode_close_account(), [9]);
        assert_eq!(rt_token::encode_freeze_account(), [10]);
        assert_eq!(rt_token::encode_thaw_account(), [11]);
        assert_eq!(rt_token::encode_sync_native(), [17]);
        assert_eq!(rt_token::encode_initialize_account(), [1]);
    }

    // ── SPL Token: initialize-account-with-owner — shipped layout ───

    #[kani::proof]
    fn spl_initialize_account2_layout() {
        let owner: [u8; 32] = kani::any();
        let d = rt_token::encode_initialize_account_with_owner(16, &owner);
        assert_eq!(d.len(), 33);
        assert_eq!(d[0], 16);
        assert_pubkey_region(&d, 1, &owner);
    }

    #[kani::proof]
    fn spl_initialize_account3_layout() {
        let owner: [u8; 32] = kani::any();
        let d = rt_token::encode_initialize_account_with_owner(18, &owner);
        assert_eq!(d.len(), 33);
        assert_eq!(d[0], 18);
        assert_pubkey_region(&d, 1, &owner);
    }

    // ── SPL Token: SetAuthority (both COption branches) — shipped ───

    #[kani::proof]
    fn spl_set_authority_some_layout() {
        let authority_type: u8 = kani::any();
        let owner: [u8; 32] = kani::any();
        let (d, len) = rt_token::encode_set_authority(authority_type, Some(&owner));
        assert_eq!(len, 35);
        assert_eq!(d[0], 6);
        assert_eq!(d[1], authority_type);
        assert_eq!(d[2], 1);
        assert_pubkey_region(&d, 3, &owner);
    }

    #[kani::proof]
    fn spl_set_authority_none_layout() {
        let authority_type: u8 = kani::any();
        let (d, len) = rt_token::encode_set_authority(authority_type, None);
        assert_eq!(len, 3);
        assert_eq!(d[0], 6);
        assert_eq!(d[1], authority_type);
        assert_eq!(d[2], 0);
    }

    // ── System program — shipped-encoder layout ─────────────────────

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
        let d = rt_system::encode_transfer(lamports);
        assert_eq!(d.len(), 12);
        assert_system_disc(&d, 2);
        assert_eq!(word_at(&d, 4), lamports);
    }

    #[kani::proof]
    fn system_allocate_layout() {
        let space: u64 = kani::any();
        let d = rt_system::encode_allocate(space);
        assert_eq!(d.len(), 12);
        assert_system_disc(&d, 8);
        assert_eq!(word_at(&d, 4), space);
    }

    #[kani::proof]
    fn system_assign_layout() {
        let owner: [u8; 32] = kani::any();
        let d = rt_system::encode_assign(&owner);
        assert_eq!(d.len(), 36);
        assert_system_disc(&d, 1);
        assert_pubkey_region(&d, 4, &owner);
    }

    #[kani::proof]
    fn system_create_account_layout() {
        let lamports: u64 = kani::any();
        let space: u64 = kani::any();
        let owner: [u8; 32] = kani::any();
        let d = rt_system::encode_create_account(lamports, space, &owner);
        assert_eq!(d.len(), 52);
        assert_system_disc(&d, 0);
        assert_eq!(word_at(&d, 4), lamports);
        assert_eq!(word_at(&d, 12), space);
        assert_pubkey_region(&d, 20, &owner);
    }

    // ── System durable-nonce family (fixed-size) — shipped layout ───

    #[kani::proof]
    fn system_advance_nonce_account_layout() {
        let d = rt_system::encode_advance_nonce_account();
        assert_eq!(d.len(), 4);
        assert_system_disc(&d, 4);
    }

    #[kani::proof]
    fn system_withdraw_nonce_account_layout() {
        let lamports: u64 = kani::any();
        let d = rt_system::encode_withdraw_nonce_account(lamports);
        assert_eq!(d.len(), 12);
        assert_system_disc(&d, 5);
        assert_eq!(word_at(&d, 4), lamports);
    }

    #[kani::proof]
    fn system_initialize_nonce_account_layout() {
        let authority: [u8; 32] = kani::any();
        let d = rt_system::encode_initialize_nonce_account(&authority);
        assert_eq!(d.len(), 36);
        assert_system_disc(&d, 6);
        assert_pubkey_region(&d, 4, &authority);
    }

    #[kani::proof]
    fn system_authorize_nonce_account_layout() {
        let new_authority: [u8; 32] = kani::any();
        let d = rt_system::encode_authorize_nonce_account(&new_authority);
        assert_eq!(d.len(), 36);
        assert_system_disc(&d, 7);
        assert_pubkey_region(&d, 4, &new_authority);
    }

    #[kani::proof]
    fn system_upgrade_nonce_account_layout() {
        let d = rt_system::encode_upgrade_nonce_account();
        assert_eq!(d.len(), 4);
        assert_system_disc(&d, 12);
    }

    // ── Differential oracle: shipped encoder == reference encoder ───
    //
    // Each proof compares the shipped `rt_*` encoder against the independent
    // reference in `super::{spl_token, system}`, field-wise / word-wise /
    // symbolic-index, so equality is proven for all symbolic inputs without a
    // large `memcmp` loop.

    #[kani::proof]
    fn spl_transfer_matches_reference() {
        let amount: u64 = kani::any();
        let s = rt_token::encode_transfer(amount);
        let r = spl_token::encode_transfer(amount);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
    }

    #[kani::proof]
    fn spl_approve_matches_reference() {
        let amount: u64 = kani::any();
        let s = rt_token::encode_approve(amount);
        let r = spl_token::encode_approve(amount);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
    }

    #[kani::proof]
    fn spl_mint_to_matches_reference() {
        let amount: u64 = kani::any();
        let s = rt_token::encode_mint_to(amount);
        let r = spl_token::encode_mint_to(amount);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
    }

    #[kani::proof]
    fn spl_burn_matches_reference() {
        let amount: u64 = kani::any();
        let s = rt_token::encode_burn(amount);
        let r = spl_token::encode_burn(amount);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
    }

    #[kani::proof]
    fn spl_transfer_checked_matches_reference() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let s = rt_token::encode_transfer_checked(amount, decimals);
        let r = spl_token::encode_transfer_checked(amount, decimals);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
        assert_eq!(s[9], r[9]);
    }

    #[kani::proof]
    fn spl_approve_checked_matches_reference() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let s = rt_token::encode_approve_checked(amount, decimals);
        let r = spl_token::encode_approve_checked(amount, decimals);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
        assert_eq!(s[9], r[9]);
    }

    #[kani::proof]
    fn spl_mint_to_checked_matches_reference() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let s = rt_token::encode_mint_to_checked(amount, decimals);
        let r = spl_token::encode_mint_to_checked(amount, decimals);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
        assert_eq!(s[9], r[9]);
    }

    #[kani::proof]
    fn spl_burn_checked_matches_reference() {
        let amount: u64 = kani::any();
        let decimals: u8 = kani::any();
        let s = rt_token::encode_burn_checked(amount, decimals);
        let r = spl_token::encode_burn_checked(amount, decimals);
        assert_eq!(s[0], r[0]);
        assert_eq!(word_at(&s, 1), word_at(&r, 1));
        assert_eq!(s[9], r[9]);
    }

    #[kani::proof]
    fn spl_single_byte_matches_reference() {
        assert_eq!(rt_token::encode_revoke(), spl_token::encode_revoke());
        assert_eq!(
            rt_token::encode_close_account(),
            spl_token::encode_close_account()
        );
        assert_eq!(
            rt_token::encode_freeze_account(),
            spl_token::encode_freeze_account()
        );
        assert_eq!(
            rt_token::encode_thaw_account(),
            spl_token::encode_thaw_account()
        );
        assert_eq!(
            rt_token::encode_sync_native(),
            spl_token::encode_sync_native()
        );
    }

    #[kani::proof]
    fn spl_initialize_account2_matches_reference() {
        let owner: [u8; 32] = kani::any();
        let s = rt_token::encode_initialize_account_with_owner(16, &owner);
        let r = spl_token::encode_initialize_account2(&owner);
        assert_eq!(s[0], r[0]);
        assert_regions_eq(&s, &r, 1);
    }

    #[kani::proof]
    fn spl_initialize_account3_matches_reference() {
        let owner: [u8; 32] = kani::any();
        let s = rt_token::encode_initialize_account_with_owner(18, &owner);
        let r = spl_token::encode_initialize_account3(&owner);
        assert_eq!(s[0], r[0]);
        assert_regions_eq(&s, &r, 1);
    }

    #[kani::proof]
    fn spl_set_authority_some_matches_reference() {
        let authority_type: u8 = kani::any();
        let owner: [u8; 32] = kani::any();
        let (s, s_len) = rt_token::encode_set_authority(authority_type, Some(&owner));
        let (r, r_len) = spl_token::encode_set_authority(authority_type, Some(&owner));
        assert_eq!(s_len, r_len);
        assert_eq!(s[0], r[0]);
        assert_eq!(s[1], r[1]);
        assert_eq!(s[2], r[2]);
        assert_regions_eq(&s, &r, 3);
    }

    #[kani::proof]
    fn spl_set_authority_none_matches_reference() {
        let authority_type: u8 = kani::any();
        let (s, s_len) = rt_token::encode_set_authority(authority_type, None);
        let (r, r_len) = spl_token::encode_set_authority(authority_type, None);
        assert_eq!(s_len, r_len);
        assert_eq!(s[0], r[0]);
        assert_eq!(s[1], r[1]);
        assert_eq!(s[2], r[2]);
    }

    #[kani::proof]
    fn system_transfer_matches_reference() {
        let lamports: u64 = kani::any();
        let s = rt_system::encode_transfer(lamports);
        let r = system::encode_transfer(lamports);
        assert_eq!(word_at(&s, 0), word_at(&r, 0));
        assert_eq!(word_at(&s, 4), word_at(&r, 4));
    }

    #[kani::proof]
    fn system_allocate_matches_reference() {
        let space: u64 = kani::any();
        let s = rt_system::encode_allocate(space);
        let r = system::encode_allocate(space);
        assert_eq!(word_at(&s, 0), word_at(&r, 0));
        assert_eq!(word_at(&s, 4), word_at(&r, 4));
    }

    #[kani::proof]
    fn system_assign_matches_reference() {
        let owner: [u8; 32] = kani::any();
        let s = rt_system::encode_assign(&owner);
        let r = system::encode_assign(&owner);
        assert_eq!(word_at(&s, 0), word_at(&r, 0));
        assert_regions_eq(&s, &r, 4);
    }

    #[kani::proof]
    fn system_create_account_matches_reference() {
        let lamports: u64 = kani::any();
        let space: u64 = kani::any();
        let owner: [u8; 32] = kani::any();
        let s = rt_system::encode_create_account(lamports, space, &owner);
        let r = system::encode_create_account(lamports, space, &owner);
        assert_eq!(word_at(&s, 0), word_at(&r, 0));
        assert_eq!(word_at(&s, 4), word_at(&r, 4));
        assert_eq!(word_at(&s, 12), word_at(&r, 12));
        assert_regions_eq(&s, &r, 20);
    }

    // ── Differential oracle: durable-nonce family ───────────────────

    #[kani::proof]
    fn system_advance_nonce_account_matches_reference() {
        // 4-byte deterministic buffers: direct array equality is 4 elements.
        assert_eq!(
            rt_system::encode_advance_nonce_account(),
            system::encode_advance_nonce_account()
        );
    }

    #[kani::proof]
    fn system_withdraw_nonce_account_matches_reference() {
        let lamports: u64 = kani::any();
        let s = rt_system::encode_withdraw_nonce_account(lamports);
        let r = system::encode_withdraw_nonce_account(lamports);
        assert_eq!(word_at(&s, 0), word_at(&r, 0));
        assert_eq!(word_at(&s, 4), word_at(&r, 4));
    }

    #[kani::proof]
    fn system_initialize_nonce_account_matches_reference() {
        let authority: [u8; 32] = kani::any();
        let s = rt_system::encode_initialize_nonce_account(&authority);
        let r = system::encode_initialize_nonce_account(&authority);
        assert_eq!(word_at(&s, 0), word_at(&r, 0));
        assert_regions_eq(&s, &r, 4);
    }

    #[kani::proof]
    fn system_authorize_nonce_account_matches_reference() {
        let new_authority: [u8; 32] = kani::any();
        let s = rt_system::encode_authorize_nonce_account(&new_authority);
        let r = system::encode_authorize_nonce_account(&new_authority);
        assert_eq!(word_at(&s, 0), word_at(&r, 0));
        assert_regions_eq(&s, &r, 4);
    }

    #[kani::proof]
    fn system_upgrade_nonce_account_matches_reference() {
        assert_eq!(
            rt_system::encode_upgrade_nonce_account(),
            system::encode_upgrade_nonce_account()
        );
    }
}
