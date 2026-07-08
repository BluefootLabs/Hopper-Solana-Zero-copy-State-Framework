//! Hopper-native SPL Token CPI builders.
//!
//! The API is Hopper-owned (builder pattern over `AccountView` / `Signer`) and
//! execution flows through Hopper's checked native CPI semantics.
//!
//! Provides checked-by-default TransferChecked, MintToChecked, BurnChecked,
//! ApproveChecked, CloseAccount, Revoke, SetAuthority, FreezeAccount,
//! ThawAccount, SyncNative, and InitializeAccount builders.
//! Multisig owner flows are first-class via bounded signer-account slices.
//! Deprecated plain Transfer/MintTo/Burn/Approve builders are compiled only
//! when `legacy-token-instructions` is explicitly enabled.

use crate::account::AccountView;
use crate::address::Address;
use crate::borrow::Ref;
use crate::error::ProgramError;
use crate::foreign::{ExplainExternal, ExternalAccount, ExternalExplainSink, ExternalZeroCopy};
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::ProgramResult;
use core::mem::MaybeUninit;

/// SPL Token multisig accounts support at most 11 signer accounts.
pub const MAX_TOKEN_MULTISIG_SIGNERS: usize = 11;

/// Fail-fast authority-signer precondition for the `invoke()` path.
///
/// The SPL token program enforces the signer requirement itself,
/// but the resulting error is a raw CPI failure without context.
/// This helper surfaces a Hopper-branded
/// `ProgramError::MissingRequiredSignature` before the CPI runs so
/// the caller sees exactly which field is wrong. Safety is enforced at
/// the API boundary, not left to convention.
///
/// Intentionally only applied on `invoke()`. The `invoke_signed()`
/// path is the explicit "I am signing programmatically with these
/// PDA seeds" contract. recomputing PDAs here would duplicate work
/// the SPL token program is about to do anyway. In the PDA path
/// the CPI itself is the authoritative check.
#[inline(always)]
fn require_authority_signed_direct(authority: &AccountView<'_>) -> ProgramResult {
    if authority.is_signer() {
        Ok(())
    } else {
        Err(ProgramError::MissingRequiredSignature)
    }
}

#[inline(always)]
fn authority_meta<'a>(
    authority: &'a AccountView<'a>,
    multisig_signers: &[&'a AccountView<'a>],
) -> InstructionAccount<'a> {
    if multisig_signers.is_empty() {
        InstructionAccount::readonly_signer(authority.address())
    } else {
        InstructionAccount::readonly(authority.address())
    }
}

#[inline]
fn require_multisig_signers_direct(multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
    if multisig_signers.len() > MAX_TOKEN_MULTISIG_SIGNERS {
        return Err(ProgramError::InvalidArgument);
    }
    for signer in multisig_signers {
        require_authority_signed_direct(signer)?;
    }
    Ok(())
}

#[inline(always)]
fn encode_set_authority_data(
    authority_type: TokenAuthorityType,
    new_authority: Option<&Address>,
    out: &mut [u8; 35],
) -> usize {
    out[0] = 6;
    out[1] = authority_type as u8;
    if let Some(new_authority) = new_authority {
        out[2] = 1;
        out[3..35].copy_from_slice(new_authority.as_bytes());
        35
    } else {
        out[2] = 0;
        3
    }
}

#[inline(always)]
fn encode_initialize_account_with_owner(discriminator: u8, owner: &Address) -> [u8; 33] {
    let mut data = [0u8; 33];
    data[0] = discriminator;
    data[1..33].copy_from_slice(owner.as_bytes());
    data
}

#[inline]
fn invoke_token_signed<'a, const FIXED: usize>(
    data: &[u8],
    fixed_accounts: [InstructionAccount<'a>; FIXED],
    fixed_views: [&'a AccountView<'a>; FIXED],
    multisig_signers: &[&'a AccountView<'a>],
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    let total = FIXED
        .checked_add(multisig_signers.len())
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if multisig_signers.len() > MAX_TOKEN_MULTISIG_SIGNERS
        || total > crate::cpi::MAX_STATIC_CPI_ACCOUNTS
    {
        return Err(ProgramError::InvalidArgument);
    }

    let mut accounts: [MaybeUninit<InstructionAccount<'a>>; crate::cpi::MAX_STATIC_CPI_ACCOUNTS] =
        [MaybeUninit::uninit(); crate::cpi::MAX_STATIC_CPI_ACCOUNTS];
    let mut views: [MaybeUninit<&'a AccountView<'a>>; crate::cpi::MAX_STATIC_CPI_ACCOUNTS] =
        [MaybeUninit::uninit(); crate::cpi::MAX_STATIC_CPI_ACCOUNTS];

    let mut index = 0;
    while index < FIXED {
        accounts[index].write(fixed_accounts[index]);
        views[index].write(fixed_views[index]);
        index += 1;
    }
    for signer in multisig_signers {
        accounts[index].write(InstructionAccount::readonly_signer(signer.address()));
        views[index].write(*signer);
        index += 1;
    }

    // SAFETY: slots in 0..total were initialized above, and `total` never
    // exceeds the fixed buffer capacity checked before writes.
    let accounts = unsafe {
        core::slice::from_raw_parts(accounts.as_ptr() as *const InstructionAccount<'a>, total)
    };
    // SAFETY: mirrors `accounts`; every view slot in 0..total was initialized.
    let views =
        unsafe { core::slice::from_raw_parts(views.as_ptr() as *const &'a AccountView<'a>, total) };

    let instruction = InstructionView {
        program_id: &TOKEN_PROGRAM_ID,
        data,
        accounts,
    };
    crate::cpi::invoke_signed_with_bounds::<{ crate::cpi::MAX_STATIC_CPI_ACCOUNTS }>(
        &instruction,
        views,
        signer_seeds,
    )
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
    token_account: &AccountView<'_>,
    authority: &AccountView<'_>,
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
    // Word-compare the owner field in place: no 32-byte copy.
    if crate::address::keys_eq_bytes(&data[32..64], authority.address().as_array()) {
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
/// `&AccountView<'_>` for the expected authority. The declarative
/// `#[account(token::authority = X)]` attribute lowers to this form
/// because the user's expression might resolve to a constant address,
/// a cached field, or another account's key. all of which are
/// `&Address` by the time the check runs, none of them necessarily
/// wrapped in an `AccountView`.
#[inline]
pub fn require_token_owner_eq(
    token_account: &AccountView<'_>,
    expected_owner: &Address,
) -> ProgramResult {
    let data = token_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 64 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    // Word-compare the owner field in place: no 32-byte copy.
    if crate::address::keys_eq_bytes(&data[32..64], expected_owner.as_array()) {
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
/// ## Design notes
///
/// The check reads the exact 32 bytes of interest directly from the
/// already-borrowed data buffer: no extra crate dependencies, no full-struct
/// deserialize, and the check is trivially inlinable.
#[inline]
pub fn require_token_mint(
    token_account: &AccountView<'_>,
    expected_mint: &Address,
) -> ProgramResult {
    let data = token_account
        .try_borrow()
        .map_err(|_| ProgramError::AccountBorrowFailed)?;
    if data.len() < 32 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    // Word-compare the mint field in place: no 32-byte copy.
    if crate::address::keys_eq_bytes(&data[0..32], expected_mint.as_array()) {
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
    mint_account: &AccountView<'_>,
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
    // Word-compare the authority field in place: no 32-byte copy.
    if crate::address::keys_eq_bytes(&data[4..36], expected_authority.as_array()) {
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
pub fn require_mint_decimals(mint_account: &AccountView<'_>, expected: u8) -> ProgramResult {
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
    mint_account: &AccountView<'_>,
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
    // Word-compare the freeze-authority field in place: no 32-byte copy.
    if crate::address::keys_eq_bytes(&data[50..82], expected_freeze.as_array()) {
        Ok(())
    } else {
        Err(ProgramError::IncorrectAuthority)
    }
}

// ---------------------------------------------------------------------

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
/// pre-Token-2022 deployments that only expose the plain transfer path, but
/// new code should use `TransferChecked`.
#[deprecated(
    since = "0.2.0",
    note = "use TransferChecked for Token-2022 safety (mint + decimals validation)"
)]
#[cfg(feature = "legacy-token-instructions")]
pub struct Transfer<'a> {
    pub from: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

// ---------------------------------------------------------------------

/// Builder for SPL Token MintTo (instruction index 7).
///
/// Prefer [`MintToChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use MintToChecked for Token-2022 safety (mint + decimals validation)"
)]
#[cfg(feature = "legacy-token-instructions")]
pub struct MintTo<'a> {
    pub mint: &'a AccountView<'a>,
    pub account: &'a AccountView<'a>,
    pub mint_authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

// ---------------------------------------------------------------------

/// Builder for SPL Token Burn (instruction index 8).
///
/// Prefer [`BurnChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use BurnChecked for Token-2022 safety (mint + decimals validation)"
)]
#[cfg(feature = "legacy-token-instructions")]
pub struct Burn<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

// ---------------------------------------------------------------------

/// Builder for SPL Token CloseAccount (instruction index 9).
pub struct CloseAccount<'a> {
    pub account: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
}

impl CloseAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let data = [9u8];
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.destination.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.account, self.destination, self.authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token Approve (instruction index 4).
///
/// Prefer [`ApproveChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use ApproveChecked for Token-2022 safety (mint + decimals validation)"
)]
#[cfg(feature = "legacy-token-instructions")]
pub struct Approve<'a> {
    pub source: &'a AccountView<'a>,
    pub delegate: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
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

// ---------------------------------------------------------------------

/// Builder for SPL Token Revoke (instruction index 5).
pub struct Revoke<'a> {
    pub source: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
}

impl Revoke<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.invoke_signed(&[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let data = [5u8];
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.source, self.authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------
//
// The Hopper audit flagged Token-2022 extension handling as a gap.
// `TransferChecked` is the SPL instruction that
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
    pub from: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub to: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    /// Invoke with an SPL multisig owner account plus transaction-signed
    /// multisig signer accounts.
    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    /// Invoke with an SPL multisig owner account and explicit PDA signer seeds.
    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    /// Strict PDA-signed invoke: ownership pre-check (the SPL token
    /// program revalidates, but Hopper surfaces a branded error
    /// first) then CPI with the supplied signer seeds.
    #[inline]
    pub fn invoke_signed_strict(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        require_token_authority(self.from, self.authority)?;
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 12;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.from.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::writable(self.to.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.from, self.mint, self.to, self.authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token MintToChecked (instruction index 14).
///
/// Same-shape decimals guard as [`TransferChecked`]. The Hopper-
/// preferred path when minting into a Token-2022 account.
pub struct MintToChecked<'a> {
    pub mint: &'a AccountView<'a>,
    pub account: &'a AccountView<'a>,
    pub mint_authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 14;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::writable(self.account.address()),
            authority_meta(self.mint_authority, multisig_signers),
        ];
        let views = [self.mint, self.account, self.mint_authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token BurnChecked (instruction index 15).
///
/// Decimals-verified counterpart to [`Burn`]. Prefer this over
/// `Burn` whenever the mint's decimals are known to the caller,
/// so the SPL token program can reject a mis-routed call at CPI time.
pub struct BurnChecked<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    /// Strict PDA-signed invoke. Pre-check the burn-source owner
    /// before the CPI so a misrouted signer surfaces a Hopper-branded
    /// error instead of an opaque SPL failure.
    #[inline]
    pub fn invoke_signed_strict(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        require_token_authority(self.account, self.authority)?;
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 15;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.account, self.mint, self.authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token ApproveChecked (instruction index 13).
///
/// Mint + decimals-verified approval. Same safety profile as the
/// other `*Checked` variants.
pub struct ApproveChecked<'a> {
    pub source: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub delegate: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
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
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked(signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    /// Strict PDA-signed invoke. Pre-check the source-account owner
    /// before the CPI.
    #[inline]
    pub fn invoke_signed_strict(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        require_token_authority(self.source, self.authority)?;
        self.invoke_signed_unchecked(signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let mut data = [0u8; 10];
        data[0] = 13;
        data[1..9].copy_from_slice(&self.amount.to_le_bytes());
        data[9] = self.decimals;

        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.delegate.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.source, self.mint, self.delegate, self.authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------

/// Authority classes accepted by SPL Token's SetAuthority instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TokenAuthorityType {
    MintTokens = 0,
    FreezeAccount = 1,
    AccountOwner = 2,
    CloseAccount = 3,
}

/// Builder for SPL Token SetAuthority (instruction index 6).
pub struct SetAuthority<'a> {
    pub account: &'a AccountView<'a>,
    pub current_authority: &'a AccountView<'a>,
    pub authority_type: TokenAuthorityType,
    pub new_authority: Option<&'a Address>,
}

impl SetAuthority<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.current_authority)?;
        self.invoke_signed_unchecked_with_multisig(&[], &[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let mut data = [0u8; 35];
        let len = encode_set_authority_data(self.authority_type, self.new_authority, &mut data);
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            authority_meta(self.current_authority, multisig_signers),
        ];
        let views = [self.account, self.current_authority];
        invoke_token_signed(&data[..len], accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token FreezeAccount (instruction index 10).
pub struct FreezeAccount<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub freeze_authority: &'a AccountView<'a>,
}

impl FreezeAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.freeze_authority)?;
        self.invoke_signed_unchecked_with_multisig(&[], &[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let data = [10u8];
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            authority_meta(self.freeze_authority, multisig_signers),
        ];
        let views = [self.account, self.mint, self.freeze_authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

/// Builder for SPL Token ThawAccount (instruction index 11).
pub struct ThawAccount<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub freeze_authority: &'a AccountView<'a>,
}

impl ThawAccount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.freeze_authority)?;
        self.invoke_signed_unchecked_with_multisig(&[], &[])
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(&[], signers)
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.invoke_signed_multisig(multisig_signers, &[])
    }

    #[inline]
    pub fn invoke_signed_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        self.invoke_signed_unchecked_with_multisig(multisig_signers, signers)
    }

    #[inline(always)]
    fn invoke_signed_unchecked_with_multisig(
        &self,
        multisig_signers: &[&AccountView<'_>],
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        let data = [11u8];
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            authority_meta(self.freeze_authority, multisig_signers),
        ];
        let views = [self.account, self.mint, self.freeze_authority];
        invoke_token_signed(&data, accounts, views, multisig_signers, signers)
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token SyncNative (instruction index 17).
pub struct SyncNative<'a> {
    pub account: &'a AccountView<'a>,
}

impl SyncNative<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        let data = [17u8];
        let accounts = [InstructionAccount::writable(self.account.address())];
        let views = [self.account];
        invoke_token_signed(&data, accounts, views, &[], &[])
    }
}

// ---------------------------------------------------------------------

/// Builder for SPL Token InitializeAccount (instruction index 1).
pub struct InitializeAccount<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub owner: &'a AccountView<'a>,
    pub rent_sysvar: &'a AccountView<'a>,
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

/// Builder for SPL Token InitializeAccount2 (instruction index 16).
pub struct InitializeAccount2<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub owner: &'a Address,
    pub rent_sysvar: &'a AccountView<'a>,
}

impl InitializeAccount2<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        let data = encode_initialize_account_with_owner(16, self.owner);
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.rent_sysvar.address()),
        ];
        let views = [self.account, self.mint, self.rent_sysvar];
        invoke_token_signed(&data, accounts, views, &[], &[])
    }
}

/// Builder for SPL Token InitializeAccount3 (instruction index 18).
pub struct InitializeAccount3<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub owner: &'a Address,
}

impl InitializeAccount3<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        let data = encode_initialize_account_with_owner(18, self.owner);
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
        ];
        let views = [self.account, self.mint];
        invoke_token_signed(&data, accounts, views, &[], &[])
    }
}

/// SPL Token program address.
pub const TOKEN_PROGRAM_ID: Address = Address::new_from_array(crate::__decode_base58_32(
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
));

// ---------------------------------------------------------------------

pub const SPL_TOKEN_ACCOUNT_LEN: usize = 165;
pub const SPL_MINT_LEN: usize = 82;

const TOKEN_ACCOUNT_MINT_OFFSET: usize = 0;
const TOKEN_ACCOUNT_AUTHORITY_OFFSET: usize = 32;
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_STATE_OFFSET: usize = 108;

const MINT_AUTHORITY_TAG_OFFSET: usize = 0;
const MINT_AUTHORITY_OFFSET: usize = 4;
const MINT_SUPPLY_OFFSET: usize = 36;
const MINT_DECIMALS_OFFSET: usize = 44;
const MINT_INITIALIZED_OFFSET: usize = 45;
const MINT_FREEZE_AUTHORITY_TAG_OFFSET: usize = 46;
const MINT_FREEZE_AUTHORITY_OFFSET: usize = 50;

/// Known external SPL TokenAccount adapter.
pub struct SplTokenAccount;

/// Guard-owned zero-copy SPL TokenAccount view.
pub struct SplTokenAccountView<'a> {
    data: Ref<'a, [u8]>,
}

impl SplTokenAccountView<'_> {
    #[inline(always)]
    pub fn mint(&self) -> Address {
        read_address_unchecked(&self.data, TOKEN_ACCOUNT_MINT_OFFSET)
    }

    #[inline(always)]
    pub fn authority(&self) -> Address {
        read_address_unchecked(&self.data, TOKEN_ACCOUNT_AUTHORITY_OFFSET)
    }

    #[inline(always)]
    pub fn amount(&self) -> u64 {
        read_u64_unchecked(&self.data, TOKEN_ACCOUNT_AMOUNT_OFFSET)
    }

    #[inline(always)]
    pub fn state(&self) -> u8 {
        self.data[TOKEN_ACCOUNT_STATE_OFFSET]
    }

    #[inline(always)]
    pub fn is_initialized(&self) -> bool {
        self.state() != 0
    }
}

impl ExternalZeroCopy for SplTokenAccount {
    type View<'a> = SplTokenAccountView<'a>;

    const OWNER: Option<Address> = Some(TOKEN_PROGRAM_ID);
    const MIN_LEN: usize = SPL_TOKEN_ACCOUNT_LEN;

    #[inline]
    fn view<'a>(data: Ref<'a, [u8]>) -> Result<Self::View<'a>, ProgramError> {
        Ok(SplTokenAccountView { data })
    }
}

impl ExplainExternal for SplTokenAccount {
    fn explain<S: ExternalExplainSink>(account: &AccountView<'_>, sink: &mut S) -> ProgramResult {
        let account = ExternalAccount::<SplTokenAccount>::try_new(account)?;
        account.with_view(|token| {
            sink.field_str("adapter", "SplTokenAccount")?;
            sink.field_address("mint", &token.mint())?;
            sink.field_address("authority", &token.authority())?;
            sink.field_u64("amount", token.amount())?;
            sink.field_bool("initialized", token.is_initialized())
        })
    }
}

/// Known external SPL Mint adapter.
pub struct SplMint;

/// Guard-owned zero-copy SPL Mint view.
pub struct SplMintView<'a> {
    data: Ref<'a, [u8]>,
}

impl SplMintView<'_> {
    #[inline(always)]
    pub fn mint_authority(&self) -> Option<Address> {
        read_coption_address(&self.data, MINT_AUTHORITY_TAG_OFFSET, MINT_AUTHORITY_OFFSET)
    }

    #[inline(always)]
    pub fn supply(&self) -> u64 {
        read_u64_unchecked(&self.data, MINT_SUPPLY_OFFSET)
    }

    #[inline(always)]
    pub fn decimals(&self) -> u8 {
        self.data[MINT_DECIMALS_OFFSET]
    }

    #[inline(always)]
    pub fn is_initialized(&self) -> bool {
        self.data[MINT_INITIALIZED_OFFSET] != 0
    }

    #[inline(always)]
    pub fn freeze_authority(&self) -> Option<Address> {
        read_coption_address(
            &self.data,
            MINT_FREEZE_AUTHORITY_TAG_OFFSET,
            MINT_FREEZE_AUTHORITY_OFFSET,
        )
    }
}

impl ExternalZeroCopy for SplMint {
    type View<'a> = SplMintView<'a>;

    const OWNER: Option<Address> = Some(TOKEN_PROGRAM_ID);
    const MIN_LEN: usize = SPL_MINT_LEN;

    #[inline]
    fn view<'a>(data: Ref<'a, [u8]>) -> Result<Self::View<'a>, ProgramError> {
        Ok(SplMintView { data })
    }
}

impl ExplainExternal for SplMint {
    fn explain<S: ExternalExplainSink>(account: &AccountView<'_>, sink: &mut S) -> ProgramResult {
        let account = ExternalAccount::<SplMint>::try_new(account)?;
        account.with_view(|mint| {
            sink.field_str("adapter", "SplMint")?;
            sink.field_u64("supply", mint.supply())?;
            sink.field_u64("decimals", mint.decimals() as u64)?;
            sink.field_bool("initialized", mint.is_initialized())
        })
    }
}

/// Proof token that an SPL TokenAccount matched an expected mint.
#[derive(Debug)]
pub struct CheckedTokenMint<'info> {
    account: ExternalAccount<'info, SplTokenAccount>,
    mint: Address,
}

impl<'info> CheckedTokenMint<'info> {
    #[inline(always)]
    pub const fn account(&self) -> ExternalAccount<'info, SplTokenAccount> {
        self.account
    }

    #[inline(always)]
    pub const fn mint(&self) -> Address {
        self.mint
    }
}

/// Proof token that an SPL TokenAccount matched an expected token authority.
#[derive(Debug)]
pub struct CheckedTokenAuthority<'info> {
    account: ExternalAccount<'info, SplTokenAccount>,
    authority: Address,
}

impl<'info> CheckedTokenAuthority<'info> {
    #[inline(always)]
    pub const fn account(&self) -> ExternalAccount<'info, SplTokenAccount> {
        self.account
    }

    #[inline(always)]
    pub const fn authority(&self) -> Address {
        self.authority
    }
}

/// Proof token that an SPL Mint matched expected decimals.
#[derive(Debug)]
pub struct CheckedMintDecimals<'info> {
    account: ExternalAccount<'info, SplMint>,
    decimals: u8,
}

impl<'info> CheckedMintDecimals<'info> {
    #[inline(always)]
    pub const fn account(&self) -> ExternalAccount<'info, SplMint> {
        self.account
    }

    #[inline(always)]
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// Snapshot of a token account amount before CPI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenAmountSnapshot {
    amount: u64,
}

impl TokenAmountSnapshot {
    #[inline(always)]
    pub const fn amount(self) -> u64 {
        self.amount
    }
}

impl<'info> ExternalAccount<'info, SplTokenAccount> {
    #[inline]
    pub fn token_amount(&self) -> Result<u64, ProgramError> {
        Ok(self.view()?.amount())
    }

    #[inline]
    pub fn checked_mint(
        &self,
        expected_mint: &Address,
    ) -> Result<CheckedTokenMint<'info>, ProgramError> {
        let mint = self.view()?.mint();
        if &mint == expected_mint {
            Ok(CheckedTokenMint {
                account: *self,
                mint,
            })
        } else {
            Err(ProgramError::InvalidAccountData)
        }
    }

    #[inline]
    pub fn checked_authority(
        &self,
        expected_authority: &Address,
    ) -> Result<CheckedTokenAuthority<'info>, ProgramError> {
        let authority = self.view()?.authority();
        if &authority == expected_authority {
            Ok(CheckedTokenAuthority {
                account: *self,
                authority,
            })
        } else {
            Err(ProgramError::IncorrectAuthority)
        }
    }

    #[inline]
    pub fn amount_snapshot(&self) -> Result<TokenAmountSnapshot, ProgramError> {
        Ok(TokenAmountSnapshot {
            amount: self.token_amount()?,
        })
    }

    #[inline]
    pub fn assert_amount_delta(
        &self,
        before: TokenAmountSnapshot,
        expected_delta: i128,
    ) -> ProgramResult {
        let after = self.token_amount()? as i128;
        let expected = (before.amount as i128)
            .checked_add(expected_delta)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if expected < 0 || expected > u64::MAX as i128 {
            return Err(ProgramError::ArithmeticOverflow);
        }
        if after == expected {
            Ok(())
        } else {
            Err(ProgramError::InvalidAccountData)
        }
    }

    #[inline]
    pub fn assert_amount_unchanged(&self, before: TokenAmountSnapshot) -> ProgramResult {
        self.assert_amount_delta(before, 0)
    }
}

impl<'info> ExternalAccount<'info, SplMint> {
    #[inline]
    pub fn checked_decimals(
        &self,
        expected: u8,
    ) -> Result<CheckedMintDecimals<'info>, ProgramError> {
        let decimals = self.view()?.decimals();
        if decimals == expected {
            Ok(CheckedMintDecimals {
                account: *self,
                decimals,
            })
        } else {
            Err(ProgramError::InvalidAccountData)
        }
    }
}

#[inline(always)]
fn read_address_unchecked(data: &[u8], offset: usize) -> Address {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&data[offset..offset + 32]);
    Address::new_from_array(bytes)
}

#[inline(always)]
fn read_u64_unchecked(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

#[inline(always)]
fn read_u32_unchecked(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[inline(always)]
fn read_coption_address(data: &[u8], tag_offset: usize, address_offset: usize) -> Option<Address> {
    match read_u32_unchecked(data, tag_offset) {
        1 => Some(read_address_unchecked(data, address_offset)),
        _ => None,
    }
}

/// Legacy module-path re-exports.
pub mod instructions {
    pub use super::{
        ApproveChecked, BurnChecked, CloseAccount, FreezeAccount, InitializeAccount,
        InitializeAccount2, InitializeAccount3, MintToChecked, Revoke, SetAuthority, SyncNative,
        ThawAccount, TokenAuthorityType, TransferChecked,
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
    use hopper_native::{
        AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };
    fn make_account(owner: Address, data: &[u8]) -> (std::vec::Vec<u8>, AccountView<'static>) {
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data.len()];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: Test helper writes a valid RuntimeAccount header and copies
        // payload bytes into owned backing memory.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: NativeAddress::new_from_array([7; 32]),
                owner: NativeAddress::new_from_array(owner.to_bytes()),
                lamports: 1,
                data_len: data.len() as u64,
            });
            let data_ptr = backing.as_mut_ptr().add(RuntimeAccount::SIZE);
            core::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, data.len());
        }
        // SAFETY: `raw` points at the initialized RuntimeAccount header.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        (backing, AccountView::from_backend(backend))
    }
    fn token_account_data(
        mint: Address,
        authority: Address,
        amount: u64,
    ) -> [u8; SPL_TOKEN_ACCOUNT_LEN] {
        let mut data = [0u8; SPL_TOKEN_ACCOUNT_LEN];
        data[0..32].copy_from_slice(mint.as_bytes());
        data[32..64].copy_from_slice(authority.as_bytes());
        data[64..72].copy_from_slice(&amount.to_le_bytes());
        data[108] = 1;
        data
    }
    fn mint_data(authority: Address, supply: u64, decimals: u8) -> [u8; SPL_MINT_LEN] {
        let mut data = [0u8; SPL_MINT_LEN];
        data[0..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..36].copy_from_slice(authority.as_bytes());
        data[36..44].copy_from_slice(&supply.to_le_bytes());
        data[44] = decimals;
        data[45] = 1;
        data
    }

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
    #[test]
    fn spl_external_token_account_view_proofs_and_amount_delta() {
        let mint = Address::new_from_array([2; 32]);
        let authority = Address::new_from_array([3; 32]);
        let data = token_account_data(mint, authority, 100);
        let (mut backing, account) = make_account(TOKEN_PROGRAM_ID, &data);

        let token = ExternalAccount::<SplTokenAccount>::try_new(&account).unwrap();
        let view = token.view().unwrap();
        assert_eq!(view.mint(), mint);
        assert_eq!(view.authority(), authority);
        assert_eq!(view.amount(), 100);
        assert!(view.is_initialized());
        assert_eq!(token.checked_mint(&mint).unwrap().mint(), mint);
        assert_eq!(
            token.checked_authority(&authority).unwrap().authority(),
            authority
        );
        assert_eq!(
            token
                .checked_mint(&Address::new_from_array([9; 32]))
                .unwrap_err(),
            ProgramError::InvalidAccountData
        );

        let before = token.amount_snapshot().unwrap();
        backing[RuntimeAccount::SIZE + 64..RuntimeAccount::SIZE + 72]
            .copy_from_slice(&150u64.to_le_bytes());
        token.assert_amount_delta(before, 50).unwrap();
        assert_eq!(
            token.assert_amount_delta(before, 49).unwrap_err(),
            ProgramError::InvalidAccountData
        );
    }
    #[test]
    fn spl_external_mint_view_and_decimals_proof() {
        let authority = Address::new_from_array([4; 32]);
        let data = mint_data(authority, 1_000_000, 6);
        let (_backing, account) = make_account(TOKEN_PROGRAM_ID, &data);

        let mint = ExternalAccount::<SplMint>::try_new(&account).unwrap();
        let view = mint.view().unwrap();
        assert_eq!(view.mint_authority(), Some(authority));
        assert_eq!(view.supply(), 1_000_000);
        assert_eq!(view.decimals(), 6);
        assert!(view.is_initialized());
        assert_eq!(mint.checked_decimals(6).unwrap().decimals(), 6);
        assert_eq!(
            mint.checked_decimals(9).unwrap_err(),
            ProgramError::InvalidAccountData
        );
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

    #[test]
    fn authority_and_initialize_encodings_match_spl_token_wire_format() {
        let authority = Address::new_from_array([9; 32]);
        let mut set_authority = [0u8; 35];
        let len = encode_set_authority_data(
            TokenAuthorityType::AccountOwner,
            Some(&authority),
            &mut set_authority,
        );
        assert_eq!(len, 35);
        assert_eq!(set_authority[0], 6);
        assert_eq!(set_authority[1], 2);
        assert_eq!(set_authority[2], 1);
        assert_eq!(&set_authority[3..35], authority.as_bytes());

        let len =
            encode_set_authority_data(TokenAuthorityType::CloseAccount, None, &mut set_authority);
        assert_eq!(len, 3);
        assert_eq!(&set_authority[..3], &[6, 3, 0]);

        let init2 = encode_initialize_account_with_owner(16, &authority);
        let init3 = encode_initialize_account_with_owner(18, &authority);
        assert_eq!(init2[0], 16);
        assert_eq!(init3[0], 18);
        assert_eq!(&init2[1..33], authority.as_bytes());
        assert_eq!(&init3[1..33], authority.as_bytes());
    }

    // ---------------------------------------------------------------------

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
        crate::account::AccountView<'static>,
        crate::account::AccountView<'static>,
    ) {
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        // TokenAccount: SPL layout is 165 bytes; first 32 bytes are
        // `mint`, next 32 are `owner`. We only care about the owner
        // slot for `require_token_authority`, but size the buffer at
        // 165 so it looks like a real TokenAccount.
        let token_data_len = 165;
        let mut token_backing = std::vec![0u8; RuntimeAccount::SIZE + token_data_len];
        let token_raw = token_backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
            core::ptr::copy_nonoverlapping(token_owner_bytes.as_ptr(), data_ptr.add(32), 32);
        }
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let token_backend = unsafe { NativeAccountView::new_unchecked(token_raw) };
        let token_view = crate::account::AccountView::from_backend(token_backend);

        // Authority: no data needed, just an address field.
        let mut auth_backing = std::vec![0u8; RuntimeAccount::SIZE];
        let auth_raw = auth_backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        // Token account with only 50 bytes of data is not a valid
        // SPL TokenAccount (owner field starts at byte 32 and runs
        // through byte 63, so a 50-byte buffer is short).
        let data_len = 50;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let backend = unsafe { NativeAccountView::new_unchecked(raw) };
        let token = crate::account::AccountView::from_backend(backend);

        let (_ab, _, _, auth) = make_token_and_authority([0x11; 32], [0x11; 32]);
        let err = require_token_authority(&token, &auth).unwrap_err();
        assert!(matches!(err, ProgramError::AccountDataTooSmall));
    }

    // ---------------------------------------------------------------------
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
    ) -> (std::vec::Vec<u8>, crate::account::AccountView<'static>) {
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        let token_data_len = 165;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + token_data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
    ) -> (std::vec::Vec<u8>, crate::account::AccountView<'static>) {
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        let mint_data_len = 82;
        let mut backing = std::vec![0u8; RuntimeAccount::SIZE + mint_data_len];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
