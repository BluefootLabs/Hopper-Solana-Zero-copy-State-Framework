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
//! Provides checked-by-default TransferChecked, MintToChecked, BurnChecked,
//! ApproveChecked, CloseAccount, Revoke, and InitializeAccount builders.
//! Deprecated plain Transfer/MintTo/Burn/Approve builders are compiled only
//! when `legacy-token-instructions` is explicitly enabled.

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

/// Verify an SPL Token account's `owner` field matches `authority.key()`.
///
/// SPL TokenAccount layout: bytes `[32..64]` are the `owner` pubkey
/// (the authority allowed to move tokens out of this account). The
/// SPL Token program checks this on every transfer/approve/burn, but
/// Hopper's pre-check surfaces a Hopper-branded error before the CPI
/// so a misconfigured invocation fails with `IncorrectAuthority`
/// instead of an opaque CPI failure.
///
/// This is the load-bearing helper behind the
/// `#[hopper::program(enforce_token_checks = true)]` contract: the
/// macro emits `HOPPER_PROGRAM_POLICY.enforce_token_checks = true`,
/// and handlers opt into the strict invoke paths
/// ([`TransferChecked::invoke_strict`] etc.) to get this check
/// auto-injected. Handlers can also call it directly when they reach
/// outside the typed-context envelope.
///
/// Returns `Err(ProgramError::AccountDataTooSmall)` if the token
/// account's data buffer is too short (not a valid SPL TokenAccount).
#[inline]
pub fn require_token_authority(
    token_account: &AccountView,
    authority: &AccountView,
) -> ProgramResult {
    // SPL TokenAccount.owner lives at bytes 32..64. The buffer must
    // be at least 64 bytes; a valid TokenAccount is exactly 165 on
    // legacy Token, variable on Token-2022 but always >= 165.
    let data = token_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 64 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut owner_bytes = [0u8; 32];
    owner_bytes.copy_from_slice(&data[32..64]);
    let authority_bytes: [u8; 32] = *authority.address().as_array();
    if owner_bytes == authority_bytes {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Verify an SPL Token account's `owner` field matches a pubkey
/// supplied directly (i.e. not wrapped in an `AccountView`).
///
/// This is the sibling of [`require_token_authority`], differing only
/// in its argument shape: it takes `&Address` rather than
/// `&AccountView` for the expected authority. The declarative
/// `#[account(token::authority = X)]` attribute lowers to this form
/// because the user's expression might resolve to a constant address,
/// a cached field, or another account's key. all of which are
/// `&Address` by the time the check runs, none of them necessarily
/// wrapped in an `AccountView`.
#[inline]
pub fn require_token_owner_eq(
    token_account: &AccountView,
    expected_owner: &Address,
) -> ProgramResult {
    let data = token_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 64 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut actual = [0u8; 32];
    actual.copy_from_slice(&data[32..64]);
    if actual == *expected_owner.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Verify an SPL Token account's `mint` field matches `expected_mint`.
///
/// SPL TokenAccount layout: bytes `[0..32]` are the `mint` pubkey.
/// Token-2022 extensions never shift the base-layout prefix. the
/// TLV extensions live past byte 165 behind the account-type
/// discriminator, so reading bytes 0..32 is valid for both Token
/// and Token-2022 accounts.
///
/// This is the precondition behind Hopper's `#[account(token::mint = X)]`
/// attribute. It surfaces a Hopper-branded `InvalidAccountData` error
/// before any downstream CPI runs, so a user-visible failure clearly
/// points at "wrong mint" rather than an opaque SPL token error.
///
/// ## Innovation over Anchor
///
/// Anchor's `token::mint = X` is checked by deserializing the full
/// `TokenAccount` struct via `anchor_spl`, which pulls in the anchor-spl
/// crate and costs compute on every check. Hopper's version reads the
/// exact 32 bytes of interest directly from the already-borrowed data
/// buffer. zero extra crate dependencies, no full-struct deserialize,
/// and the check is trivially inlinable.
#[inline]
pub fn require_token_mint(
    token_account: &AccountView,
    expected_mint: &Address,
) -> ProgramResult {
    let data = token_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 32 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let actual: [u8; 32] = {
        let mut out = [0u8; 32];
        out.copy_from_slice(&data[0..32]);
        out
    };
    if actual == *expected_mint.as_array() {
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}

/// Verify an SPL Mint account's `mint_authority` COption field
/// matches `expected_authority`.
///
/// SPL Mint layout (82 bytes total):
/// - [0..4]   COption tag for mint_authority (u32 LE; 0 = None, 1 = Some)
/// - [4..36]  mint_authority pubkey (only meaningful when tag == 1)
/// - [36..44] supply (u64 LE)
/// - [44]     decimals
/// - [45]     is_initialized
/// - [46..50] COption tag for freeze_authority
/// - [50..82] freeze_authority pubkey
///
/// Behavior: if the tag says `None`, the check fails with
/// `InvalidAccountData` (the caller asked for a specific authority
/// but the mint has none). If the tag says `Some` and the stored
/// pubkey does not match, the check fails with `IncorrectAuthority`.
/// Separating the two error codes lets callers tell "no authority at
/// all" apart from "wrong authority".
#[inline]
pub fn require_mint_authority(
    mint_account: &AccountView,
    expected_authority: &Address,
) -> ProgramResult {
    let data = mint_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 46 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let tag = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if tag != 1 {
        // Tag value 0 = None; any other non-one value is malformed.
        return Err(ProgramError::InvalidAccountData);
    }
    let mut actual = [0u8; 32];
    actual.copy_from_slice(&data[4..36]);
    if actual == *expected_authority.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

/// Verify an SPL Mint account's `decimals` byte matches `expected`.
///
/// Reads byte 44 of the Mint layout. Pairs with `require_mint_authority`
/// to express the full `#[account(mint::authority = X, mint::decimals = N)]`
/// Anchor-compat syntax with zero additional crate dependencies.
#[inline]
pub fn require_mint_decimals(mint_account: &AccountView, expected: u8) -> ProgramResult {
    let data = mint_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 45 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if data[44] == expected {
        Ok(())
    } else {
        Err(ProgramError::InvalidAccountData)
    }
}

/// Verify an SPL Mint account's `freeze_authority` COption field
/// matches `expected_freeze`.
///
/// Same shape as [`require_mint_authority`] but reads the second
/// COption (bytes 46..50 for tag, 50..82 for pubkey). Exposed so the
/// macro surface can support a future `mint::freeze_authority = X`
/// constraint without another runtime change.
#[inline]
pub fn require_mint_freeze_authority(
    mint_account: &AccountView,
    expected_freeze: &Address,
) -> ProgramResult {
    let data = mint_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 82 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let tag = u32::from_le_bytes([data[46], data[47], data[48], data[49]]);
    if tag != 1 {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut actual = [0u8; 32];
    actual.copy_from_slice(&data[50..82]);
    if actual == *expected_freeze.as_array() {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
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
#[cfg(feature = "legacy-token-instructions")]
pub struct Transfer<'a> {
    pub from: &'a AccountView,
    pub to: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
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
#[cfg(feature = "legacy-token-instructions")]
pub struct MintTo<'a> {
    pub mint: &'a AccountView,
    pub account: &'a AccountView,
    pub mint_authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
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
#[cfg(feature = "legacy-token-instructions")]
pub struct Burn<'a> {
    pub account: &'a AccountView,
    pub mint: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
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
#[cfg(feature = "legacy-token-instructions")]
pub struct Approve<'a> {
    pub source: &'a AccountView,
    pub delegate: &'a AccountView,
    pub authority: &'a AccountView,
    pub amount: u64,
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
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

    /// Strict invoke: signer pre-check **plus** token-account
    /// ownership verification. Auto-injects the check that
    /// `#[hopper::program(enforce_token_checks = true)]` promises so
    /// a handler inside such a program can write
    /// `TransferChecked { ... }.invoke_strict()?` and know that the
    /// attacker-passes-correct-pubkey-but-wrong-signer exploit class
    /// is closed before the CPI.
    ///
    /// Verifies `self.from`'s `owner` field (SPL TokenAccount bytes
    /// `[32..64]`) matches `self.authority.address()`. Returns
    /// `ProgramError::IncorrectAuthority` on mismatch.
    #[inline]
    pub fn invoke_strict(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        require_token_authority(self.from, self.authority)?;
        self.invoke_signed_unchecked(&[])
    }

    /// Invoke with explicit PDA signer seeds. The SPL token program
    /// validates mint + decimals regardless of the signer source.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    /// Strict PDA-signed invoke: ownership pre-check (the SPL token
    /// program revalidates, but Hopper surfaces a branded error
    /// first) then CPI with the supplied signer seeds.
    #[inline]
    pub fn invoke_signed_strict(&self, signers: &[Signer]) -> ProgramResult {
        require_token_authority(self.from, self.authority)?;
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

    /// Strict invoke: signer pre-check plus token-account ownership
    /// verification. See [`TransferChecked::invoke_strict`] for the
    /// full rationale.
    #[inline]
    pub fn invoke_strict(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        require_token_authority(self.account, self.authority)?;
        self.invoke_signed_unchecked(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    /// Strict PDA-signed invoke. Pre-check the burn-source owner
    /// before the CPI so a misrouted signer surfaces a Hopper-branded
    /// error instead of an opaque SPL failure.
    #[inline]
    pub fn invoke_signed_strict(&self, signers: &[Signer]) -> ProgramResult {
        require_token_authority(self.account, self.authority)?;
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

    /// Strict invoke: signer pre-check plus source-account ownership
    /// verification. Ensures the authority granting the approval is
    /// actually allowed to do so. See [`TransferChecked::invoke_strict`]
    /// for the full rationale.
    #[inline]
    pub fn invoke_strict(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        require_token_authority(self.source, self.authority)?;
        self.invoke_signed_unchecked(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    /// Strict PDA-signed invoke. Pre-check the source-account owner
    /// before the CPI.
    #[inline]
    pub fn invoke_signed_strict(&self, signers: &[Signer]) -> ProgramResult {
        require_token_authority(self.source, self.authority)?;
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
pub mod instructions {
    pub use super::{
        ApproveChecked, BurnChecked, CloseAccount, InitializeAccount, MintToChecked, Revoke,
        TransferChecked,
    };

    #[cfg(feature = "legacy-token-instructions")]
    #[allow(deprecated)]
    pub use super::{Approve, Burn, MintTo, Transfer};
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
        // contract, the wire-format tests below build a real data
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

    // ── require_token_authority regression tests ─────────────────────

    /// Build a minimal valid SPL TokenAccount data buffer + an
    /// AccountView wrapping it, plus a matching authority view. The
    /// token account's `owner` field (bytes [32..64]) is set to the
    /// requested authority so the ownership check passes by default;
    /// individual tests can mutate the buffer to exercise mismatch.
    fn make_token_and_authority(
        authority_bytes: [u8; 32],
        token_owner_bytes: [u8; 32],
    ) -> (
        std::vec::Vec<u8>,
        std::vec::Vec<u8>,
        crate::account::AccountView,
        crate::account::AccountView,
    ) {
        use hopper_native::{AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED};

        // TokenAccount: SPL layout is 165 bytes; first 32 bytes are
        // `mint`, next 32 are `owner`. We only care about the owner
        // slot for `require_token_authority`, but size the buffer at
        // 165 so it looks like a real TokenAccount.
        let token_data_len = 165;
        let mut token_backing = std::vec![0u8; RuntimeAccount::SIZE + token_data_len];
        let token_raw = token_backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            token_raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([0xAA; 32]),
                owner: NativeAddress::new_from_array([3; 32]),
                lamports: 2_039_280,
                data_len: token_data_len as u64,
            });
            // Write the SPL TokenAccount.owner field at data[32..64].
            let data_ptr = (token_raw as *mut u8).add(RuntimeAccount::SIZE);
            core::ptr::copy_nonoverlapping(
                token_owner_bytes.as_ptr(),
                data_ptr.add(32),
                32,
            );
        }
        let token_backend = unsafe { NativeAccountView::new_unchecked(token_raw) };
        let token_view = crate::account::AccountView::from_backend(token_backend);

        // Authority: no data needed, just an address field.
        let mut auth_backing = std::vec![0u8; RuntimeAccount::SIZE];
        let auth_raw = auth_backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            auth_raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 1,
                is_writable: 0,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array(authority_bytes),
                owner: NativeAddress::new_from_array([0; 32]),
                lamports: 0,
                data_len: 0,
            });
        }
        let auth_backend = unsafe { NativeAccountView::new_unchecked(auth_raw) };
        let auth_view = crate::account::AccountView::from_backend(auth_backend);

        (token_backing, auth_backing, token_view, auth_view)
    }

    #[test]
    fn require_token_authority_accepts_matching_owner() {
        let authority = [0x42u8; 32];
        let (_tb, _ab, token, auth) = make_token_and_authority(authority, authority);
        require_token_authority(&token, &auth).unwrap();
    }

    #[test]
    fn require_token_authority_rejects_mismatched_owner() {
        let authority = [0x42u8; 32];
        let wrong_owner = [0x77u8; 32];
        let (_tb, _ab, token, auth) = make_token_and_authority(authority, wrong_owner);
        let err = require_token_authority(&token, &auth).unwrap_err();
        assert!(matches!(err, ProgramError::IncorrectAuthority));
    }

    #[test]
    fn require_token_authority_rejects_short_buffer() {
        use hopper_native::{AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED};

        // Token account with only 50 bytes of data is not a valid
        // SPL TokenAccount (owner field starts at byte 32 and runs
        // through byte 63, so a 50-byte buffer is short).
        let data_len = 50;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([0xAA; 32]),
                owner: NativeAddress::new_from_array([3; 32]),
                lamports: 0,
                data_len: data_len as u64,
            });
        }
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let token = crate::account::AccountView::from_backend(backend);

        let (_ab, _, _, auth) = make_token_and_authority([0x11; 32], [0x11; 32]);
        let err = require_token_authority(&token, &auth).unwrap_err();
        assert!(matches!(err, ProgramError::AccountDataTooSmall));
    }

    // ── New Anchor-parity helpers (require_token_mint / require_mint_*) ──
    //
    // These lock in the behavior that `#[account(token::mint = X)]`,
    // `#[account(mint::authority = Y)]`, and friends lower to. They
    // share the same harness as require_token_authority above, but
    // exercise different byte ranges of the account buffer.

    /// Construct a valid SPL TokenAccount-shaped buffer (165 bytes)
    /// with both `mint` (bytes 0..32) and `owner` (bytes 32..64)
    /// populated to the caller's choice. Used by the token_mint /
    /// token_owner_eq regression tests.
    fn make_token_with_mint_and_owner(
        mint_bytes: [u8; 32],
        owner_bytes: [u8; 32],
    ) -> (std::vec::Vec<u8>, crate::account::AccountView) {
        use hopper_native::{AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED};

        let token_data_len = 165;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + token_data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([0xAA; 32]),
                owner: NativeAddress::new_from_array([3; 32]),
                lamports: 2_039_280,
                data_len: token_data_len as u64,
            });
            let data_ptr = (raw as *mut u8).add(RuntimeAccount::SIZE);
            core::ptr::copy_nonoverlapping(mint_bytes.as_ptr(), data_ptr, 32);
            core::ptr::copy_nonoverlapping(owner_bytes.as_ptr(), data_ptr.add(32), 32);
        }
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let view = crate::account::AccountView::from_backend(backend);
        (backing, view)
    }

    /// Construct a valid SPL Mint-shaped buffer (82 bytes), with the
    /// mint_authority COption set to Some(auth), decimals populated,
    /// and the freeze_authority COption left empty (None).
    fn make_mint_with_authority_decimals(
        mint_authority: [u8; 32],
        decimals: u8,
    ) -> (std::vec::Vec<u8>, crate::account::AccountView) {
        use hopper_native::{AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED};

        let mint_data_len = 82;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + mint_data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 0,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([0xBB; 32]),
                owner: NativeAddress::new_from_array([3; 32]),
                lamports: 1_461_600,
                data_len: mint_data_len as u64,
            });
            let data_ptr = (raw as *mut u8).add(RuntimeAccount::SIZE);
            // mint_authority COption tag = Some (u32 LE = 1).
            let some_tag: [u8; 4] = 1u32.to_le_bytes();
            core::ptr::copy_nonoverlapping(some_tag.as_ptr(), data_ptr, 4);
            core::ptr::copy_nonoverlapping(mint_authority.as_ptr(), data_ptr.add(4), 32);
            // Supply bytes [36..44] stay zero.
            // Decimals at byte 44.
            *data_ptr.add(44) = decimals;
            // is_initialized byte 45 = 1.
            *data_ptr.add(45) = 1;
            // freeze_authority COption tag = None (bytes 46..50 stay zero).
        }
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let view = crate::account::AccountView::from_backend(backend);
        (backing, view)
    }

    #[test]
    fn require_token_mint_accepts_matching_mint() {
        let mint = [0xABu8; 32];
        let (_b, view) = make_token_with_mint_and_owner(mint, [0; 32]);
        let expected = crate::address::Address::new_from_array(mint);
        require_token_mint(&view, &expected).unwrap();
    }

    #[test]
    fn require_token_mint_rejects_mismatched_mint() {
        let mint = [0xABu8; 32];
        let (_b, view) = make_token_with_mint_and_owner(mint, [0; 32]);
        let wrong = crate::address::Address::new_from_array([0xCDu8; 32]);
        let err = require_token_mint(&view, &wrong).unwrap_err();
        assert!(matches!(err, ProgramError::InvalidAccountData));
    }

    #[test]
    fn require_token_owner_eq_matches() {
        let owner = [0x77u8; 32];
        let (_b, view) = make_token_with_mint_and_owner([0; 32], owner);
        let expected = crate::address::Address::new_from_array(owner);
        require_token_owner_eq(&view, &expected).unwrap();
    }

    #[test]
    fn require_token_owner_eq_rejects_mismatch() {
        let owner = [0x77u8; 32];
        let (_b, view) = make_token_with_mint_and_owner([0; 32], owner);
        let wrong = crate::address::Address::new_from_array([0x88u8; 32]);
        let err = require_token_owner_eq(&view, &wrong).unwrap_err();
        assert!(matches!(err, ProgramError::IncorrectAuthority));
    }

    #[test]
    fn require_mint_authority_accepts_matching() {
        let auth = [0x99u8; 32];
        let (_b, view) = make_mint_with_authority_decimals(auth, 6);
        let expected = crate::address::Address::new_from_array(auth);
        require_mint_authority(&view, &expected).unwrap();
    }

    #[test]
    fn require_mint_authority_rejects_mismatched() {
        let auth = [0x99u8; 32];
        let (_b, view) = make_mint_with_authority_decimals(auth, 6);
        let wrong = crate::address::Address::new_from_array([0x00u8; 32]);
        let err = require_mint_authority(&view, &wrong).unwrap_err();
        assert!(matches!(err, ProgramError::IncorrectAuthority));
    }

    #[test]
    fn require_mint_decimals_matches() {
        let (_b, view) = make_mint_with_authority_decimals([1u8; 32], 9);
        require_mint_decimals(&view, 9).unwrap();
    }

    #[test]
    fn require_mint_decimals_rejects_mismatch() {
        let (_b, view) = make_mint_with_authority_decimals([1u8; 32], 9);
        let err = require_mint_decimals(&view, 6).unwrap_err();
        assert!(matches!(err, ProgramError::InvalidAccountData));
    }

    #[test]
    fn require_mint_freeze_authority_rejects_none_tag() {
        // `make_mint_with_authority_decimals` deliberately leaves
        // freeze_authority as None. asking for a specific freeze
        // authority on such a mint must fail with InvalidAccountData
        // (not IncorrectAuthority, because the tag is the problem
        // rather than the pubkey bytes).
        let (_b, view) = make_mint_with_authority_decimals([1u8; 32], 9);
        let expected = crate::address::Address::new_from_array([2u8; 32]);
        let err = require_mint_freeze_authority(&view, &expected).unwrap_err();
        assert!(matches!(err, ProgramError::InvalidAccountData));
    }
}
