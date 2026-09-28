//! Token-2022 instruction builders that have no SPL Token counterpart:
//! the native-mint and non-transferable-mint initializers, `Reallocate`,
//! and every extension family's instructions (transfer fee, default
//! account state, memo transfer, interest-bearing, CPI guard, permanent
//! delegate, transfer hook, metadata / group / group-member pointers,
//! scaled UI amount, pausable, permissioned burn, mint close authority).
//!
//! Each builder encodes its bytes once in a [`TokenInstruction::emit`]
//! impl, exactly as the shared builders in [`crate::token`] do, so a
//! [`crate::token::TokenBatch`] can carry them too. `invoke()` always
//! targets Token-2022. Builders that initialize a mint extension run
//! before `InitializeMint2`, which is what [`crate::token_mint::MintPlan`]
//! sequences for the fixed-size extensions; the builders here are the
//! same bytes for callers that drive the sequence themselves or that need
//! the post-initialization updates.
//!
//! The wire formats follow the Token-2022 program's instruction enum: a
//! family discriminator (25 for mint close authority, 26 transfer fee, 28
//! default account state, 30 memo transfer, 33 interest bearing, 34 CPI
//! guard, 35 permanent delegate, 36 transfer hook, 39 metadata pointer,
//! 40 group pointer, 41 group member pointer, 43 scaled UI amount, 44
//! pausable, 46 permissioned burn) followed, where the family has more
//! than one instruction, by a sub-discriminator. Optional addresses are
//! either `COption` (`[0]` or `[1][32 bytes]`) or nullable (32 zero bytes
//! mean "none"); each encoder's documentation says which.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionAccount, Signer};
use crate::token::{
    authority_meta, encoders as token_encoders, require_authority_signed_direct,
    require_multisig_signers_direct, Invoke, TokenInstruction, TokenSink, Trailing,
};
use crate::ProgramResult;

/// A bounded, stack-resident instruction-data buffer.
#[derive(Clone, Copy)]
pub struct Bytes<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Bytes<N> {
    #[inline(always)]
    const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    #[inline(always)]
    fn push(&mut self, bytes: &[u8]) {
        self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    /// `[1][address]` when present, `[0]` when absent.
    #[inline(always)]
    fn coption(&mut self, address: Option<&Address>) {
        match address {
            Some(address) => {
                self.push(&[1]);
                self.push(address.as_array());
            }
            None => self.push(&[0]),
        }
    }

    /// The address, or 32 zero bytes when absent.
    #[inline(always)]
    fn nullable(&mut self, address: Option<&Address>) {
        match address {
            Some(address) => self.push(address.as_array()),
            None => self.push(&[0; 32]),
        }
    }

    /// The encoded bytes.
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// Refuse a "present" optional address that is all zeros: Token-2022 stores
/// these as nullable addresses, so a zero address would silently mean
/// "none" on chain.
#[inline(always)]
fn non_zero(address: Option<&Address>) -> ProgramResult {
    if address.is_some_and(|a| a.as_array() == &[0; 32]) {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Byte-exact encoders for the Token-2022-only instructions.
///
/// Public for the same reason as [`crate::token::encoders`]: the
/// builders call them, and so do the layout proofs and
/// [`crate::token_mint::MintExtension`], so there is one source of the
/// bytes.
pub mod encoders {
    use super::{non_zero, Bytes};
    use crate::address::Address;
    use crate::error::ProgramError;
    use crate::ProgramResult;

    pub const IX_INITIALIZE_MINT_CLOSE_AUTHORITY: u8 = 25;
    pub const IX_TRANSFER_FEE: u8 = 26;
    pub const IX_DEFAULT_ACCOUNT_STATE: u8 = 28;
    pub const IX_REALLOCATE: u8 = 29;
    pub const IX_MEMO_TRANSFER: u8 = 30;
    pub const IX_CREATE_NATIVE_MINT: u8 = 31;
    pub const IX_INITIALIZE_NON_TRANSFERABLE_MINT: u8 = 32;
    pub const IX_INTEREST_BEARING_MINT: u8 = 33;
    pub const IX_CPI_GUARD: u8 = 34;
    pub const IX_INITIALIZE_PERMANENT_DELEGATE: u8 = 35;
    pub const IX_TRANSFER_HOOK: u8 = 36;
    pub const IX_METADATA_POINTER: u8 = 39;
    pub const IX_GROUP_POINTER: u8 = 40;
    pub const IX_GROUP_MEMBER_POINTER: u8 = 41;
    pub const IX_SCALED_UI_AMOUNT: u8 = 43;
    pub const IX_PAUSABLE: u8 = 44;
    pub const IX_PERMISSIONED_BURN: u8 = 46;

    /// Token-2022 `AccountState::Initialized`.
    pub const ACCOUNT_STATE_INITIALIZED: u8 = 1;
    /// Token-2022 `AccountState::Frozen`.
    pub const ACCOUNT_STATE_FROZEN: u8 = 2;

    /// The largest transfer fee, 100% in basis points.
    pub const MAX_FEE_BASIS_POINTS: u16 = 10_000;

    /// `[31]`.
    #[inline(always)]
    pub fn encode_create_native_mint() -> [u8; 1] {
        [IX_CREATE_NATIVE_MINT]
    }

    /// `[32]`.
    #[inline(always)]
    pub fn encode_initialize_non_transferable_mint() -> [u8; 1] {
        [IX_INITIALIZE_NON_TRANSFERABLE_MINT]
    }

    /// `[25][COption<close_authority>]`.
    #[inline(always)]
    pub fn encode_initialize_mint_close_authority(close_authority: Option<&Address>) -> Bytes<34> {
        let mut out = Bytes::new();
        out.push(&[IX_INITIALIZE_MINT_CLOSE_AUTHORITY]);
        out.coption(close_authority);
        out
    }

    /// `[26][0][COption<config_authority>][COption<withdraw_authority>][basis_points: u16 LE][maximum_fee: u64 LE]`.
    /// Refuses a fee above 100%.
    #[inline(always)]
    pub fn encode_initialize_transfer_fee_config(
        transfer_fee_config_authority: Option<&Address>,
        withdraw_withheld_authority: Option<&Address>,
        transfer_fee_basis_points: u16,
        maximum_fee: u64,
    ) -> Result<Bytes<78>, ProgramError> {
        if transfer_fee_basis_points > MAX_FEE_BASIS_POINTS {
            return Err(ProgramError::InvalidArgument);
        }
        let mut out = Bytes::new();
        out.push(&[IX_TRANSFER_FEE, 0]);
        out.coption(transfer_fee_config_authority);
        out.coption(withdraw_withheld_authority);
        out.push(&transfer_fee_basis_points.to_le_bytes());
        out.push(&maximum_fee.to_le_bytes());
        Ok(out)
    }

    /// `[26][1][amount: u64 LE][decimals][fee: u64 LE]`.
    #[inline(always)]
    pub fn encode_transfer_checked_with_fee(amount: u64, decimals: u8, fee: u64) -> [u8; 19] {
        let mut data = [0u8; 19];
        data[0] = IX_TRANSFER_FEE;
        data[1] = 1;
        data[2..10].copy_from_slice(&amount.to_le_bytes());
        data[10] = decimals;
        data[11..19].copy_from_slice(&fee.to_le_bytes());
        data
    }

    /// `[26][2]`.
    #[inline(always)]
    pub fn encode_withdraw_withheld_tokens_from_mint() -> [u8; 2] {
        [IX_TRANSFER_FEE, 2]
    }

    /// `[26][3][num_token_accounts]`.
    #[inline(always)]
    pub fn encode_withdraw_withheld_tokens_from_accounts(num_token_accounts: u8) -> [u8; 3] {
        [IX_TRANSFER_FEE, 3, num_token_accounts]
    }

    /// `[26][4]`.
    #[inline(always)]
    pub fn encode_harvest_withheld_tokens_to_mint() -> [u8; 2] {
        [IX_TRANSFER_FEE, 4]
    }

    /// `[26][5][basis_points: u16 LE][maximum_fee: u64 LE]`. Refuses a fee
    /// above 100%.
    #[inline(always)]
    pub fn encode_set_transfer_fee(
        transfer_fee_basis_points: u16,
        maximum_fee: u64,
    ) -> Result<[u8; 12], ProgramError> {
        if transfer_fee_basis_points > MAX_FEE_BASIS_POINTS {
            return Err(ProgramError::InvalidArgument);
        }
        let mut data = [0u8; 12];
        data[0] = IX_TRANSFER_FEE;
        data[1] = 5;
        data[2..4].copy_from_slice(&transfer_fee_basis_points.to_le_bytes());
        data[4..12].copy_from_slice(&maximum_fee.to_le_bytes());
        Ok(data)
    }

    #[inline(always)]
    fn check_account_state(state: u8) -> ProgramResult {
        if state == ACCOUNT_STATE_INITIALIZED || state == ACCOUNT_STATE_FROZEN {
            Ok(())
        } else {
            Err(ProgramError::InvalidArgument)
        }
    }

    /// `[28][0][state]`; `state` is 1 (initialized) or 2 (frozen).
    #[inline(always)]
    pub fn encode_initialize_default_account_state(state: u8) -> Result<[u8; 3], ProgramError> {
        check_account_state(state)?;
        Ok([IX_DEFAULT_ACCOUNT_STATE, 0, state])
    }

    /// `[28][1][state]`; `state` is 1 (initialized) or 2 (frozen).
    #[inline(always)]
    pub fn encode_update_default_account_state(state: u8) -> Result<[u8; 3], ProgramError> {
        check_account_state(state)?;
        Ok([IX_DEFAULT_ACCOUNT_STATE, 1, state])
    }

    /// `[30][0]` (enable) or `[30][1]` (disable).
    #[inline(always)]
    pub fn encode_memo_transfer(enable: bool) -> [u8; 2] {
        [IX_MEMO_TRANSFER, u8::from(!enable)]
    }

    /// `[33][0][rate_authority: nullable 32][rate: i16 LE]`.
    #[inline(always)]
    pub fn encode_initialize_interest_bearing_mint(
        rate_authority: Option<&Address>,
        rate: i16,
    ) -> Result<Bytes<36>, ProgramError> {
        non_zero(rate_authority)?;
        let mut out = Bytes::new();
        out.push(&[IX_INTEREST_BEARING_MINT, 0]);
        out.nullable(rate_authority);
        out.push(&rate.to_le_bytes());
        Ok(out)
    }

    /// `[33][1][rate: i16 LE]`.
    #[inline(always)]
    pub fn encode_update_interest_rate(rate: i16) -> [u8; 4] {
        let rate = rate.to_le_bytes();
        [IX_INTEREST_BEARING_MINT, 1, rate[0], rate[1]]
    }

    /// `[34][0]` (enable) or `[34][1]` (disable).
    #[inline(always)]
    pub fn encode_cpi_guard(enable: bool) -> [u8; 2] {
        [IX_CPI_GUARD, u8::from(!enable)]
    }

    /// `[35][delegate: 32]`.
    #[inline(always)]
    pub fn encode_initialize_permanent_delegate(delegate: &Address) -> Bytes<33> {
        let mut out = Bytes::new();
        out.push(&[IX_INITIALIZE_PERMANENT_DELEGATE]);
        out.push(delegate.as_array());
        out
    }

    /// `[family][0][authority: nullable 32][target: nullable 32]`, the
    /// shape shared by the transfer hook (36), metadata pointer (39),
    /// group pointer (40), and group member pointer (41) initializers.
    #[inline(always)]
    pub fn encode_initialize_pointer(
        family: u8,
        authority: Option<&Address>,
        target: Option<&Address>,
    ) -> Result<Bytes<66>, ProgramError> {
        non_zero(authority)?;
        non_zero(target)?;
        let mut out = Bytes::new();
        out.push(&[family, 0]);
        out.nullable(authority);
        out.nullable(target);
        Ok(out)
    }

    /// `[family][1][target: nullable 32]`, the shape shared by the transfer
    /// hook, metadata pointer, group pointer, and group member pointer
    /// updates.
    #[inline(always)]
    pub fn encode_update_pointer(
        family: u8,
        target: Option<&Address>,
    ) -> Result<Bytes<34>, ProgramError> {
        non_zero(target)?;
        let mut out = Bytes::new();
        out.push(&[family, 1]);
        out.nullable(target);
        Ok(out)
    }

    /// Whether an `f64` multiplier is positive and finite, decided on its
    /// bit pattern so that no soft-float comparison is linked on chain.
    #[inline(always)]
    pub const fn multiplier_is_valid(multiplier: f64) -> bool {
        let bits = multiplier.to_bits();
        let exponent = (bits >> 52) & 0x7ff;
        bits >> 63 == 0 && exponent != 0x7ff && bits != 0
    }

    /// `[43][0][authority: nullable 32][multiplier: f64 LE]`. The multiplier
    /// must be positive and finite.
    #[inline(always)]
    pub fn encode_initialize_scaled_ui_amount(
        authority: Option<&Address>,
        multiplier: f64,
    ) -> Result<Bytes<42>, ProgramError> {
        non_zero(authority)?;
        if !multiplier_is_valid(multiplier) {
            return Err(ProgramError::InvalidArgument);
        }
        let mut out = Bytes::new();
        out.push(&[IX_SCALED_UI_AMOUNT, 0]);
        out.nullable(authority);
        out.push(&multiplier.to_le_bytes());
        Ok(out)
    }

    /// `[43][1][multiplier: f64 LE][effective_timestamp: i64 LE]`.
    #[inline(always)]
    pub fn encode_update_scaled_ui_amount_multiplier(
        multiplier: f64,
        effective_timestamp: i64,
    ) -> Result<[u8; 18], ProgramError> {
        if !multiplier_is_valid(multiplier) {
            return Err(ProgramError::InvalidArgument);
        }
        let mut data = [0u8; 18];
        data[0] = IX_SCALED_UI_AMOUNT;
        data[1] = 1;
        data[2..10].copy_from_slice(&multiplier.to_le_bytes());
        data[10..18].copy_from_slice(&effective_timestamp.to_le_bytes());
        Ok(data)
    }

    /// `[family][0][authority: 32]`, the shape shared by the pausable (44)
    /// and permissioned burn (46) initializers. The authority must not be
    /// the zero address.
    #[inline(always)]
    pub fn encode_initialize_authority(
        family: u8,
        authority: &Address,
    ) -> Result<Bytes<34>, ProgramError> {
        non_zero(Some(authority))?;
        let mut out = Bytes::new();
        out.push(&[family, 0]);
        out.push(authority.as_array());
        Ok(out)
    }

    /// `[44][1]` (pause) or `[44][2]` (resume).
    #[inline(always)]
    pub fn encode_pausable(pause: bool) -> [u8; 2] {
        [IX_PAUSABLE, if pause { 1 } else { 2 }]
    }

    /// `[46][1][amount: u64 LE]`.
    #[inline(always)]
    pub fn encode_permissioned_burn(amount: u64) -> [u8; 10] {
        let mut data = [0u8; 10];
        data[0] = IX_PERMISSIONED_BURN;
        data[1] = 1;
        data[2..10].copy_from_slice(&amount.to_le_bytes());
        data
    }

    /// `[46][2][amount: u64 LE][decimals]`.
    #[inline(always)]
    pub fn encode_permissioned_burn_checked(amount: u64, decimals: u8) -> [u8; 11] {
        let mut data = [0u8; 11];
        data[0] = IX_PERMISSIONED_BURN;
        data[1] = 2;
        data[2..10].copy_from_slice(&amount.to_le_bytes());
        data[10] = decimals;
        data
    }
}

use encoders::*;

/// The Token-2022 entry points: `invoke` (authority signed directly),
/// `invoke_signed` (PDA seeds), and for builders with an authority the
/// multisig forms.
macro_rules! t22_methods {
    ($name:ident) => {
        impl $name<'_> {
            /// Send this instruction to Token-2022.
            #[inline]
            pub fn invoke(&self) -> ProgramResult {
                self.emit(&[], &mut Invoke::token_2022(&[]))
            }

            /// Send this instruction to Token-2022 with PDA signers.
            #[inline]
            pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
                self.emit(&[], &mut Invoke::token_2022(signers))
            }
        }
    };
    ($name:ident, authority = $auth:ident) => {
        impl $name<'_> {
            /// Send this instruction to Token-2022 with the authority
            /// signed directly. Fails with `MissingRequiredSignature`
            /// before the CPI if it is not.
            #[inline]
            pub fn invoke(&self) -> ProgramResult {
                require_authority_signed_direct(self.$auth)?;
                self.emit(&[], &mut Invoke::token_2022(&[]))
            }

            /// Send this instruction to Token-2022 with PDA signers.
            #[inline]
            pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
                self.emit(&[], &mut Invoke::token_2022(signers))
            }

            /// Send this instruction to Token-2022 with a multisig
            /// authority whose signers signed directly.
            #[inline]
            pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
                require_multisig_signers_direct(multisig_signers)?;
                self.emit(multisig_signers, &mut Invoke::token_2022(&[]))
            }

            /// Send this instruction to Token-2022 with a multisig
            /// authority and PDA signers.
            #[inline]
            pub fn invoke_signed_multisig(
                &self,
                multisig_signers: &[&AccountView<'_>],
                signers: &[Signer<'_, '_>],
            ) -> ProgramResult {
                self.emit(multisig_signers, &mut Invoke::token_2022(signers))
            }
        }
    };
}

/// A builder whose only account is a writable mint and whose data is fixed
/// at construction: the extension initializers.
macro_rules! mint_initializer {
    ($(#[$doc:meta])* $name:ident { $($field:ident : $ty:ty),* $(,)? } data = |$s:ident| $data:expr;) => {
        $(#[$doc])*
        pub struct $name<'a> {
            pub mint: &'a AccountView<'a>,
            $(pub $field: $ty,)*
        }

        impl<'a, 'x: 'a> TokenInstruction<'a> for $name<'x> {
            #[inline(always)]
            fn emit(
                &self,
                multisig_signers: &[&'a AccountView<'a>],
                sink: &mut impl TokenSink<'a>,
            ) -> ProgramResult {
                let $s = self;
                let data = $data;
                let accounts = [InstructionAccount::writable(self.mint.address())];
                let views = [self.mint];
                sink.emit(data.as_ref(), accounts, views, &[Trailing::signers(multisig_signers)])
            }
        }

        t22_methods!($name);
    };
}

/// A builder over a writable target account and an authority (single key
/// or multisig) with fixed data: the extension updates and toggles.
macro_rules! authority_update {
    ($(#[$doc:meta])* $name:ident { target = $target:ident, authority = $auth:ident $(, $field:ident : $ty:ty)* $(,)? } data = |$s:ident| $data:expr;) => {
        $(#[$doc])*
        pub struct $name<'a> {
            pub $target: &'a AccountView<'a>,
            pub $auth: &'a AccountView<'a>,
            $(pub $field: $ty,)*
        }

        impl<'a, 'x: 'a> TokenInstruction<'a> for $name<'x> {
            #[inline(always)]
            fn emit(
                &self,
                multisig_signers: &[&'a AccountView<'a>],
                sink: &mut impl TokenSink<'a>,
            ) -> ProgramResult {
                let $s = self;
                let data = $data;
                let accounts = [
                    InstructionAccount::writable(self.$target.address()),
                    authority_meta(self.$auth, multisig_signers),
                ];
                let views = [self.$target, self.$auth];
                sink.emit(data.as_ref(), accounts, views, &[Trailing::signers(multisig_signers)])
            }
        }

        t22_methods!($name, authority = $auth);
    };
}

impl<const N: usize> AsRef<[u8]> for Bytes<N> {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

// ---------------------------------------------------------------------
// Program-level instructions

/// `CreateNativeMint` (31): create the Token-2022 wrapped-SOL mint
/// (`9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP`). `payer` funds the
/// rent and signs.
pub struct CreateNativeMint<'a> {
    pub payer: &'a AccountView<'a>,
    pub native_mint: &'a AccountView<'a>,
    pub system_program: &'a AccountView<'a>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for CreateNativeMint<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_create_native_mint();
        let accounts = [
            InstructionAccount::writable_signer(self.payer.address()),
            InstructionAccount::writable(self.native_mint.address()),
            InstructionAccount::readonly(self.system_program.address()),
        ];
        let views = [self.payer, self.native_mint, self.system_program];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

t22_methods!(CreateNativeMint);

mint_initializer! {
    /// `InitializeNonTransferableMint` (32): mark a not-yet-initialized
    /// mint so that its tokens can never be transferred. Runs before
    /// `InitializeMint2`.
    InitializeNonTransferableMint {}
    data = |_s| encode_initialize_non_transferable_mint();
}

/// `Reallocate` (29): grow an initialized token account so that it can
/// hold `extension_types` (Token-2022 TLV type numbers); `payer` funds the
/// rent difference and `owner` authorizes, directly or through multisig
/// signers.
pub struct Reallocate<'a> {
    pub account: &'a AccountView<'a>,
    pub payer: &'a AccountView<'a>,
    pub system_program: &'a AccountView<'a>,
    pub owner: &'a AccountView<'a>,
    pub extension_types: &'a [u16],
}

impl<'a, 'x: 'a> TokenInstruction<'a> for Reallocate<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) =
            token_encoders::encode_extension_types(IX_REALLOCATE, self.extension_types)
                .ok_or(ProgramError::InvalidArgument)?;
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable_signer(self.payer.address()),
            InstructionAccount::readonly(self.system_program.address()),
            authority_meta(self.owner, multisig_signers),
        ];
        let views = [self.account, self.payer, self.system_program, self.owner];
        sink.emit(
            &data[..len],
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

t22_methods!(Reallocate, authority = owner);

// ---------------------------------------------------------------------
// Mint extension initializers (before InitializeMint2)

mint_initializer! {
    /// `InitializeMintCloseAuthority` (25): let `close_authority` close the
    /// mint once its supply is zero.
    InitializeMintCloseAuthority { close_authority: Option<&'a Address> }
    data = |s| encode_initialize_mint_close_authority(s.close_authority);
}

mint_initializer! {
    /// `TransferFeeExtension::InitializeTransferFeeConfig` (26/0): charge
    /// `transfer_fee_basis_points` of every transfer, capped at
    /// `maximum_fee`, withheld in the destination account.
    InitializeTransferFeeConfig {
        transfer_fee_config_authority: Option<&'a Address>,
        withdraw_withheld_authority: Option<&'a Address>,
        transfer_fee_basis_points: u16,
        maximum_fee: u64,
    }
    data = |s| encode_initialize_transfer_fee_config(
        s.transfer_fee_config_authority,
        s.withdraw_withheld_authority,
        s.transfer_fee_basis_points,
        s.maximum_fee,
    )?;
}

mint_initializer! {
    /// `DefaultAccountStateExtension::Initialize` (28/0): new token
    /// accounts of this mint start in `state` (1 initialized, 2 frozen).
    InitializeDefaultAccountState { state: u8 }
    data = |s| encode_initialize_default_account_state(s.state)?;
}

mint_initializer! {
    /// `InterestBearingMintExtension::Initialize` (33/0): display balances
    /// with continuously compounding interest at `rate` basis points.
    InitializeInterestBearingMint { rate_authority: Option<&'a Address>, rate: i16 }
    data = |s| encode_initialize_interest_bearing_mint(s.rate_authority, s.rate)?;
}

mint_initializer! {
    /// `InitializePermanentDelegate` (35): `delegate` may transfer or burn
    /// from any account of this mint, forever.
    InitializePermanentDelegate { delegate: &'a Address }
    data = |s| encode_initialize_permanent_delegate(s.delegate);
}

mint_initializer! {
    /// `TransferHookExtension::Initialize` (36/0): every transfer of this
    /// mint calls `program_id`; `authority` may change it later.
    InitializeTransferHook { authority: Option<&'a Address>, program_id: Option<&'a Address> }
    data = |s| encode_initialize_pointer(IX_TRANSFER_HOOK, s.authority, s.program_id)?;
}

mint_initializer! {
    /// `MetadataPointerExtension::Initialize` (39/0).
    InitializeMetadataPointer { authority: Option<&'a Address>, metadata_address: Option<&'a Address> }
    data = |s| encode_initialize_pointer(IX_METADATA_POINTER, s.authority, s.metadata_address)?;
}

mint_initializer! {
    /// `GroupPointerExtension::Initialize` (40/0).
    InitializeGroupPointer { authority: Option<&'a Address>, group_address: Option<&'a Address> }
    data = |s| encode_initialize_pointer(IX_GROUP_POINTER, s.authority, s.group_address)?;
}

mint_initializer! {
    /// `GroupMemberPointerExtension::Initialize` (41/0).
    InitializeGroupMemberPointer { authority: Option<&'a Address>, member_address: Option<&'a Address> }
    data = |s| encode_initialize_pointer(IX_GROUP_MEMBER_POINTER, s.authority, s.member_address)?;
}

mint_initializer! {
    /// `ScaledUiAmountExtension::Initialize` (43/0): display balances
    /// multiplied by `multiplier` (positive and finite).
    InitializeScaledUiAmount { authority: Option<&'a Address>, multiplier: f64 }
    data = |s| encode_initialize_scaled_ui_amount(s.authority, s.multiplier)?;
}

mint_initializer! {
    /// `PausableExtension::Initialize` (44/0): `authority` may pause and
    /// resume every transfer, mint, and burn of this mint.
    InitializePausable { authority: &'a Address }
    data = |s| encode_initialize_authority(IX_PAUSABLE, s.authority)?;
}

mint_initializer! {
    /// `PermissionedBurnExtension::Initialize` (46/0): burns of this mint
    /// need `authority`'s signature next to the holder's.
    InitializePermissionedBurn { authority: &'a Address }
    data = |s| encode_initialize_authority(IX_PERMISSIONED_BURN, s.authority)?;
}

// ---------------------------------------------------------------------
// Transfer fee operations

/// `TransferFeeExtension::TransferCheckedWithFee` (26/1): a
/// `TransferChecked` that also states the fee the caller expects, which
/// the program refuses to exceed.
pub struct TransferCheckedWithFee<'a> {
    pub source: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: u64,
    pub decimals: u8,
    pub fee: u64,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for TransferCheckedWithFee<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_transfer_checked_with_fee(self.amount, self.decimals, self.fee);
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::writable(self.destination.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.source, self.mint, self.destination, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

t22_methods!(TransferCheckedWithFee, authority = authority);

/// `TransferFeeExtension::WithdrawWithheldTokensFromMint` (26/2): move
/// the fees harvested into the mint to `destination`.
pub struct WithdrawWithheldTokensFromMint<'a> {
    pub mint: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub withdraw_withheld_authority: &'a AccountView<'a>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for WithdrawWithheldTokensFromMint<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_withdraw_withheld_tokens_from_mint();
        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::writable(self.destination.address()),
            authority_meta(self.withdraw_withheld_authority, multisig_signers),
        ];
        let views = [
            self.mint,
            self.destination,
            self.withdraw_withheld_authority,
        ];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

t22_methods!(
    WithdrawWithheldTokensFromMint,
    authority = withdraw_withheld_authority
);

/// `TransferFeeExtension::WithdrawWithheldTokensFromAccounts` (26/3): move
/// the fees withheld in `sources` (writable token accounts of `mint`) to
/// `destination`.
pub struct WithdrawWithheldTokensFromAccounts<'a> {
    pub mint: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub withdraw_withheld_authority: &'a AccountView<'a>,
    pub sources: &'a [&'a AccountView<'a>],
}

impl<'a, 'x: 'a> TokenInstruction<'a> for WithdrawWithheldTokensFromAccounts<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let count = u8::try_from(self.sources.len()).map_err(|_| ProgramError::InvalidArgument)?;
        let data = encode_withdraw_withheld_tokens_from_accounts(count);
        let accounts = [
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::writable(self.destination.address()),
            authority_meta(self.withdraw_withheld_authority, multisig_signers),
        ];
        let views = [
            self.mint,
            self.destination,
            self.withdraw_withheld_authority,
        ];
        sink.emit(
            &data,
            accounts,
            views,
            &[
                Trailing::signers(multisig_signers),
                Trailing::writable(self.sources),
            ],
        )
    }
}

t22_methods!(
    WithdrawWithheldTokensFromAccounts,
    authority = withdraw_withheld_authority
);

/// `TransferFeeExtension::HarvestWithheldTokensToMint` (26/4): move the
/// fees withheld in `sources` into the mint; anyone may call it.
pub struct HarvestWithheldTokensToMint<'a> {
    pub mint: &'a AccountView<'a>,
    pub sources: &'a [&'a AccountView<'a>],
}

impl<'a, 'x: 'a> TokenInstruction<'a> for HarvestWithheldTokensToMint<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_harvest_withheld_tokens_to_mint();
        let accounts = [InstructionAccount::writable(self.mint.address())];
        let views = [self.mint];
        sink.emit(&data, accounts, views, &[Trailing::writable(self.sources)])
    }
}

t22_methods!(HarvestWithheldTokensToMint);

authority_update! {
    /// `TransferFeeExtension::SetTransferFee` (26/5): change the fee; it
    /// takes effect two epochs later.
    SetTransferFee { target = mint, authority = transfer_fee_config_authority, transfer_fee_basis_points: u16, maximum_fee: u64 }
    data = |s| encode_set_transfer_fee(s.transfer_fee_basis_points, s.maximum_fee)?;
}

// ---------------------------------------------------------------------
// Mint and account updates

authority_update! {
    /// `DefaultAccountStateExtension::Update` (28/1): change the state new
    /// accounts start in; signed by the mint's freeze authority.
    UpdateDefaultAccountState { target = mint, authority = freeze_authority, state: u8 }
    data = |s| encode_update_default_account_state(s.state)?;
}

authority_update! {
    /// `MemoTransferExtension::Enable` (30/0): the account refuses incoming
    /// transfers that carry no memo.
    EnableMemoTransfer { target = account, authority = owner }
    data = |_s| encode_memo_transfer(true);
}

authority_update! {
    /// `MemoTransferExtension::Disable` (30/1).
    DisableMemoTransfer { target = account, authority = owner }
    data = |_s| encode_memo_transfer(false);
}

authority_update! {
    /// `InterestBearingMintExtension::UpdateRate` (33/1).
    UpdateInterestRate { target = mint, authority = rate_authority, rate: i16 }
    data = |s| encode_update_interest_rate(s.rate);
}

authority_update! {
    /// `CpiGuardExtension::Enable` (34/0): the account refuses transfers,
    /// approvals, burns, and close from inside a CPI unless a PDA of the
    /// calling program owns it.
    EnableCpiGuard { target = account, authority = owner }
    data = |_s| encode_cpi_guard(true);
}

authority_update! {
    /// `CpiGuardExtension::Disable` (34/1).
    DisableCpiGuard { target = account, authority = owner }
    data = |_s| encode_cpi_guard(false);
}

authority_update! {
    /// `TransferHookExtension::Update` (36/1): point the mint at another
    /// hook program, or at none.
    UpdateTransferHook { target = mint, authority = authority, program_id: Option<&'a Address> }
    data = |s| encode_update_pointer(IX_TRANSFER_HOOK, s.program_id)?;
}

authority_update! {
    /// `MetadataPointerExtension::Update` (39/1).
    UpdateMetadataPointer { target = mint, authority = authority, metadata_address: Option<&'a Address> }
    data = |s| encode_update_pointer(IX_METADATA_POINTER, s.metadata_address)?;
}

authority_update! {
    /// `GroupPointerExtension::Update` (40/1).
    UpdateGroupPointer { target = mint, authority = authority, group_address: Option<&'a Address> }
    data = |s| encode_update_pointer(IX_GROUP_POINTER, s.group_address)?;
}

authority_update! {
    /// `GroupMemberPointerExtension::Update` (41/1).
    UpdateGroupMemberPointer { target = mint, authority = authority, member_address: Option<&'a Address> }
    data = |s| encode_update_pointer(IX_GROUP_MEMBER_POINTER, s.member_address)?;
}

authority_update! {
    /// `ScaledUiAmountExtension::UpdateMultiplier` (43/1): a new
    /// multiplier that takes effect at `effective_timestamp` (0 for now).
    UpdateScaledUiAmountMultiplier { target = mint, authority = authority, multiplier: f64, effective_timestamp: i64 }
    data = |s| encode_update_scaled_ui_amount_multiplier(s.multiplier, s.effective_timestamp)?;
}

authority_update! {
    /// `PausableExtension::Pause` (44/1): stop every transfer, mint, and
    /// burn of the mint.
    Pause { target = mint, authority = authority }
    data = |_s| encode_pausable(true);
}

authority_update! {
    /// `PausableExtension::Resume` (44/2).
    Resume { target = mint, authority = authority }
    data = |_s| encode_pausable(false);
}

// ---------------------------------------------------------------------
// Permissioned burn

/// `PermissionedBurnExtension::Burn` (46/1): burn from `account` with the
/// holder's `authority` and the mint's `permissioned_burn_authority` both
/// signing.
pub struct PermissionedBurn<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub permissioned_burn_authority: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: u64,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for PermissionedBurn<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_permissioned_burn(self.amount);
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::readonly_signer(self.permissioned_burn_authority.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [
            self.account,
            self.mint,
            self.permissioned_burn_authority,
            self.authority,
        ];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

t22_methods!(PermissionedBurn, authority = authority);

/// `PermissionedBurnExtension::BurnChecked` (46/2): [`PermissionedBurn`]
/// with the mint's decimals stated.
pub struct PermissionedBurnChecked<'a> {
    pub account: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub permissioned_burn_authority: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: u64,
    pub decimals: u8,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for PermissionedBurnChecked<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encode_permissioned_burn_checked(self.amount, self.decimals);
        let accounts = [
            InstructionAccount::writable(self.account.address()),
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::readonly_signer(self.permissioned_burn_authority.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [
            self.account,
            self.mint,
            self.permissioned_burn_authority,
            self.authority,
        ];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

t22_methods!(PermissionedBurnChecked, authority = authority);

#[cfg(test)]
mod tests {
    use super::encoders::*;
    use crate::address::Address;

    fn addr(byte: u8) -> Address {
        Address::new_from_array([byte; 32])
    }

    #[test]
    fn fixed_encoders_match_the_token_2022_wire_format() {
        assert_eq!(encode_create_native_mint(), [31]);
        assert_eq!(encode_initialize_non_transferable_mint(), [32]);
        assert_eq!(encode_withdraw_withheld_tokens_from_mint(), [26, 2]);
        assert_eq!(encode_withdraw_withheld_tokens_from_accounts(3), [26, 3, 3]);
        assert_eq!(encode_harvest_withheld_tokens_to_mint(), [26, 4]);
        assert_eq!(encode_memo_transfer(true), [30, 0]);
        assert_eq!(encode_memo_transfer(false), [30, 1]);
        assert_eq!(encode_cpi_guard(true), [34, 0]);
        assert_eq!(encode_cpi_guard(false), [34, 1]);
        assert_eq!(encode_pausable(true), [44, 1]);
        assert_eq!(encode_pausable(false), [44, 2]);
        assert_eq!(encode_update_interest_rate(-2), [33, 1, 0xfe, 0xff]);
        assert_eq!(
            encode_transfer_checked_with_fee(1, 9, 2),
            [26, 1, 1, 0, 0, 0, 0, 0, 0, 0, 9, 2, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            encode_set_transfer_fee(100, 5).unwrap(),
            [26, 5, 100, 0, 5, 0, 0, 0, 0, 0, 0, 0]
        );
        assert!(encode_set_transfer_fee(10_001, 5).is_err());
        assert_eq!(
            encode_initialize_default_account_state(2).unwrap(),
            [28, 0, 2]
        );
        assert_eq!(encode_update_default_account_state(1).unwrap(), [28, 1, 1]);
        assert!(encode_initialize_default_account_state(0).is_err());
        assert!(encode_update_default_account_state(3).is_err());
        assert_eq!(encode_permissioned_burn(1), [46, 1, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            encode_permissioned_burn_checked(1, 6),
            [46, 2, 1, 0, 0, 0, 0, 0, 0, 0, 6]
        );
    }

    #[test]
    fn optional_address_encoders_distinguish_coption_from_nullable() {
        let a = addr(1);
        let b = addr(2);

        let some = encode_initialize_mint_close_authority(Some(&a));
        assert_eq!(some.as_slice().len(), 34);
        assert_eq!(&some.as_slice()[..2], &[25, 1]);
        assert_eq!(&some.as_slice()[2..], a.as_bytes());
        let none = encode_initialize_mint_close_authority(None);
        assert_eq!(none.as_slice(), &[25, 0]);

        let fee = encode_initialize_transfer_fee_config(Some(&a), None, 250, 7).unwrap();
        let bytes = fee.as_slice();
        assert_eq!(bytes.len(), 2 + 33 + 1 + 2 + 8);
        assert_eq!(&bytes[..3], &[26, 0, 1]);
        assert_eq!(&bytes[3..35], a.as_bytes());
        assert_eq!(bytes[35], 0);
        assert_eq!(&bytes[36..38], &250u16.to_le_bytes());
        assert_eq!(&bytes[38..46], &7u64.to_le_bytes());
        assert!(encode_initialize_transfer_fee_config(None, None, 10_001, 0).is_err());

        let hook = encode_initialize_pointer(36, Some(&a), Some(&b)).unwrap();
        assert_eq!(hook.as_slice().len(), 66);
        assert_eq!(&hook.as_slice()[..2], &[36, 0]);
        assert_eq!(&hook.as_slice()[2..34], a.as_bytes());
        assert_eq!(&hook.as_slice()[34..66], b.as_bytes());
        let bare = encode_initialize_pointer(39, None, None).unwrap();
        assert_eq!(&bare.as_slice()[2..], &[0u8; 64]);
        assert!(encode_initialize_pointer(40, Some(&Address::default()), None).is_err());

        let update = encode_update_pointer(41, None).unwrap();
        assert_eq!(update.as_slice().len(), 34);
        assert_eq!(&update.as_slice()[..2], &[41, 1]);

        let interest = encode_initialize_interest_bearing_mint(None, 300).unwrap();
        assert_eq!(interest.as_slice().len(), 36);
        assert_eq!(&interest.as_slice()[34..], &300i16.to_le_bytes());

        let delegate = encode_initialize_permanent_delegate(&a);
        assert_eq!(delegate.as_slice()[0], 35);
        assert_eq!(&delegate.as_slice()[1..], a.as_bytes());

        let pausable = encode_initialize_authority(44, &a).unwrap();
        assert_eq!(&pausable.as_slice()[..2], &[44, 0]);
        assert_eq!(&pausable.as_slice()[2..], a.as_bytes());
        assert!(encode_initialize_authority(46, &Address::default()).is_err());
    }

    #[test]
    fn scaled_ui_amount_multiplier_is_validated_on_bits() {
        assert!(multiplier_is_valid(1.0));
        assert!(multiplier_is_valid(0.5));
        assert!(multiplier_is_valid(f64::MIN_POSITIVE));
        assert!(!multiplier_is_valid(0.0));
        assert!(!multiplier_is_valid(-0.0));
        assert!(!multiplier_is_valid(-1.0));
        assert!(!multiplier_is_valid(f64::INFINITY));
        assert!(!multiplier_is_valid(f64::NAN));

        let init = encode_initialize_scaled_ui_amount(None, 2.0).unwrap();
        assert_eq!(init.as_slice().len(), 42);
        assert_eq!(&init.as_slice()[..2], &[43, 0]);
        assert_eq!(&init.as_slice()[34..], &2.0f64.to_le_bytes());
        assert!(encode_initialize_scaled_ui_amount(None, 0.0).is_err());

        let update = encode_update_scaled_ui_amount_multiplier(3.0, 17).unwrap();
        assert_eq!(&update[..2], &[43, 1]);
        assert_eq!(&update[2..10], &3.0f64.to_le_bytes());
        assert_eq!(&update[10..18], &17i64.to_le_bytes());
    }
}
