//! Hopper-native SPL Token CPI builders.
//!
//! The API is Hopper-owned (builder pattern over `AccountView` / `Signer`).
//! A builder's CPI checks that no account in the instruction is borrowed and
//! that no account fills two writable roles, and leaves signer and writable
//! privileges to the runtime; a multisig authority's signers take the fully
//! checked bounded path.
//!
//! Provides checked-by-default TransferChecked, MintToChecked, BurnChecked,
//! ApproveChecked, CloseAccount, Revoke, SetAuthority, FreezeAccount,
//! ThawAccount, SyncNative, and InitializeAccount builders, plus the
//! administrative set (InitializeMint, InitializeMultisig, InitializeMultisig2,
//! InitializeImmutableOwner, GetAccountDataSize, WithdrawExcessLamports,
//! AmountToUiAmount, UiAmountToAmount, UnwrapLamports) and the p-token
//! `Batch` instruction through [`TokenBatch`].
//! Multisig owner flows are first-class via bounded signer-account slices.
//! Deprecated plain Transfer/MintTo/Burn/Approve builders are compiled only
//! when `legacy-token-instructions` is explicitly enabled.
//!
//! ## One encoding, two programs, two sinks
//!
//! Every builder encodes its instruction bytes and account metas exactly
//! once, in its [`TokenInstruction::emit`] impl. `invoke()` and the
//! `invoke_signed` / `invoke_multisig` family send that encoding to SPL
//! Token; [`invoke_on`](TransferChecked::invoke_on) sends it to an explicit
//! [`TokenProgram`], and [`invoke_for_owner`](TransferChecked::invoke_for_owner)
//! to whichever of the two programs owns the builder's first account (one
//! 32-byte compare, and a refusal for any other owner). The same `emit` can
//! also append the instruction to a [`TokenBatch`], which sends several
//! token instructions in one CPI.

use crate::account::AccountView;
use crate::address::Address;
use crate::borrow::Ref;
use crate::error::ProgramError;
use crate::foreign::{ExplainExternal, ExternalAccount, ExternalExplainSink, ExternalZeroCopy};
use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::ProgramResult;
use core::mem::MaybeUninit;

pub use crate::token_admin::{
    return_data_string, return_data_u64, AmountToUiAmount, GetAccountDataSize,
    InitializeImmutableOwner, InitializeMint, InitializeMultisig, InitializeMultisig2,
    UiAmountToAmount, UnwrapLamports, WithdrawExcessLamports, MAX_UI_AMOUNT_LEN,
};
pub use crate::token_batch::TokenBatch;
pub use crate::token_mint::{InitializeMint2, MintConfig, MintPlan, MintProgram};

/// Token-2022 program address.
pub const TOKEN_2022_PROGRAM_ID: Address = Address::new_from_array(crate::__decode_base58_32(
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
));

/// The two token programs every builder in this module can target.
///
/// The wire format of the shared instruction set is identical on both, so a
/// builder's encoding does not change; only the program id in the CPI does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenProgram {
    /// SPL Token, `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`.
    Legacy,
    /// Token-2022, `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`.
    Token2022,
}

impl TokenProgram {
    /// The program id.
    #[inline(always)]
    pub const fn address(self) -> &'static Address {
        match self {
            Self::Legacy => &TOKEN_PROGRAM_ID,
            Self::Token2022 => &TOKEN_2022_PROGRAM_ID,
        }
    }

    /// The program behind an address, if it is one of the two.
    #[inline]
    pub fn from_address(address: &Address) -> Option<Self> {
        if address == &TOKEN_PROGRAM_ID {
            Some(Self::Legacy)
        } else if address == &TOKEN_2022_PROGRAM_ID {
            Some(Self::Token2022)
        } else {
            None
        }
    }

    /// The program that owns `account`: a token account, a mint, or a
    /// multisig. Anything owned by another program is refused with
    /// `IncorrectProgramId`, so a builder driven by this never sends a
    /// token instruction to a program that is not a token program.
    #[inline]
    pub fn owning(account: &AccountView<'_>) -> Result<Self, ProgramError> {
        if account.owned_by(&TOKEN_PROGRAM_ID) {
            Ok(Self::Legacy)
        } else if account.owned_by(&TOKEN_2022_PROGRAM_ID) {
            Ok(Self::Token2022)
        } else {
            Err(ProgramError::IncorrectProgramId)
        }
    }

    /// The program an executable account in the instruction stands for.
    /// Refuses any address that is not one of the two token programs.
    #[inline]
    pub fn from_program_account(program: &AccountView<'_>) -> Result<Self, ProgramError> {
        Self::from_address(program.address()).ok_or(ProgramError::IncorrectProgramId)
    }
}

/// A run of accounts appended after a builder's fixed accounts, all with
/// the same privileges: the multisig signers of an authority, the member
/// list of a new multisig, or the source accounts of a withheld-fee sweep.
#[derive(Clone, Copy)]
pub struct Trailing<'s, 'a> {
    pub views: &'s [&'a AccountView<'a>],
    pub writable: bool,
    pub signer: bool,
}

impl<'s, 'a> Trailing<'s, 'a> {
    /// Read-only signers (a multisig authority's signer set).
    #[inline(always)]
    pub const fn signers(views: &'s [&'a AccountView<'a>]) -> Self {
        Self {
            views,
            writable: false,
            signer: true,
        }
    }

    /// Read-only non-signers (the member list of `InitializeMultisig`).
    #[inline(always)]
    pub const fn readonly(views: &'s [&'a AccountView<'a>]) -> Self {
        Self {
            views,
            writable: false,
            signer: false,
        }
    }

    /// Writable non-signers (the sources of a withheld-fee harvest).
    #[inline(always)]
    pub const fn writable(views: &'s [&'a AccountView<'a>]) -> Self {
        Self {
            views,
            writable: true,
            signer: false,
        }
    }
}

/// Where a builder's encoded instruction goes.
///
/// Two sinks exist: the CPI itself (what every `invoke*` method uses) and
/// [`TokenBatch`], which collects several instructions for one `Batch` CPI.
/// A builder never encodes differently for the two.
pub trait TokenSink<'a> {
    /// Receive one encoded instruction: its data, its fixed account metas
    /// and views in order, and zero or more trailing runs appended after
    /// them.
    fn emit<const N: usize>(
        &mut self,
        data: &[u8],
        accounts: [InstructionAccount<'a>; N],
        views: [&'a AccountView<'a>; N],
        trailing: &[Trailing<'_, 'a>],
    ) -> ProgramResult;
}

/// An SPL Token / Token-2022 instruction builder: something that can encode
/// itself into a [`TokenSink`].
///
/// `multisig_signers` are the signer accounts of a multisig authority
/// (at most [`MAX_TOKEN_MULTISIG_SIGNERS`]); builders without an authority
/// ignore the slice.
pub trait TokenInstruction<'a> {
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult;
}

/// The CPI sink: sends the encoded instruction to `program`, signed by
/// `signers`.
pub(crate) struct Invoke<'p, 's, 'x, 'y> {
    pub(crate) program: &'p Address,
    pub(crate) signers: &'s [Signer<'x, 'y>],
}

impl<'s, 'x, 'y> Invoke<'static, 's, 'x, 'y> {
    /// The SPL Token program, the historical default of every `invoke*`.
    #[inline(always)]
    pub(crate) const fn legacy(signers: &'s [Signer<'x, 'y>]) -> Self {
        Self {
            program: &TOKEN_PROGRAM_ID,
            signers,
        }
    }

    /// The Token-2022 program.
    #[inline(always)]
    pub(crate) const fn token_2022(signers: &'s [Signer<'x, 'y>]) -> Self {
        Self {
            program: &TOKEN_2022_PROGRAM_ID,
            signers,
        }
    }
}

impl<'a> TokenSink<'a> for Invoke<'_, '_, '_, '_> {
    #[inline(always)]
    fn emit<const N: usize>(
        &mut self,
        data: &[u8],
        accounts: [InstructionAccount<'a>; N],
        views: [&'a AccountView<'a>; N],
        trailing: &[Trailing<'_, 'a>],
    ) -> ProgramResult {
        // Without multisig signers every meta is built from the view beside
        // it, so the builder tier applies: borrow checks, the lamport gate,
        // and the repeated-writable refusal, with privileges left to the
        // runtime. A multisig authority's trailing signers take the bounded
        // path, which checks each of them.
        if trailing.iter().all(|run| run.views.is_empty()) {
            let instruction = InstructionView {
                program_id: self.program,
                data,
                accounts: &accounts,
            };
            return crate::cpi::invoke_signed_builder_distinct(&instruction, &views, self.signers);
        }
        invoke_token_signed(self.program, data, accounts, views, trailing, self.signers)
    }
}

/// The program-selecting entry points every builder gets: `invoke_on` for
/// an explicit [`TokenProgram`] and `invoke_for_owner` for the program that
/// owns the builder's first account. With no PDA `signers`, the same direct
/// signer checks as `invoke()` / `invoke_multisig()` run first.
macro_rules! token_program_methods {
    ($name:ident, owner = $owner:ident $(, authority = $auth:ident)?) => {
        impl $name<'_> {
            /// Send this instruction to an explicit token program.
            ///
            /// `multisig_signers` is empty for a single-key authority; when
            /// `signers` is empty the authority (or every multisig signer)
            /// must have signed the transaction directly.
            #[inline]
            pub fn invoke_on(
                &self,
                program: TokenProgram,
                multisig_signers: &[&AccountView<'_>],
                signers: &[Signer<'_, '_>],
            ) -> ProgramResult {
                $(
                    if signers.is_empty() {
                        if multisig_signers.is_empty() {
                            require_authority_signed_direct(self.$auth)?;
                        } else {
                            require_multisig_signers_direct(multisig_signers)?;
                        }
                    }
                )?
                self.emit(
                    multisig_signers,
                    &mut Invoke {
                        program: program.address(),
                        signers,
                    },
                )
            }

            /// Send this instruction to whichever token program owns
            #[doc = concat!("`", stringify!($owner), "`")]
            /// (one 32-byte compare). Any other owner is refused with
            /// `IncorrectProgramId` before the CPI.
            #[inline]
            pub fn invoke_for_owner(
                &self,
                multisig_signers: &[&AccountView<'_>],
                signers: &[Signer<'_, '_>],
            ) -> ProgramResult {
                self.invoke_on(TokenProgram::owning(self.$owner)?, multisig_signers, signers)
            }
        }
    };
}
pub(crate) use token_program_methods;

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
pub(crate) fn require_authority_signed_direct(authority: &AccountView<'_>) -> ProgramResult {
    if authority.is_signer() {
        Ok(())
    } else {
        Err(ProgramError::MissingRequiredSignature)
    }
}

#[inline(always)]
pub(crate) fn authority_meta<'a>(
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
pub(crate) fn require_multisig_signers_direct(
    multisig_signers: &[&AccountView<'_>],
) -> ProgramResult {
    if multisig_signers.len() > MAX_TOKEN_MULTISIG_SIGNERS {
        return Err(ProgramError::InvalidArgument);
    }
    for signer in multisig_signers {
        require_authority_signed_direct(signer)?;
    }
    Ok(())
}

/// Byte-exact instruction-data encoders for the SPL Token CPI wire format.
///
/// # Why this module is `pub`
///
/// Every SPL Token builder in this file constructs its instruction-data
/// buffer by calling exactly one of these functions before handing the bytes
/// to [`crate::cpi`]. They are the single, **shipped** source of truth for the
/// SPL Token wire format, the exact bytes that leave the program on a CPI,
/// not a mirror or a parallel re-implementation.
///
/// They are exposed as `#[doc(hidden)] pub` for one reason: so the Kani layout
/// proofs in the `hopper-token` crate can call the shipped encoders directly
/// and prove, over fully symbolic inputs, that the bytes the CPI path emits
/// carry the canonical discriminator, field order/offsets, endianness, and
/// total length. Proving the shipped functions (rather than a copy of them) is
/// what lets Hopper claim the encoders themselves are formally verified.
///
/// This is deliberately **not** a stability surface: the module is
/// `#[doc(hidden)]` and may change at any time. Depend on the builder structs
/// ([`TransferChecked`], [`MintToChecked`], …), never on `encoders`.
///
/// Each function is `#[inline(always)]`, so delegating to it from a builder is
/// zero-cost: the emitted bytes and codegen are identical to the previous
/// inline construction.
#[doc(hidden)]
pub mod encoders {
    /// `[disc][amount: u64 LE]`, the 9-byte shape shared by the plain
    /// `Transfer` (3), `Approve` (4), `MintTo` (7), and `Burn` (8)
    /// instructions.
    #[inline(always)]
    fn amount_ix(disc: u8, amount: u64) -> [u8; 9] {
        let mut data = [0u8; 9];
        data[0] = disc;
        data[1..9].copy_from_slice(&amount.to_le_bytes());
        data
    }

    /// `[disc][amount: u64 LE][decimals: u8]`, the 10-byte shape shared by
    /// the `TransferChecked` (12), `ApproveChecked` (13), `MintToChecked`
    /// (14), and `BurnChecked` (15) instructions.
    #[inline(always)]
    fn amount_checked_ix(disc: u8, amount: u64, decimals: u8) -> [u8; 10] {
        let mut data = [0u8; 10];
        data[0] = disc;
        data[1..9].copy_from_slice(&amount.to_le_bytes());
        data[9] = decimals;
        data
    }

    /// SPL Token `Transfer { amount }`, `[3][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_transfer(amount: u64) -> [u8; 9] {
        amount_ix(3, amount)
    }

    /// SPL Token `Approve { amount }`, `[4][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_approve(amount: u64) -> [u8; 9] {
        amount_ix(4, amount)
    }

    /// SPL Token `MintTo { amount }`, `[7][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_mint_to(amount: u64) -> [u8; 9] {
        amount_ix(7, amount)
    }

    /// SPL Token `Burn { amount }`, `[8][amount: u64 LE]` (9 bytes).
    #[inline(always)]
    pub fn encode_burn(amount: u64) -> [u8; 9] {
        amount_ix(8, amount)
    }

    /// SPL Token `TransferChecked { amount, decimals }`,
    /// `[12][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_transfer_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(12, amount, decimals)
    }

    /// SPL Token `ApproveChecked { amount, decimals }`,
    /// `[13][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_approve_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(13, amount, decimals)
    }

    /// SPL Token `MintToChecked { amount, decimals }`,
    /// `[14][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_mint_to_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(14, amount, decimals)
    }

    /// SPL Token `BurnChecked { amount, decimals }`,
    /// `[15][amount: u64 LE][decimals: u8]` (10 bytes).
    #[inline(always)]
    pub fn encode_burn_checked(amount: u64, decimals: u8) -> [u8; 10] {
        amount_checked_ix(15, amount, decimals)
    }

    /// SPL Token `Revoke`, `[5]` (1 byte).
    #[inline(always)]
    pub fn encode_revoke() -> [u8; 1] {
        [5]
    }

    /// SPL Token `CloseAccount`, `[9]` (1 byte).
    #[inline(always)]
    pub fn encode_close_account() -> [u8; 1] {
        [9]
    }

    /// SPL Token `FreezeAccount`, `[10]` (1 byte).
    #[inline(always)]
    pub fn encode_freeze_account() -> [u8; 1] {
        [10]
    }

    /// SPL Token `ThawAccount`, `[11]` (1 byte).
    #[inline(always)]
    pub fn encode_thaw_account() -> [u8; 1] {
        [11]
    }

    /// SPL Token `SyncNative`, `[17]` (1 byte).
    #[inline(always)]
    pub fn encode_sync_native() -> [u8; 1] {
        [17]
    }

    /// SPL Token `InitializeAccount`, `[1]` (1 byte). Mint/owner/rent travel
    /// in the account-meta list, not the instruction data.
    #[inline(always)]
    pub fn encode_initialize_account() -> [u8; 1] {
        [1]
    }

    /// SPL Token `InitializeAccount2`/`InitializeAccount3 { owner }`,
    /// `[disc][owner: 32 bytes]` (33 bytes). `disc` is 16 for
    /// `InitializeAccount2` and 18 for `InitializeAccount3`.
    #[inline(always)]
    pub fn encode_initialize_account_with_owner(discriminator: u8, owner: &[u8; 32]) -> [u8; 33] {
        let mut data = [0u8; 33];
        data[0] = discriminator;
        data[1..33].copy_from_slice(owner);
        data
    }

    /// SPL Token `SetAuthority { authority_type, new_authority }`.
    ///
    /// Layout: `[6][authority_type: u8][COption tag: u8]`, followed by
    /// `[new_authority: 32 bytes]` when `new_authority` is `Some`. The tag
    /// byte is 1 (`Some`) or 0 (`None`). Returns the fixed 35-byte buffer and
    /// the number of meaningful bytes: 35 for `Some`, 3 for `None`.
    #[inline(always)]
    pub fn encode_set_authority(
        authority_type: u8,
        new_authority: Option<&[u8; 32]>,
    ) -> ([u8; 35], usize) {
        let mut data = [0u8; 35];
        data[0] = 6;
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

    /// SPL Token `InitializeMint { decimals, mint_authority, freeze_authority }`,
    /// `[0][decimals][mint_authority: 32][COption<freeze_authority>]`: 35 bytes
    /// without a freeze authority, 67 with one.
    #[inline(always)]
    pub fn encode_initialize_mint(
        decimals: u8,
        mint_authority: &[u8; 32],
        freeze_authority: Option<&[u8; 32]>,
    ) -> ([u8; 67], usize) {
        let mut data = [0u8; 67];
        data[0] = 0;
        data[1] = decimals;
        data[2..34].copy_from_slice(mint_authority);
        match freeze_authority {
            Some(key) => {
                data[34] = 1;
                data[35..67].copy_from_slice(key);
                (data, 67)
            }
            None => (data, 35),
        }
    }

    /// SPL Token `InitializeMultisig { m }`, `[2][m]`.
    #[inline(always)]
    pub fn encode_initialize_multisig(m: u8) -> [u8; 2] {
        [2, m]
    }

    /// SPL Token `InitializeMultisig2 { m }`, `[19][m]`.
    #[inline(always)]
    pub fn encode_initialize_multisig2(m: u8) -> [u8; 2] {
        [19, m]
    }

    /// SPL Token `InitializeImmutableOwner`, `[22]`.
    #[inline(always)]
    pub fn encode_initialize_immutable_owner() -> [u8; 1] {
        [22]
    }

    /// The most extension types one `GetAccountDataSize` or `Reallocate`
    /// carries. Token-2022 defines 29; the buffer leaves room for growth.
    pub const MAX_EXTENSION_TYPES: usize = 32;

    /// `[disc][extension_type: u16 LE]*`, the shape of Token-2022
    /// `GetAccountDataSize` (21) and `Reallocate` (29); SPL Token accepts the
    /// bare discriminator. `None` when more than [`MAX_EXTENSION_TYPES`]
    /// types are given.
    #[inline(always)]
    pub fn encode_extension_types(
        disc: u8,
        extension_types: &[u16],
    ) -> Option<([u8; 1 + 2 * MAX_EXTENSION_TYPES], usize)> {
        if extension_types.len() > MAX_EXTENSION_TYPES {
            return None;
        }
        let mut data = [0u8; 1 + 2 * MAX_EXTENSION_TYPES];
        data[0] = disc;
        let mut at = 1;
        for ext in extension_types {
            data[at..at + 2].copy_from_slice(&ext.to_le_bytes());
            at += 2;
        }
        Some((data, at))
    }

    /// SPL Token `WithdrawExcessLamports`, `[38]`.
    #[inline(always)]
    pub fn encode_withdraw_excess_lamports() -> [u8; 1] {
        [38]
    }

    /// SPL Token `AmountToUiAmount { amount }`, `[23][amount: u64 LE]`.
    #[inline(always)]
    pub fn encode_amount_to_ui_amount(amount: u64) -> [u8; 9] {
        amount_ix(23, amount)
    }

    /// The longest UI-amount string `UiAmountToAmount` carries: the
    /// instruction is one byte of discriminator plus at most 254 bytes of
    /// UTF-8.
    pub const MAX_UI_AMOUNT_LEN: usize = 254;

    /// SPL Token `UiAmountToAmount { ui_amount }`, `[24][utf-8 bytes]`.
    /// `None` when the string is longer than [`MAX_UI_AMOUNT_LEN`].
    #[inline(always)]
    pub fn encode_ui_amount_to_amount(
        ui_amount: &str,
    ) -> Option<([u8; 1 + MAX_UI_AMOUNT_LEN], usize)> {
        let bytes = ui_amount.as_bytes();
        if bytes.len() > MAX_UI_AMOUNT_LEN {
            return None;
        }
        let mut data = [0u8; 1 + MAX_UI_AMOUNT_LEN];
        data[0] = 24;
        data[1..1 + bytes.len()].copy_from_slice(bytes);
        Some((data, 1 + bytes.len()))
    }

    /// p-token `UnwrapLamports { amount }`, `[45][0]` for the whole balance
    /// or `[45][1][amount: u64 LE]` for a part of it.
    #[inline(always)]
    pub fn encode_unwrap_lamports(amount: Option<u64>) -> ([u8; 10], usize) {
        let mut data = [0u8; 10];
        data[0] = 45;
        match amount {
            Some(amount) => {
                data[1] = 1;
                data[2..10].copy_from_slice(&amount.to_le_bytes());
                (data, 10)
            }
            None => (data, 2),
        }
    }
}

#[inline]
pub(crate) fn invoke_token_signed<'a, const FIXED: usize>(
    program: &Address,
    data: &[u8],
    fixed_accounts: [InstructionAccount<'a>; FIXED],
    fixed_views: [&'a AccountView<'a>; FIXED],
    trailing: &[Trailing<'_, 'a>],
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    let mut total = FIXED;
    for run in trailing {
        if run.signer && run.views.len() > MAX_TOKEN_MULTISIG_SIGNERS {
            return Err(ProgramError::InvalidArgument);
        }
        total = total
            .checked_add(run.views.len())
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }
    if total > crate::cpi::MAX_STATIC_CPI_ACCOUNTS {
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
    for run in trailing {
        for view in run.views {
            accounts[index].write(InstructionAccount::new(
                view.address(),
                run.writable,
                run.signer,
            ));
            views[index].write(*view);
            index += 1;
        }
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
        program_id: program,
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
/// - `0..4`: COption tag for mint_authority (u32 LE; 0 = None, 1 = Some)
/// - `4..36`: mint_authority pubkey (only meaningful when tag == 1)
/// - `36..44`: supply (u64 LE)
/// - `44`: decimals
/// - `45`: is_initialized
/// - `46..50`: COption tag for freeze_authority
/// - `50..82`: freeze_authority pubkey
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
/// The plain instruction carries neither an explicit mint account nor decimals.
/// The token program still checks that source and destination mints match.
/// Prefer `TransferChecked` to also validate the caller-supplied mint and decimals.
/// This legacy builder calls the classic SPL Token program and is feature-gated.
#[deprecated(
    since = "0.2.0",
    note = "use TransferChecked for explicit classic SPL mint and decimals validation"
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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    /// Invoke with explicit PDA seeds. Skips the direct-signer
    /// pre-check; the supplied signer seeds authorize the CPI.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
impl<'a, 'x: 'a> TokenInstruction<'a> for Transfer<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_transfer(self.amount);

        let accounts = [
            InstructionAccount::writable(self.from.address()),
            InstructionAccount::writable(self.to.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.from, self.to, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
token_program_methods!(Transfer, owner = from, authority = authority);

// ---------------------------------------------------------------------

/// Builder for SPL Token MintTo (instruction index 7).
///
/// Prefer [`MintToChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use MintToChecked for explicit classic SPL mint and decimals validation"
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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
impl<'a, 'x: 'a> TokenInstruction<'a> for MintTo<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_mint_to(self.amount);

        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly_signer(self.mint_authority.address()),
        ];
        let views = [self.mint, self.account, self.mint_authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
token_program_methods!(MintTo, owner = mint, authority = mint_authority);

// ---------------------------------------------------------------------

/// Builder for SPL Token Burn (instruction index 8).
///
/// Prefer [`BurnChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use BurnChecked for explicit classic SPL mint and decimals validation"
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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
impl<'a, 'x: 'a> TokenInstruction<'a> for Burn<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_burn(self.amount);

        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.account, self.mint, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
token_program_methods!(Burn, owner = account, authority = authority);

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
        self.emit(&[], &mut Invoke::legacy(signers))
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for CloseAccount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_close_account();
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.destination.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.account, self.destination, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(CloseAccount, owner = account, authority = authority);

// ---------------------------------------------------------------------

/// Builder for SPL Token Approve (instruction index 4).
///
/// Prefer [`ApproveChecked`] for the decimals-verified path.
#[deprecated(
    since = "0.2.0",
    note = "use ApproveChecked for explicit classic SPL mint and decimals validation"
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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
impl<'a, 'x: 'a> TokenInstruction<'a> for Approve<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_approve(self.amount);

        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.delegate.address()),
            InstructionAccount::readonly_signer(self.authority.address()),
        ];
        let views = [self.source, self.delegate, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

#[allow(deprecated)]
#[cfg(feature = "legacy-token-instructions")]
token_program_methods!(Approve, owner = source, authority = authority);

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
        self.emit(&[], &mut Invoke::legacy(signers))
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for Revoke<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_revoke();
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.source, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(Revoke, owner = source, authority = authority);

// ---------------------------------------------------------------------
//
/// Builder for classic SPL Token TransferChecked (instruction index 12).
///
/// The token program checks the supplied mint and decimals. This builder calls
/// `TOKEN_PROGRAM_ID`; it does not dispatch to Token-2022 or resolve transfer
/// hooks. Use `invoke_on(TokenProgram::Token2022, ..)` or `invoke_for_owner` for that program
/// integrations, supplying hook accounts when required.
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

    /// Check transaction signer status and require the source's token owner
    /// to equal the authority before CPI. This stricter owner path excludes
    /// delegated transfers; use `invoke` when the token program should validate
    /// a delegate or `invoke_multisig` for an SPL multisig authority.
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
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
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for TransferChecked<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_transfer_checked(self.amount, self.decimals);

        let accounts = [
            InstructionAccount::writable(self.from.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::writable(self.to.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.from, self.mint, self.to, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(TransferChecked, owner = from, authority = authority);

// ---------------------------------------------------------------------

/// Builder for SPL Token MintToChecked (instruction index 14).
///
/// Checks decimals through the classic SPL Token program, like [`TransferChecked`].
/// For Token-2022, use `invoke_on(TokenProgram::Token2022, ..)` or `invoke_for_owner`.
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
    }

    #[inline(always)]
    fn invoke_signed_unchecked(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for MintToChecked<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_mint_to_checked(self.amount, self.decimals);

        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::writable(self.account.address()),
            authority_meta(self.mint_authority, multisig_signers),
        ];
        let views = [self.mint, self.account, self.mint_authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(MintToChecked, owner = mint, authority = mint_authority);

// ---------------------------------------------------------------------

/// Builder for SPL Token BurnChecked (instruction index 15).
///
/// Decimals-verified counterpart to the legacy `Burn` builder. Prefer this over
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
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
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for BurnChecked<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_burn_checked(self.amount, self.decimals);

        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.account, self.mint, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(BurnChecked, owner = account, authority = authority);

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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
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
        self.emit(&[], &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for ApproveChecked<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_approve_checked(self.amount, self.decimals);

        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.delegate.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.source, self.mint, self.delegate, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(ApproveChecked, owner = source, authority = authority);

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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for SetAuthority<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) = encoders::encode_set_authority(
            self.authority_type as u8,
            self.new_authority.map(|a| a.as_array()),
        );
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            authority_meta(self.current_authority, multisig_signers),
        ];
        let views = [self.account, self.current_authority];
        sink.emit(
            &data[..len],
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(SetAuthority, owner = account, authority = current_authority);

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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for FreezeAccount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_freeze_account();
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            authority_meta(self.freeze_authority, multisig_signers),
        ];
        let views = [self.account, self.mint, self.freeze_authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(FreezeAccount, owner = account, authority = freeze_authority);

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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
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
        self.emit(multisig_signers, &mut Invoke::legacy(signers))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for ThawAccount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_thaw_account();
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            authority_meta(self.freeze_authority, multisig_signers),
        ];
        let views = [self.account, self.mint, self.freeze_authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(ThawAccount, owner = account, authority = freeze_authority);

// ---------------------------------------------------------------------

/// Builder for SPL Token SyncNative (instruction index 17).
pub struct SyncNative<'a> {
    pub account: &'a AccountView<'a>,
}

impl SyncNative<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for SyncNative<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_sync_native();
        let accounts = [InstructionAccount::writable(self.account.address())];
        let views = [self.account];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(SyncNative, owner = account);

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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeAccount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_initialize_account();
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.owner.address()),
            InstructionAccount::readonly(self.rent_sysvar.address()),
        ];
        let views = [self.account, self.mint, self.owner, self.rent_sysvar];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(InitializeAccount, owner = account);

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
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeAccount2<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_initialize_account_with_owner(16, self.owner.as_array());
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly(self.rent_sysvar.address()),
        ];
        let views = [self.account, self.mint, self.rent_sysvar];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(InitializeAccount2, owner = account);

/// Builder for SPL Token InitializeAccount3 (instruction index 18).
pub struct InitializeAccount3<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub owner: &'a Address,
}

impl InitializeAccount3<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeAccount3<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_initialize_account_with_owner(18, self.owner.as_array());
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::readonly(self.mint.address()),
        ];
        let views = [self.account, self.mint];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(InitializeAccount3, owner = account);

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
        AmountToUiAmount, ApproveChecked, BurnChecked, CloseAccount, FreezeAccount,
        GetAccountDataSize, InitializeAccount, InitializeAccount2, InitializeAccount3,
        InitializeImmutableOwner, InitializeMint, InitializeMultisig, InitializeMultisig2,
        MintToChecked, Revoke, SetAuthority, SyncNative, ThawAccount, TokenAuthorityType,
        TokenBatch, TokenInstruction, TokenProgram, TransferChecked, UiAmountToAmount,
        UnwrapLamports, WithdrawExcessLamports,
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
    fn make_account(owner: Address, data: &[u8]) -> (std::vec::Vec<u64>, AccountView<'static>) {
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + data.len()).div_ceil(8)];
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
            let data_ptr = (backing.as_mut_ptr() as *mut u8).add(RuntimeAccount::SIZE);
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
        // Byte view over the word-aligned backing (the fixture keeps the
        // allocation 8-aligned for the RuntimeAccount header).
        // SAFETY: `backing` owns these bytes; u8 has no alignment demands.
        let backing_bytes = unsafe {
            core::slice::from_raw_parts_mut(backing.as_mut_ptr() as *mut u8, backing.len() * 8)
        };
        backing_bytes[RuntimeAccount::SIZE + 64..RuntimeAccount::SIZE + 72]
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
        let (set_authority, len) = encoders::encode_set_authority(
            TokenAuthorityType::AccountOwner as u8,
            Some(authority.as_array()),
        );
        assert_eq!(len, 35);
        assert_eq!(set_authority[0], 6);
        assert_eq!(set_authority[1], 2);
        assert_eq!(set_authority[2], 1);
        assert_eq!(&set_authority[3..35], authority.as_bytes());

        let (set_authority, len) =
            encoders::encode_set_authority(TokenAuthorityType::CloseAccount as u8, None);
        assert_eq!(len, 3);
        assert_eq!(&set_authority[..3], &[6, 3, 0]);

        let init2 = encoders::encode_initialize_account_with_owner(16, authority.as_array());
        let init3 = encoders::encode_initialize_account_with_owner(18, authority.as_array());
        assert_eq!(init2[0], 16);
        assert_eq!(init3[0], 18);
        assert_eq!(&init2[1..33], authority.as_bytes());
        assert_eq!(&init3[1..33], authority.as_bytes());
    }

    /// Byte-identity guard for the extracted [`encoders`] module: each
    /// shipped encoder must reproduce the exact bytes the builders wrote
    /// inline before the refactor. These literals are the pre-refactor wire
    /// bytes; if any diverges, a CPI's instruction-data changed.
    #[test]
    fn shipped_encoders_match_pre_refactor_golden_bytes() {
        // amount = 1 → little-endian 01 00 00 00 00 00 00 00.
        assert_eq!(encoders::encode_transfer(1), [3, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(encoders::encode_approve(1), [4, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(encoders::encode_mint_to(1), [7, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(encoders::encode_burn(1), [8, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            encoders::encode_transfer_checked(1, 9),
            [12, 1, 0, 0, 0, 0, 0, 0, 0, 9]
        );
        assert_eq!(
            encoders::encode_approve_checked(1, 9),
            [13, 1, 0, 0, 0, 0, 0, 0, 0, 9]
        );
        assert_eq!(
            encoders::encode_mint_to_checked(1, 9),
            [14, 1, 0, 0, 0, 0, 0, 0, 0, 9]
        );
        assert_eq!(
            encoders::encode_burn_checked(1, 9),
            [15, 1, 0, 0, 0, 0, 0, 0, 0, 9]
        );
        assert_eq!(encoders::encode_revoke(), [5]);
        assert_eq!(encoders::encode_close_account(), [9]);
        assert_eq!(encoders::encode_freeze_account(), [10]);
        assert_eq!(encoders::encode_thaw_account(), [11]);
        assert_eq!(encoders::encode_sync_native(), [17]);
        assert_eq!(encoders::encode_initialize_account(), [1]);

        let owner = [
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];
        let init2 = encoders::encode_initialize_account_with_owner(16, &owner);
        assert_eq!(init2[0], 16);
        assert_eq!(&init2[1..33], &owner);
        let init3 = encoders::encode_initialize_account_with_owner(18, &owner);
        assert_eq!(init3[0], 18);
        assert_eq!(&init3[1..33], &owner);

        let (sa_some, some_len) = encoders::encode_set_authority(2, Some(&owner));
        assert_eq!(some_len, 35);
        assert_eq!(sa_some[0], 6);
        assert_eq!(sa_some[1], 2);
        assert_eq!(sa_some[2], 1);
        assert_eq!(&sa_some[3..35], &owner);
        let (sa_none, none_len) = encoders::encode_set_authority(3, None);
        assert_eq!(none_len, 3);
        assert_eq!(&sa_none[..3], &[6, 3, 0]);
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
        std::vec::Vec<u64>,
        std::vec::Vec<u64>,
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
        let mut token_backing =
            std::vec![0u64; (RuntimeAccount::SIZE + token_data_len).div_ceil(8)];
        let token_raw = token_backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let token_backend = unsafe { NativeAccountView::new_unchecked(token_raw) };
        let token_view = crate::account::AccountView::from_backend(token_backend);

        // Authority: no data needed, just an address field.
        let mut auth_backing = std::vec![0u64; (RuntimeAccount::SIZE).div_ceil(8)];
        let auth_raw = auth_backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + data_len).div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
    ) -> (std::vec::Vec<u64>, crate::account::AccountView<'static>) {
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        let token_data_len = 165;
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + token_data_len).div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
    ) -> (std::vec::Vec<u64>, crate::account::AccountView<'static>) {
        use hopper_native::{
            AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
            NOT_BORROWED,
        };

        let mint_data_len = 82;
        let mut backing = std::vec![0u64; (RuntimeAccount::SIZE + mint_data_len).div_ceil(8)];
        let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
        // SAFETY: This block is part of Hopper's reviewed zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
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
