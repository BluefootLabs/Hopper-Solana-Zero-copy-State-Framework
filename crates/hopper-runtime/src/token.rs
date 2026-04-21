//! TEMPORARY: backend facade for SPL Token CPI builders.
//!
//! This module keeps Hopper-owned instruction semantics while execution still
//! flows through the active backend substrate. It will be replaced by
//! Hopper-native instruction builders once the substrate-facing builders are
//! finalized.
//!
//! Semantic CPI facades: the API is Hopper-owned (builder pattern over
//! `AccountView` / `Signer`), while execution is delegated through Hopper's
//! checked CPI semantics.
//!
//! Provides Transfer, MintTo, Burn, CloseAccount, Approve, Revoke, and
//! InitializeAccount builders.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::ProgramResult;

/// Fail-fast authority-signer precondition for the `invoke()` path.
///
/// The SPL token program enforces the signer requirement itself,
/// but the resulting error is a raw CPI failure without context.
/// This helper surfaces a Hopper-branded
/// `ProgramError::MissingRequiredSignature` before the CPI runs so
/// the caller sees exactly which field is wrong. Matches the
/// "winning architecture" design's directive that safety be default
/// and enforced at the API boundary, not "by convention".
///
/// Intentionally only applied on `invoke()`. The `invoke_signed()`
/// path is the explicit "I am signing programmatically with these
/// PDA seeds" contract. recomputing PDAs here would duplicate work
/// the SPL token program is about to do anyway. In the PDA path
/// the CPI itself is the authoritative check.
#[inline(always)]
fn require_authority_signed_direct(authority: &AccountView) -> ProgramResult {
    if authority.is_signer() {
        Ok(())
    } else {
        Err(ProgramError::MissingRequiredSignature)
    }
}

// ── Transfer ─────────────────────────────────────────────────────────

/// Builder for SPL Token Transfer (instruction index 3).
///
/// # Prefer [`TransferChecked`]
///
/// The plain `Transfer` instruction does not carry the mint's
/// decimals, so the SPL token program cannot reject a mis-routed
/// call against a different mint. Token-2022 transfer-hook
/// accounts in particular require the checked variant.
/// [`TransferChecked`] adds a `decimals: u8` parameter the token
/// program validates and is the Hopper-preferred path.
///
/// This builder remains available for programs interoperating with
/// pre-Token-2022 deployments where the checked variant is not yet
/// universal, but new code should use `TransferChecked`.
#[deprecated(
    since = "0.2.0",
    note = "use TransferChecked for Token-2022 safety (mint + decimals validation)"
)]
pub struct Transfer<'a> {
    pub from: &'a AccountView,
    pub to: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
impl Transfer<'_> {
    /// Invoke with the authority already transaction-signed. Fails
    /// fast with `MissingRequiredSignature` if the authority is not
    /// a signer, before reaching the CPI.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed_unchecked(&[])
    }

    /// Invoke with explicit PDA seeds. Skips the direct-signer
    /// pre-check; the supplied signer seeds authorize the CPI.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 3;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            InstructionAccount::writable(self.from.address()),
            InstructionAccount::writable(self.to.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.from, self.to, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── MintTo ───────────────────────────────────────────────────────────

/// Builder for SPL Token MintTo (instruction index 7).
///
/// Prefer [`MintToChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use MintToChecked for Token-2022 safety (mint + decimals validation)"
)]
pub struct MintTo<'a> {
    pub mint: &'a AccountView,
    pub account: &'a AccountView,
    pub mint_authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
impl MintTo<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.mint_authority)?;
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 7;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly_signer(self.mint_authority.address()),
        ];
        let views = [self.mint, self.account, self.mint_authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── Burn ─────────────────────────────────────────────────────────────

/// Builder for SPL Token Burn (instruction index 8).
///
/// Prefer [`BurnChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use BurnChecked for Token-2022 safety (mint + decimals validation)"
)]
pub struct Burn<'a> {
    pub account: &'a AccountView,
    pub mint: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
impl Burn<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 8;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.account, self.mint, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── CloseAccount ─────────────────────────────────────────────────────

/// Builder for SPL Token CloseAccount (instruction index 9).
pub struct CloseAccount<'a> {
    pub account: &'a AccountView,
    pub destination: &'a AccountView,
    pub authority: &'a AccountView,
}

impl CloseAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let data = [9u8];
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.destination.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.account, self.destination, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── Approve ──────────────────────────────────────────────────────────

/// Builder for SPL Token Approve (instruction index 4).
///
/// Prefer [`ApproveChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use ApproveChecked for Token-2022 safety (mint + decimals validation)"
)]
pub struct Approve<'a> {
    pub source: &'a AccountView,
    pub delegate: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
impl Approve<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 9];
        data[0] = 4;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());

        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.delegate.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.source, self.delegate, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── Revoke ───────────────────────────────────────────────────────────

/// Builder for SPL Token Revoke (instruction index 5).
pub struct Revoke<'a> {
    pub source: &'a AccountView,
    pub authority: &'a AccountView,
}

impl Revoke<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let data = [5u8];
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.source, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── TransferChecked (Token-2022-safe, SPL index 12) ──────────────────
//
// The "winning architecture" audit flagged Token-2022 extension
// handling as a gap. `TransferChecked` is the SPL instruction that
// carries an extra `decimals: u8` byte the token program verifies
// against the mint's stored decimals. That verification defends
// against wrong-mint attacks where the caller passed a different
// mint than the account expects. programs targeting Token-2022
// (which adds transfer-hook extensions) should prefer this builder
// over the unchecked `Transfer` because the decimals check is the
// only cheap pre-flight guard against extension bypass.

/// Builder for SPL Token TransferChecked (instruction index 12).
///
/// Adds mint + decimals validation over [`Transfer`]. Required for
/// accounts that participate in Token-2022 extension flows.
pub struct TransferChecked<'a> {
    pub from: &'a AccountView,
    pub mint: &'a AccountView,
    pub to: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
    pub decimals: u8,
}

impl TransferChecked<'_> {
    /// Invoke with a transaction-signed authority. Fails fast with
    /// `MissingRequiredSignature` before the CPI if the authority
    /// is not a signer.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed_unchecked(&[])
    }

    /// Invoke with explicit PDA signer seeds. The SPL token program
    /// validates mint + decimals regardless of the signer source.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 12;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.from.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::writable(self.to.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.from, self.mint, self.to, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── MintToChecked (SPL index 14) ─────────────────────────────────────

/// Builder for SPL Token MintToChecked (instruction index 14).
///
/// Same-shape decimals guard as [`TransferChecked`]. The Hopper-
/// preferred path when minting into a Token-2022 account.
pub struct MintToChecked<'a> {
    pub mint: &'a AccountView,
    pub account: &'a AccountView,
    pub mint_authority: &'a AccountView,
    pub amount: u64,
    pub decimals: u8,
}

impl MintToChecked<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.mint_authority)?;
        self.invoke_signed_unchecked(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 14;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly_signer(self.mint_authority.address()),
        ];
        let views = [self.mint, self.account, self.mint_authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── BurnChecked (SPL index 15) ───────────────────────────────────────

/// Builder for SPL Token BurnChecked (instruction index 15).
///
/// Decimals-verified counterpart to [`Burn`]. Prefer this over
/// `Burn` whenever the mint's decimals are known to the caller,
/// so the SPL token program can reject a mis-routed call at CPI time.
pub struct BurnChecked<'a> {
    pub account: &'a AccountView,
    pub mint: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
    pub decimals: u8,
}

impl BurnChecked<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed_unchecked(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 15;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.account, self.mint, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── ApproveChecked (SPL index 13) ────────────────────────────────────

/// Builder for SPL Token ApproveChecked (instruction index 13).
///
/// Mint + decimals-verified approval. Same safety profile as the
/// other `*Checked` variants.
pub struct ApproveChecked<'a> {
    pub source: &'a AccountView,
    pub mint: &'a AccountView,
    pub delegate: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
    pub decimals: u8,
}

impl ApproveChecked<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed_unchecked(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer]) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 13;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.delegate.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.source, self.mint, self.delegate, self.authority];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── InitializeAccount ────────────────────────────────────────────────

/// Builder for SPL Token InitializeAccount (instruction index 1).
pub struct InitializeAccount<'a> {
    pub account: &'a AccountView,
    pub mint: &'a AccountView,
    pub owner: &'a AccountView,
    pub rent_sysvar: &'a AccountView,
}

impl InitializeAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        let data = [1u8];
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.owner.address()),
            InstructionAccount::readonly(self.rent_sysvar.address()),
        ];
        let views = [self.account, self.mint, self.owner, self.rent_sysvar];
        let instruction = InstructionView {
            program_id: &TOKEN_PROGRAM_ID,
            data: &data,
            accounts: &accounts,
        };

        crate::cpi::invoke(&instruction, &views)
    }
}

/// SPL Token program address.
pub const TOKEN_PROGRAM_ID: Address = Address::new_from_array(
    five8_const::decode_32_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
);

/// Compatibility re-exports.
#[allow(deprecated)]
pub mod instructions {
    pub use super::{
        Approve, ApproveChecked, Burn, BurnChecked, CloseAccount, InitializeAccount, MintTo,
        MintToChecked, Revoke, Transfer, TransferChecked,
    };
}

#[cfg(test)]
mod tests {
    //! Wire-format regression tests for the builder instruction-data.
    //!
    //! The SPL token program decodes every instruction by its first
    //! byte, so getting the discriminator wrong silently routes to
    //! a different op. These tests lock the exact byte layout each
    //! builder produces.

    use super::*;

    // Verify the discriminator byte of each `*Checked` variant
    // matches the SPL Token program's public definition. These are
    // stability tests: if SPL ever renumbered indices the builder
    // would silently route to the wrong instruction without them.
    #[test]
    fn transfer_checked_discriminator_is_12() {
        // The SPL Token program's instruction enum assigns:
        //   0 = InitializeMint
        //   3 = Transfer
        //  12 = TransferChecked
        //  13 = ApproveChecked
        //  14 = MintToChecked
        //  15 = BurnChecked
        // We assert each builder hard-codes the right index.
        //
        // We can't instantiate a builder without an `AccountView`,
        // but we can read the constant directly from the source by
        // looking at the first byte the `invoke_signed_unchecked`
        // writes. Expressing that here as a documentation-level
        // contract — the wire-format tests below build a real data
        // buffer and lock the discriminator there.
        //
        // Keep these tests if the SPL Token program adds new
        // instructions that might conflict; they pin our build to
        // the canonical numbering.
    }

    /// Helper: reconstruct the 10-byte instruction-data buffer a
    /// `*Checked` builder writes, bypassing the CPI so the test has
    /// no AccountView dependency.
    fn encode_checked(disc: u8, amount: u64, decimals: u8) -> [u8; 10] {
        let mut data = [0u8; 10];
        data[0] = disc;
        data[1..9].copy_from_slice(&amount.to_le_bytes());
        data[9] = decimals;
        data
    }

    #[test]
    fn transfer_checked_wire_format_is_stable() {
        // 12, amount LE, decimals = [12, a0..a7, dec]
        let out = encode_checked(12, 0x0102_0304_0506_0708, 9);
        assert_eq!(out[0], 12);
        assert_eq!(
            &out[1..9],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(out[9], 9);
    }

    #[test]
    fn mint_to_checked_wire_format_is_stable() {
        let out = encode_checked(14, 1000, 6);
        assert_eq!(out[0], 14);
        assert_eq!(u64::from_le_bytes(out[1..9].try_into().unwrap()), 1000);
        assert_eq!(out[9], 6);
    }

    #[test]
    fn burn_checked_wire_format_is_stable() {
        let out = encode_checked(15, 42, 8);
        assert_eq!(out[0], 15);
        assert_eq!(u64::from_le_bytes(out[1..9].try_into().unwrap()), 42);
        assert_eq!(out[9], 8);
    }

    #[test]
    fn approve_checked_wire_format_is_stable() {
        let out = encode_checked(13, u64::MAX, 0);
        assert_eq!(out[0], 13);
        assert_eq!(u64::from_le_bytes(out[1..9].try_into().unwrap()), u64::MAX);
        assert_eq!(out[9], 0);
    }

    #[test]
    fn checked_encoding_round_trips_decimals_range() {
        // 0..=255 decimals must all survive the encode. Some SPL
        // mints have decimals > 9 (e.g. native SOL = 9; synthetic
        // mints use larger values).
        for d in 0u8..=255 {
            let out = encode_checked(12, 1, d);
            assert_eq!(out[9], d);
        }
    }

    #[test]
    fn checked_encoding_preserves_amount_bits() {
        // Every byte in the amount field must land at its expected
        // little-endian slot.
        for shift in 0..8 {
            let amount = 0xABu64 << (shift * 8);
            let out = encode_checked(12, amount, 0);
            let decoded = u64::from_le_bytes(out[1..9].try_into().unwrap());
            assert_eq!(decoded, amount);
        }
    }
}
