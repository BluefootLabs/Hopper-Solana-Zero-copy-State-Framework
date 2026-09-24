//! Allocation and initialization of legacy SPL and Token-2022 mints.
//!
//! A [`MintPlan`] ties the exact allocation to the extension initializers it
//! will execute. It uses no heap allocation and initializes extensions before
//! the base mint. It deliberately supports a bounded set of fixed-size mint
//! extensions; variable-length metadata and confidential extensions are not
//! inferred or initialized automatically.

use crate::instruction::{InstructionAccount, InstructionView, Signer};
use crate::{AccountView, Address, ProgramError, ProgramResult};

/// Token-2022 program address.
pub const TOKEN_2022_PROGRAM_ID: Address = Address::new_from_array(crate::__decode_base58_32(
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
));

/// The two token programs supported by mint initialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MintProgram {
    Legacy,
    Token2022,
}

impl MintProgram {
    pub const fn address(self) -> &'static Address {
        match self {
            Self::Legacy => &crate::token::TOKEN_PROGRAM_ID,
            Self::Token2022 => &TOKEN_2022_PROGRAM_ID,
        }
    }
}

/// Base mint configuration. Initializing a mint does not mint any supply.
#[derive(Clone, Copy)]
pub struct MintConfig<'a> {
    pub decimals: u8,
    pub mint_authority: &'a Address,
    pub freeze_authority: Option<&'a Address>,
}

/// Fixed-size mint extensions supported by [`MintPlan`].
///
/// Extension authorities and hook/metadata addresses use Token-2022's nullable
/// address encoding where applicable. `Some(zero_address)` is rejected for
/// those fields, rather than silently being interpreted as `None`.
#[derive(Clone, Copy)]
pub enum MintExtension<'a> {
    TransferFeeConfig {
        authority: Option<&'a Address>,
        withdraw_authority: Option<&'a Address>,
        basis_points: u16,
        maximum_fee: u64,
    },
    MintCloseAuthority(Option<&'a Address>),
    NonTransferable,
    PermanentDelegate(&'a Address),
    TransferHook {
        authority: Option<&'a Address>,
        program_id: Option<&'a Address>,
    },
    MetadataPointer {
        authority: Option<&'a Address>,
        metadata_address: Option<&'a Address>,
    },
}

impl MintExtension<'_> {
    /// Token-2022's TLV extension discriminator (not an instruction tag).
    pub const fn extension_type(&self) -> u16 {
        match self {
            Self::TransferFeeConfig { .. } => 1,
            Self::MintCloseAuthority(_) => 3,
            Self::NonTransferable => 9,
            Self::PermanentDelegate(_) => 12,
            Self::TransferHook { .. } => 14,
            Self::MetadataPointer { .. } => 18,
        }
    }

    const fn value_len(&self) -> usize {
        match self {
            Self::TransferFeeConfig { .. } => 108,
            Self::MintCloseAuthority(_) | Self::PermanentDelegate(_) => 32,
            Self::NonTransferable => 0,
            Self::TransferHook { .. } | Self::MetadataPointer { .. } => 64,
        }
    }

    fn validate(&self) -> ProgramResult {
        fn nullable(value: Option<&Address>) -> ProgramResult {
            if value.is_some_and(|a| a.as_array() == &[0; 32]) {
                Err(ProgramError::InvalidArgument)
            } else {
                Ok(())
            }
        }
        match *self {
            Self::TransferFeeConfig {
                authority,
                withdraw_authority,
                basis_points,
                ..
            } => {
                nullable(authority)?;
                nullable(withdraw_authority)?;
                if basis_points > 10_000 {
                    return Err(ProgramError::InvalidArgument);
                }
            }
            Self::MintCloseAuthority(authority) => nullable(authority)?,
            Self::PermanentDelegate(delegate) => nullable(Some(delegate))?,
            Self::TransferHook {
                authority,
                program_id,
            } => {
                nullable(authority)?;
                nullable(program_id)?;
            }
            Self::MetadataPointer {
                authority,
                metadata_address,
            } => {
                nullable(authority)?;
                nullable(metadata_address)?;
            }
            Self::NonTransferable => {}
        }
        Ok(())
    }

    /// Canonical instruction bytes, also usable by off-chain instruction builders.
    pub fn instruction_data(&self) -> Result<MintInstructionData, ProgramError> {
        self.validate()?;
        let mut out = MintInstructionData::new();
        match *self {
            Self::TransferFeeConfig {
                authority,
                withdraw_authority,
                basis_points,
                maximum_fee,
            } => {
                out.push(&[26, 0]);
                out.option(authority);
                out.option(withdraw_authority);
                out.push(&basis_points.to_le_bytes());
                out.push(&maximum_fee.to_le_bytes());
            }
            Self::MintCloseAuthority(authority) => {
                out.push(&[25]);
                out.option(authority);
            }
            Self::NonTransferable => out.push(&[32]),
            Self::PermanentDelegate(delegate) => {
                out.push(&[35]);
                out.push(delegate.as_array());
            }
            Self::TransferHook {
                authority,
                program_id,
            } => {
                out.push(&[36, 0]);
                out.nullable(authority);
                out.nullable(program_id);
            }
            Self::MetadataPointer {
                authority,
                metadata_address,
            } => {
                out.push(&[39, 0]);
                out.nullable(authority);
                out.nullable(metadata_address);
            }
        }
        Ok(out)
    }
}

/// Stack-backed canonical mint instruction data, bounded at 78 bytes.
pub struct MintInstructionData {
    bytes: [u8; 78],
    len: usize,
}

impl MintInstructionData {
    fn new() -> Self {
        Self {
            bytes: [0; 78],
            len: 0,
        }
    }
    fn push(&mut self, bytes: &[u8]) {
        self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }
    fn option(&mut self, address: Option<&Address>) {
        self.push(&[u8::from(address.is_some())]);
        if let Some(address) = address {
            self.push(address.as_array());
        }
    }
    fn nullable(&mut self, address: Option<&Address>) {
        self.push(address.map_or(&[0; 32][..], |a| &a.as_array()[..]));
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl MintConfig<'_> {
    /// Encode `InitializeMint2` (no Rent sysvar account is required).
    pub fn instruction_data(&self) -> MintInstructionData {
        let mut out = MintInstructionData::new();
        out.push(&[20, self.decimals]);
        out.push(self.mint_authority.as_array());
        out.option(self.freeze_authority);
        out
    }
}

/// Low-level `InitializeMint2` CPI builder for either token program.
///
/// The token processor checks allocation, rent, extension compatibility and
/// initialization state. Use [`MintPlan`] to also bind allocation and extension
/// initialization together. The mint must have been allocated and assigned to
/// the selected token program already.
pub struct InitializeMint2<'a, 'b> {
    pub mint: &'a AccountView<'a>,
    pub program: MintProgram,
    pub config: MintConfig<'b>,
}

impl InitializeMint2<'_, '_> {
    pub fn invoke(&self) -> ProgramResult {
        invoke_mint(
            self.mint,
            self.program,
            self.config.instruction_data().as_bytes(),
        )
    }
}

fn invoke_mint(mint: &AccountView<'_>, program: MintProgram, data: &[u8]) -> ProgramResult {
    if !mint.owned_by(program.address()) {
        return Err(ProgramError::IncorrectProgramId);
    }
    if !mint.is_writable() {
        return Err(ProgramError::InvalidArgument);
    }
    let accounts = [InstructionAccount::writable(mint.address())];
    let instruction = InstructionView {
        program_id: program.address(),
        accounts: &accounts,
        data,
    };
    crate::cpi::invoke(&instruction, &[mint])
}

/// Validated, allocation-free mint creation plan.
///
/// `new` rejects duplicate extensions, invalid nullable addresses and fee rates,
/// and extensions on the legacy program. `space` includes Token-2022 padding
/// and TLV headers. There is no arbitrary spare capacity: Token-2022 requires
/// the allocation to equal the initialized extension set's canonical size.
pub struct MintPlan<'a> {
    program: MintProgram,
    config: MintConfig<'a>,
    extensions: &'a [MintExtension<'a>],
    space: usize,
}

impl<'a> MintPlan<'a> {
    pub fn new(
        program: MintProgram,
        config: MintConfig<'a>,
        extensions: &'a [MintExtension<'a>],
    ) -> Result<Self, ProgramError> {
        if program == MintProgram::Legacy && !extensions.is_empty() {
            return Err(ProgramError::InvalidArgument);
        }
        let mut seen = 0u32;
        let mut space = if extensions.is_empty() { 82 } else { 166 };
        for extension in extensions {
            extension.validate()?;
            let mask = 1u32 << extension.extension_type();
            if seen & mask != 0 {
                return Err(ProgramError::InvalidArgument);
            }
            seen |= mask;
            space += 4 + extension.value_len();
        }
        // Token-2022 reserves the legacy multisig size and pads past it.
        if space == 355 {
            space += 2;
        }
        Ok(Self {
            program,
            config,
            extensions,
            space,
        })
    }

    pub const fn space(&self) -> usize {
        self.space
    }

    /// Check an explicitly supplied allocation before paying rent or invoking CPI.
    pub fn check_space(&self, space: usize) -> ProgramResult {
        if space != self.space {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Initialize an already allocated, rent-exempt, entirely zeroed mint.
    /// Include the selected executable token program in the outer instruction.
    /// Propagate errors: if a later CPI fails, the enclosing instruction must
    /// fail to roll back earlier extension initialization.
    pub fn initialize(&self, mint: &AccountView<'_>) -> ProgramResult {
        if !mint.owned_by(self.program.address()) {
            return Err(ProgramError::IncorrectProgramId);
        }
        if !mint.is_writable() {
            return Err(ProgramError::InvalidArgument);
        }
        self.check_space(mint.data_len())?;
        if mint.lamports() < crate::rent::minimum_balance_live(self.space)? {
            return Err(ProgramError::AccountNotRentExempt);
        }
        {
            let data = mint.try_borrow()?;
            if data.iter().any(|b| *b != 0) {
                return Err(ProgramError::AccountAlreadyInitialized);
            }
        }
        for extension in self.extensions {
            invoke_mint(mint, self.program, extension.instruction_data()?.as_bytes())?;
        }
        InitializeMint2 {
            mint,
            program: self.program,
            config: self.config,
        }
        .invoke()
    }

    /// Allocate a fresh System-owned mint, initialize extensions, then the base
    /// mint. Prefunding is supported; only the live rent shortfall is charged.
    ///
    /// Include the System and selected token programs in the outer instruction.
    /// The mint and payer must sign, directly or through `signers`. As with any
    /// sequence of CPIs, propagate errors to preserve transaction atomicity.
    /// Uses System `CreateAccountAllowPrefund`; the target cluster must enable it.
    pub fn create(
        &self,
        payer: &AccountView<'_>,
        mint: &AccountView<'_>,
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        if payer.address() == mint.address() || !payer.is_writable() || !mint.is_writable() {
            return Err(ProgramError::InvalidArgument);
        }
        if !mint.owned_by(&crate::system::SYSTEM_PROGRAM_ID)
            || !payer.owned_by(&crate::system::SYSTEM_PROGRAM_ID)
        {
            return Err(ProgramError::IncorrectProgramId);
        }
        if mint.data_len() != 0 || payer.data_len() != 0 {
            return Err(ProgramError::InvalidAccountData);
        }
        let rent = crate::rent::minimum_balance_live(self.space)?;
        let funding = rent.saturating_sub(mint.lamports());
        if payer.lamports() < funding {
            return Err(ProgramError::InsufficientFunds);
        }
        crate::system::CreateAccountAllowPrefund {
            to: mint,
            funding: Some((payer, funding)),
            space: self.space as u64,
            owner: self.program.address(),
        }
        .invoke_signed(signers)?;
        self.initialize(mint)
    }
}
