//! Administrative SPL Token / Token-2022 builders: mint and multisig
//! initialization, immutable-owner accounts, the data-size and UI-amount
//! queries, and lamport withdrawal from token accounts.
//!
//! Every builder here is re-exported from [`crate::token`] and follows the
//! same contract as the transfer family: one [`TokenInstruction::emit`]
//! encodes the bytes and metas, `invoke()` sends them to SPL Token,
//! `invoke_on` to an explicit [`TokenProgram`], `invoke_for_owner` to the
//! program that owns the first account, and a [`crate::token::TokenBatch`]
//! can collect them.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionAccount, Signer};
use crate::token::{
    authority_meta, encoders, require_authority_signed_direct, require_multisig_signers_direct,
    token_program_methods, Invoke, TokenInstruction, TokenProgram, TokenSink, Trailing,
    MAX_TOKEN_MULTISIG_SIGNERS,
};
use crate::ProgramResult;

pub use crate::token::encoders::{MAX_EXTENSION_TYPES, MAX_UI_AMOUNT_LEN};

// ---------------------------------------------------------------------

/// `InitializeMint` (0): the Rent-sysvar form of
/// [`InitializeMint2`](crate::token_mint::InitializeMint2). The mint must
/// already be allocated (82 bytes on SPL Token) and owned by the token
/// program. Prefer `InitializeMint2` in new code; this form exists for
/// callers whose instruction already carries the Rent sysvar.
pub struct InitializeMint<'a> {
    pub mint: &'a AccountView<'a>,
    pub rent_sysvar: &'a AccountView<'a>,
    pub decimals: u8,
    pub mint_authority: &'a Address,
    pub freeze_authority: Option<&'a Address>,
}

impl InitializeMint<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeMint<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) = encoders::encode_initialize_mint(
            self.decimals,
            self.mint_authority.as_array(),
            self.freeze_authority.map(|a| a.as_array()),
        );
        let accounts = [
            InstructionAccount::writable(self.mint.address()),
            InstructionAccount::readonly(self.rent_sysvar.address()),
        ];
        let views = [self.mint, self.rent_sysvar];
        sink.emit(
            &data[..len],
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(InitializeMint, owner = mint);

// ---------------------------------------------------------------------

/// Refuse a multisig configuration the token program would refuse:
/// `1 <= m <= n <= 11` signers.
#[inline(always)]
fn check_multisig_shape(m: u8, signers: usize) -> ProgramResult {
    if m == 0 || signers == 0 || signers > MAX_TOKEN_MULTISIG_SIGNERS || usize::from(m) > signers {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// `InitializeMultisig` (2): turn a 355-byte, token-program-owned account
/// into an `m`-of-`n` multisig over `signers`. The member accounts are
/// listed read-only and do not sign. Requires the Rent sysvar account;
/// [`InitializeMultisig2`] does not.
pub struct InitializeMultisig<'a> {
    pub multisig: &'a AccountView<'a>,
    pub rent_sysvar: &'a AccountView<'a>,
    /// The member accounts, 1 to 11 of them.
    pub signers: &'a [&'a AccountView<'a>],
    /// How many members must sign, 1 to `signers.len()`.
    pub m: u8,
}

impl InitializeMultisig<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeMultisig<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        check_multisig_shape(self.m, self.signers.len())?;
        let data = encoders::encode_initialize_multisig(self.m);
        let accounts = [
            InstructionAccount::writable(self.multisig.address()),
            InstructionAccount::readonly(self.rent_sysvar.address()),
        ];
        let views = [self.multisig, self.rent_sysvar];
        sink.emit(&data, accounts, views, &[Trailing::readonly(self.signers)])
    }
}

token_program_methods!(InitializeMultisig, owner = multisig);

/// `InitializeMultisig2` (19): [`InitializeMultisig`] without the Rent
/// sysvar account.
pub struct InitializeMultisig2<'a> {
    pub multisig: &'a AccountView<'a>,
    /// The member accounts, 1 to 11 of them.
    pub signers: &'a [&'a AccountView<'a>],
    /// How many members must sign, 1 to `signers.len()`.
    pub m: u8,
}

impl InitializeMultisig2<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeMultisig2<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        check_multisig_shape(self.m, self.signers.len())?;
        let data = encoders::encode_initialize_multisig2(self.m);
        let accounts = [InstructionAccount::writable(self.multisig.address())];
        let views = [self.multisig];
        sink.emit(&data, accounts, views, &[Trailing::readonly(self.signers)])
    }
}

token_program_methods!(InitializeMultisig2, owner = multisig);

// ---------------------------------------------------------------------

/// `InitializeImmutableOwner` (22): mark a not-yet-initialized token
/// account so that its owner can never be changed. Runs before
/// `InitializeAccount3`. On Token-2022 it needs the 4-byte extension
/// header in the allocation ([`GetAccountDataSize`] with type 7 reports
/// the size); SPL Token accepts it as a no-op on a 165-byte account.
pub struct InitializeImmutableOwner<'a> {
    pub account: &'a AccountView<'a>,
}

impl InitializeImmutableOwner<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeImmutableOwner<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_initialize_immutable_owner();
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

token_program_methods!(InitializeImmutableOwner, owner = account);

// ---------------------------------------------------------------------

/// `GetAccountDataSize` (21): ask the token program how many bytes a
/// token account for `mint` needs, as a `u64` in the return data. On
/// Token-2022 the mint's required account extensions are included
/// automatically and `extension_types` adds more (for example
/// [`crate::token_2022_ext::EXT_IMMUTABLE_OWNER`]); SPL Token ignores
/// the list and answers 165.
pub struct GetAccountDataSize<'a> {
    pub mint: &'a AccountView<'a>,
    pub extension_types: &'a [u16],
}

impl GetAccountDataSize<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    /// Invoke on `program` and read the answer from the return data.
    #[inline]
    pub fn query(&self, program: TokenProgram) -> Result<u64, ProgramError> {
        self.invoke_on(program, &[], &[])?;
        return_data_u64(program.address())
    }

    /// [`query`](Self::query) on whichever token program owns `mint`.
    #[inline]
    pub fn query_for_owner(&self) -> Result<u64, ProgramError> {
        self.query(TokenProgram::owning(self.mint)?)
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for GetAccountDataSize<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) = encoders::encode_extension_types(21, self.extension_types)
            .ok_or(ProgramError::InvalidArgument)?;
        let accounts = [InstructionAccount::readonly(self.mint.address())];
        let views = [self.mint];
        sink.emit(
            &data[..len],
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(GetAccountDataSize, owner = mint);

// ---------------------------------------------------------------------

/// `AmountToUiAmount` (23): format a raw amount with the mint's decimals
/// (and, on Token-2022, its interest or scaled-UI configuration) as a
/// UTF-8 string in the return data. Read it with [`return_data_string`].
pub struct AmountToUiAmount<'a> {
    pub mint: &'a AccountView<'a>,
    pub amount: u64,
}

impl AmountToUiAmount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    /// Invoke on `program` and copy the string the program returned into
    /// `out`, giving its length.
    #[inline]
    pub fn query(
        &self,
        program: TokenProgram,
        out: &mut [u8; MAX_UI_AMOUNT_LEN],
    ) -> Result<usize, ProgramError> {
        self.invoke_on(program, &[], &[])?;
        return_data_string(program.address(), out)
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for AmountToUiAmount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_amount_to_ui_amount(self.amount);
        let accounts = [InstructionAccount::readonly(self.mint.address())];
        let views = [self.mint];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(AmountToUiAmount, owner = mint);

/// `UiAmountToAmount` (24): parse a decimal string against the mint and
/// return the raw amount as a `u64` in the return data. Read it with
/// [`return_data_u64`]. Strings longer than [`MAX_UI_AMOUNT_LEN`] bytes
/// are refused before the CPI.
pub struct UiAmountToAmount<'a> {
    pub mint: &'a AccountView<'a>,
    pub ui_amount: &'a str,
}

impl UiAmountToAmount<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    /// Invoke on `program` and read the raw amount from the return data.
    #[inline]
    pub fn query(&self, program: TokenProgram) -> Result<u64, ProgramError> {
        self.invoke_on(program, &[], &[])?;
        return_data_u64(program.address())
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for UiAmountToAmount<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) = encoders::encode_ui_amount_to_amount(self.ui_amount)
            .ok_or(ProgramError::InvalidArgument)?;
        let accounts = [InstructionAccount::readonly(self.mint.address())];
        let views = [self.mint];
        sink.emit(
            &data[..len],
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(UiAmountToAmount, owner = mint);

// ---------------------------------------------------------------------

/// `WithdrawExcessLamports` (38): move every lamport above the
/// rent-exempt minimum out of a token account, mint, or multisig into
/// `destination`. `authority` is the account's owner (or the mint's
/// close authority, or the multisig itself), directly signed or through
/// multisig signers.
pub struct WithdrawExcessLamports<'a> {
    pub source: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
}

impl WithdrawExcessLamports<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.emit(multisig_signers, &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for WithdrawExcessLamports<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_withdraw_excess_lamports();
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::writable(self.destination.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.source, self.destination, self.authority];
        sink.emit(
            &data,
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(
    WithdrawExcessLamports,
    owner = source,
    authority = authority
);

/// `UnwrapLamports` (45): move lamports out of a native (wrapped SOL)
/// token account to `destination` without closing it, the whole balance
/// with `amount: None` or a part of it. The instruction exists in the
/// p-token build of SPL Token; a program that does not know discriminator
/// 45 refuses it with `InvalidInstructionData`.
pub struct UnwrapLamports<'a> {
    pub source: &'a AccountView<'a>,
    pub destination: &'a AccountView<'a>,
    pub authority: &'a AccountView<'a>,
    pub amount: Option<u64>,
}

impl UnwrapLamports<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        require_authority_signed_direct(self.authority)?;
        self.emit(&[], &mut Invoke::legacy(&[]))
    }

    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.emit(&[], &mut Invoke::legacy(signers))
    }

    #[inline]
    pub fn invoke_multisig(&self, multisig_signers: &[&AccountView<'_>]) -> ProgramResult {
        require_multisig_signers_direct(multisig_signers)?;
        self.emit(multisig_signers, &mut Invoke::legacy(&[]))
    }
}

impl<'a, 'x: 'a> TokenInstruction<'a> for UnwrapLamports<'x> {
    #[inline(always)]
    fn emit(
        &self,
        multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let (data, len) = encoders::encode_unwrap_lamports(self.amount);
        let accounts = [
            InstructionAccount::writable(self.source.address()),
            InstructionAccount::writable(self.destination.address()),
            authority_meta(self.authority, multisig_signers),
        ];
        let views = [self.source, self.destination, self.authority];
        sink.emit(
            &data[..len],
            accounts,
            views,
            &[Trailing::signers(multisig_signers)],
        )
    }
}

token_program_methods!(UnwrapLamports, owner = source, authority = authority);

// ---------------------------------------------------------------------

/// The `u64` a token program left in the return data, refusing return
/// data set by any other program or shorter than eight bytes.
#[inline]
pub fn return_data_u64(program: &Address) -> Result<u64, ProgramError> {
    let returned =
        crate::return_data::get_return_data().ok_or(ProgramError::InvalidInstructionData)?;
    if returned.program_id() != program {
        return Err(ProgramError::IncorrectProgramId);
    }
    let bytes: [u8; 8] = returned
        .data()
        .get(..8)
        .and_then(|b| b.try_into().ok())
        .ok_or(ProgramError::InvalidInstructionData)?;
    Ok(u64::from_le_bytes(bytes))
}

/// The string a token program left in the return data, copied into `out`;
/// the result is its length. Refuses return data from any other program
/// and anything longer than the buffer.
#[inline]
pub fn return_data_string(
    program: &Address,
    out: &mut [u8; MAX_UI_AMOUNT_LEN],
) -> Result<usize, ProgramError> {
    let returned =
        crate::return_data::get_return_data().ok_or(ProgramError::InvalidInstructionData)?;
    if returned.program_id() != program {
        return Err(ProgramError::IncorrectProgramId);
    }
    let data = returned.data();
    if data.len() > out.len() {
        return Err(ProgramError::InvalidInstructionData);
    }
    out[..data.len()].copy_from_slice(data);
    Ok(data.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multisig_shape_is_checked_before_the_cpi() {
        assert!(check_multisig_shape(1, 1).is_ok());
        assert!(check_multisig_shape(11, 11).is_ok());
        assert!(check_multisig_shape(0, 1).is_err());
        assert!(check_multisig_shape(2, 1).is_err());
        assert!(check_multisig_shape(1, 0).is_err());
        assert!(check_multisig_shape(1, 12).is_err());
    }

    #[test]
    fn admin_encoders_match_the_token_wire_format() {
        let authority = [7u8; 32];
        let freeze = [9u8; 32];
        let (data, len) = encoders::encode_initialize_mint(6, &authority, Some(&freeze));
        assert_eq!(len, 67);
        assert_eq!(&data[..2], &[0, 6]);
        assert_eq!(&data[2..34], &authority);
        assert_eq!(data[34], 1);
        assert_eq!(&data[35..67], &freeze);
        let (data, len) = encoders::encode_initialize_mint(0, &authority, None);
        assert_eq!(len, 35);
        assert_eq!(data[34], 0);

        assert_eq!(encoders::encode_initialize_multisig(2), [2, 2]);
        assert_eq!(encoders::encode_initialize_multisig2(3), [19, 3]);
        assert_eq!(encoders::encode_initialize_immutable_owner(), [22]);
        assert_eq!(encoders::encode_withdraw_excess_lamports(), [38]);
        assert_eq!(
            encoders::encode_amount_to_ui_amount(258),
            [23, 2, 1, 0, 0, 0, 0, 0, 0]
        );

        let (data, len) = encoders::encode_extension_types(21, &[7, 0x0102]).unwrap();
        assert_eq!(len, 5);
        assert_eq!(&data[..5], &[21, 7, 0, 2, 1]);
        let (data, len) = encoders::encode_extension_types(29, &[]).unwrap();
        assert_eq!((data[0], len), (29, 1));
        assert!(encoders::encode_extension_types(21, &[0; MAX_EXTENSION_TYPES + 1]).is_none());

        let (data, len) = encoders::encode_ui_amount_to_amount("1.5").unwrap();
        assert_eq!(&data[..len], b"\x181.5");
        let long = [b'1'; MAX_UI_AMOUNT_LEN];
        let (_, len) =
            encoders::encode_ui_amount_to_amount(core::str::from_utf8(&long).unwrap()).unwrap();
        assert_eq!(len, 1 + MAX_UI_AMOUNT_LEN);
        let too_long = [b'1'; MAX_UI_AMOUNT_LEN + 1];
        assert!(
            encoders::encode_ui_amount_to_amount(core::str::from_utf8(&too_long).unwrap())
                .is_none()
        );

        let (data, len) = encoders::encode_unwrap_lamports(None);
        assert_eq!(&data[..len], &[45, 0]);
        let (data, len) = encoders::encode_unwrap_lamports(Some(1));
        assert_eq!(&data[..len], &[45, 1, 1, 0, 0, 0, 0, 0, 0, 0]);
    }
}
