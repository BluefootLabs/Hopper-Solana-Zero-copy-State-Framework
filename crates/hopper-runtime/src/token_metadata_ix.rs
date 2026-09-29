//! Token-metadata and token-group interface instructions.
//!
//! Both interfaces are implemented by Token-2022 itself (a mint that
//! carries the `TokenMetadata` or `TokenGroup` extension is its own
//! metadata or group account) and can be implemented by any other program,
//! so every builder here targets Token-2022 from `invoke()` and any
//! interface program from `invoke_on_program`.
//!
//! Instructions are selected by an 8-byte discriminator (the first eight
//! bytes of the SHA-256 of the interface's hash input); the payload is
//! Borsh: a string is a little-endian `u32` length followed by its bytes,
//! an `Option<u64>` is a tag byte and, when present, the value, and an
//! optional address is 32 bytes that are all zero when absent.
//!
//! The variable-length instructions are encoded on the stack. The whole
//! instruction must fit [`MAX_METADATA_INSTRUCTION_DATA`] bytes; a longer
//! one is refused with `InvalidArgument` before the CPI. Token-2022
//! reallocates the mint to hold the metadata and does not fund it: the
//! caller transfers the additional rent to the mint first.

use crate::account::AccountView;
use crate::address::Address;
use crate::error::ProgramError;
use crate::instruction::{InstructionAccount, Signer};
use crate::token::{Invoke, TokenInstruction, TokenSink, TOKEN_2022_PROGRAM_ID};
use crate::ProgramResult;

/// The largest instruction data a metadata builder encodes.
pub const MAX_METADATA_INSTRUCTION_DATA: usize = 512;

/// `spl_token_metadata_interface:initialize_account`.
pub const DISC_METADATA_INITIALIZE: [u8; 8] = [210, 225, 30, 162, 88, 184, 77, 141];
/// `spl_token_metadata_interface:updating_field`.
pub const DISC_METADATA_UPDATE_FIELD: [u8; 8] = [221, 233, 49, 45, 181, 202, 220, 200];
/// `spl_token_metadata_interface:remove_key_ix`.
pub const DISC_METADATA_REMOVE_KEY: [u8; 8] = [234, 18, 32, 56, 89, 141, 37, 181];
/// `spl_token_metadata_interface:update_the_authority`.
pub const DISC_METADATA_UPDATE_AUTHORITY: [u8; 8] = [215, 228, 166, 228, 84, 100, 86, 123];
/// `spl_token_metadata_interface:emitter`.
pub const DISC_METADATA_EMIT: [u8; 8] = [250, 166, 180, 250, 13, 12, 184, 70];
/// `spl_token_group_interface:initialize_token_group`.
pub const DISC_GROUP_INITIALIZE: [u8; 8] = [121, 113, 108, 39, 54, 51, 0, 4];
/// `spl_token_group_interface:update_group_max_size`.
pub const DISC_GROUP_UPDATE_MAX_SIZE: [u8; 8] = [108, 37, 171, 143, 248, 30, 18, 110];
/// `spl_token_group_interface:update_authority`.
pub const DISC_GROUP_UPDATE_AUTHORITY: [u8; 8] = [161, 105, 88, 1, 237, 221, 216, 203];
/// `spl_token_group_interface:initialize_member`.
pub const DISC_GROUP_INITIALIZE_MEMBER: [u8; 8] = [152, 32, 222, 176, 223, 237, 116, 134];

/// A bounded instruction-data writer. Every push checks the remaining
/// room, so an oversized string is an error, never a truncation.
pub struct Data {
    buf: [u8; MAX_METADATA_INSTRUCTION_DATA],
    len: usize,
}

impl Data {
    #[inline(always)]
    fn new(discriminator: &[u8; 8]) -> Self {
        let mut buf = [0u8; MAX_METADATA_INSTRUCTION_DATA];
        buf[..8].copy_from_slice(discriminator);
        Self { buf, len: 8 }
    }

    #[inline(always)]
    fn bytes(&mut self, bytes: &[u8]) -> ProgramResult {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(ProgramError::InvalidArgument)?;
        if end > MAX_METADATA_INSTRUCTION_DATA {
            return Err(ProgramError::InvalidArgument);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    /// A Borsh string: `u32` length, then the bytes.
    #[inline(always)]
    fn string(&mut self, text: &str) -> ProgramResult {
        let len = u32::try_from(text.len()).map_err(|_| ProgramError::InvalidArgument)?;
        self.bytes(&len.to_le_bytes())?;
        self.bytes(text.as_bytes())
    }

    /// An optional address: the address, or 32 zero bytes. A present
    /// all-zero address would read back as absent, so it is refused.
    #[inline(always)]
    fn nullable(&mut self, address: Option<&Address>) -> ProgramResult {
        match address {
            Some(address) if address.as_array() == &[0u8; 32] => Err(ProgramError::InvalidArgument),
            Some(address) => self.bytes(address.as_array()),
            None => self.bytes(&[0u8; 32]),
        }
    }

    /// A Borsh `Option<u64>`.
    #[inline(always)]
    fn option_u64(&mut self, value: Option<u64>) -> ProgramResult {
        match value {
            Some(value) => {
                self.bytes(&[1])?;
                self.bytes(&value.to_le_bytes())
            }
            None => self.bytes(&[0]),
        }
    }

    /// The encoded bytes.
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// The field an [`UpdateMetadataField`] writes: one of the three base
/// fields or an additional key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataField<'a> {
    Name,
    Symbol,
    Uri,
    Key(&'a str),
}

/// Byte-exact encoders, shared by the builders and the tests.
pub mod encoders {
    use super::*;

    /// `[disc][name][symbol][uri]`, each a Borsh string.
    #[inline(always)]
    pub fn encode_initialize(name: &str, symbol: &str, uri: &str) -> Result<Data, ProgramError> {
        let mut data = Data::new(&DISC_METADATA_INITIALIZE);
        data.string(name)?;
        data.string(symbol)?;
        data.string(uri)?;
        Ok(data)
    }

    /// `[disc][field][value]`: the field is a variant byte (0 name, 1
    /// symbol, 2 URI, 3 key followed by the key string).
    #[inline(always)]
    pub fn encode_update_field(
        field: MetadataField<'_>,
        value: &str,
    ) -> Result<Data, ProgramError> {
        let mut data = Data::new(&DISC_METADATA_UPDATE_FIELD);
        match field {
            MetadataField::Name => data.bytes(&[0])?,
            MetadataField::Symbol => data.bytes(&[1])?,
            MetadataField::Uri => data.bytes(&[2])?,
            MetadataField::Key(key) => {
                data.bytes(&[3])?;
                data.string(key)?;
            }
        }
        data.string(value)?;
        Ok(data)
    }

    /// `[disc][idempotent][key]`.
    #[inline(always)]
    pub fn encode_remove_key(idempotent: bool, key: &str) -> Result<Data, ProgramError> {
        let mut data = Data::new(&DISC_METADATA_REMOVE_KEY);
        data.bytes(&[u8::from(idempotent)])?;
        data.string(key)?;
        Ok(data)
    }

    /// `[disc][new_authority: nullable 32]`, for the metadata
    /// (`DISC_METADATA_UPDATE_AUTHORITY`) and the group
    /// (`DISC_GROUP_UPDATE_AUTHORITY`) authority.
    #[inline(always)]
    pub fn encode_update_authority(
        discriminator: &[u8; 8],
        new_authority: Option<&Address>,
    ) -> Result<Data, ProgramError> {
        let mut data = Data::new(discriminator);
        data.nullable(new_authority)?;
        Ok(data)
    }

    /// `[disc][start: Option<u64>][end: Option<u64>]`.
    #[inline(always)]
    pub fn encode_emit(start: Option<u64>, end: Option<u64>) -> Result<Data, ProgramError> {
        let mut data = Data::new(&DISC_METADATA_EMIT);
        data.option_u64(start)?;
        data.option_u64(end)?;
        Ok(data)
    }

    /// `[disc][update_authority: nullable 32][max_size: u64 LE]`.
    #[inline(always)]
    pub fn encode_initialize_group(
        update_authority: Option<&Address>,
        max_size: u64,
    ) -> Result<Data, ProgramError> {
        let mut data = Data::new(&DISC_GROUP_INITIALIZE);
        data.nullable(update_authority)?;
        data.bytes(&max_size.to_le_bytes())?;
        Ok(data)
    }

    /// `[disc][max_size: u64 LE]`.
    #[inline(always)]
    pub fn encode_update_group_max_size(max_size: u64) -> Result<Data, ProgramError> {
        let mut data = Data::new(&DISC_GROUP_UPDATE_MAX_SIZE);
        data.bytes(&max_size.to_le_bytes())?;
        Ok(data)
    }

    /// `[disc]`.
    #[inline(always)]
    pub fn encode_initialize_member() -> Data {
        Data::new(&DISC_GROUP_INITIALIZE_MEMBER)
    }
}

/// The entry points of an interface builder: Token-2022 by default, any
/// program that implements the interface by address.
macro_rules! interface_methods {
    ($name:ident $(, signer = $signer:ident)?) => {
        impl $name<'_> {
            /// Send to Token-2022.
            #[inline]
            pub fn invoke(&self) -> ProgramResult {
                self.invoke_on_program(&TOKEN_2022_PROGRAM_ID, &[])
            }

            /// Send to Token-2022 with PDA signers.
            #[inline]
            pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
                self.invoke_on_program(&TOKEN_2022_PROGRAM_ID, signers)
            }

            /// Send to any program that implements the interface. With no
            /// PDA `signers`, the instruction's signer must have signed
            /// the transaction directly.
            #[inline]
            pub fn invoke_on_program(
                &self,
                program: &Address,
                signers: &[Signer<'_, '_>],
            ) -> ProgramResult {
                $(
                    if signers.is_empty() && !self.$signer.is_signer() {
                        return Err(ProgramError::MissingRequiredSignature);
                    }
                )?
                self.emit(&[], &mut Invoke { program, signers })
            }
        }
    };
}

// ---------------------------------------------------------------------
// Token metadata

/// `Initialize`: write the base metadata (name, symbol, URI) into
/// `metadata`, which for Token-2022 is the mint itself. The mint authority
/// signs. The account must already hold the rent for its new size.
pub struct InitializeTokenMetadata<'a> {
    pub metadata: &'a AccountView<'a>,
    pub update_authority: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub mint_authority: &'a AccountView<'a>,
    pub name: &'a str,
    pub symbol: &'a str,
    pub uri: &'a str,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeTokenMetadata<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_initialize(self.name, self.symbol, self.uri)?;
        let accounts = [
            InstructionAccount::writable(self.metadata.address()),
            InstructionAccount::readonly(self.update_authority.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly_signer(self.mint_authority.address()),
        ];
        let views = [
            self.metadata,
            self.update_authority,
            self.mint,
            self.mint_authority,
        ];
        sink.emit(data.as_slice(), accounts, views, &[])
    }
}

interface_methods!(InitializeTokenMetadata, signer = mint_authority);

/// A builder over a writable target and one signing authority.
macro_rules! authority_instruction {
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
                _multisig_signers: &[&'a AccountView<'a>],
                sink: &mut impl TokenSink<'a>,
            ) -> ProgramResult {
                let $s = self;
                let data: Data = $data;
                let accounts = [
                    InstructionAccount::writable(self.$target.address()),
                    InstructionAccount::readonly_signer(self.$auth.address()),
                ];
                let views = [self.$target, self.$auth];
                sink.emit(data.as_slice(), accounts, views, &[])
            }
        }

        interface_methods!($name, signer = $auth);
    };
}

authority_instruction! {
    /// `UpdateField`: set the name, the symbol, the URI, or an additional
    /// key to `value`. A new key or a longer value grows the account,
    /// which must already hold the rent for it.
    UpdateMetadataField { target = metadata, authority = update_authority, field: MetadataField<'a>, value: &'a str }
    data = |s| encoders::encode_update_field(s.field, s.value)?;
}

authority_instruction! {
    /// `RemoveKey`: remove an additional key. With `idempotent` a missing
    /// key is not an error.
    RemoveMetadataKey { target = metadata, authority = update_authority, idempotent: bool, key: &'a str }
    data = |s| encoders::encode_remove_key(s.idempotent, s.key)?;
}

authority_instruction! {
    /// `UpdateAuthority`: hand the metadata's update authority to
    /// `new_authority`, or remove it with `None`.
    UpdateMetadataAuthority { target = metadata, authority = current_authority, new_authority: Option<&'a Address> }
    data = |s| encoders::encode_update_authority(&DISC_METADATA_UPDATE_AUTHORITY, s.new_authority)?;
}

/// `Emit`: return the serialized metadata (or the `start..end` slice of
/// it) as the program's return data.
pub struct EmitTokenMetadata<'a> {
    pub metadata: &'a AccountView<'a>,
    pub start: Option<u64>,
    pub end: Option<u64>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for EmitTokenMetadata<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_emit(self.start, self.end)?;
        let accounts = [InstructionAccount::readonly(self.metadata.address())];
        let views = [self.metadata];
        sink.emit(data.as_slice(), accounts, views, &[])
    }
}

interface_methods!(EmitTokenMetadata);

// ---------------------------------------------------------------------
// Token group

/// `InitializeGroup`: make `group` (for Token-2022, the mint itself) a
/// group of at most `max_size` members. The mint authority signs.
pub struct InitializeTokenGroup<'a> {
    pub group: &'a AccountView<'a>,
    pub mint: &'a AccountView<'a>,
    pub mint_authority: &'a AccountView<'a>,
    pub update_authority: Option<&'a Address>,
    pub max_size: u64,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeTokenGroup<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_initialize_group(self.update_authority, self.max_size)?;
        let accounts = [
            InstructionAccount::writable(self.group.address()),
            InstructionAccount::readonly(self.mint.address()),
            InstructionAccount::readonly_signer(self.mint_authority.address()),
        ];
        let views = [self.group, self.mint, self.mint_authority];
        sink.emit(data.as_slice(), accounts, views, &[])
    }
}

interface_methods!(InitializeTokenGroup, signer = mint_authority);

authority_instruction! {
    /// `UpdateGroupMaxSize`: change the largest number of members.
    UpdateTokenGroupMaxSize { target = group, authority = update_authority, max_size: u64 }
    data = |s| encoders::encode_update_group_max_size(s.max_size)?;
}

authority_instruction! {
    /// `UpdateGroupAuthority`: hand the group's update authority to
    /// `new_authority`, or remove it with `None`.
    UpdateTokenGroupAuthority { target = group, authority = current_authority, new_authority: Option<&'a Address> }
    data = |s| encoders::encode_update_authority(&DISC_GROUP_UPDATE_AUTHORITY, s.new_authority)?;
}

/// `InitializeMember`: make `member` (for Token-2022, the member's mint)
/// a member of `group`. The member's mint authority and the group's
/// update authority both sign.
pub struct InitializeTokenGroupMember<'a> {
    pub member: &'a AccountView<'a>,
    pub member_mint: &'a AccountView<'a>,
    pub member_mint_authority: &'a AccountView<'a>,
    pub group: &'a AccountView<'a>,
    pub group_update_authority: &'a AccountView<'a>,
}

impl<'a, 'x: 'a> TokenInstruction<'a> for InitializeTokenGroupMember<'x> {
    #[inline(always)]
    fn emit(
        &self,
        _multisig_signers: &[&'a AccountView<'a>],
        sink: &mut impl TokenSink<'a>,
    ) -> ProgramResult {
        let data = encoders::encode_initialize_member();
        let accounts = [
            InstructionAccount::writable(self.member.address()),
            InstructionAccount::readonly(self.member_mint.address()),
            InstructionAccount::readonly_signer(self.member_mint_authority.address()),
            InstructionAccount::writable(self.group.address()),
            InstructionAccount::readonly_signer(self.group_update_authority.address()),
        ];
        let views = [
            self.member,
            self.member_mint,
            self.member_mint_authority,
            self.group,
            self.group_update_authority,
        ];
        sink.emit(data.as_slice(), accounts, views, &[])
    }
}

impl InitializeTokenGroupMember<'_> {
    /// Send to Token-2022; both authorities signed directly.
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_on_program(&TOKEN_2022_PROGRAM_ID, &[])
    }

    /// Send to Token-2022 with PDA signers.
    #[inline]
    pub fn invoke_signed(&self, signers: &[Signer<'_, '_>]) -> ProgramResult {
        self.invoke_on_program(&TOKEN_2022_PROGRAM_ID, signers)
    }

    /// Send to any program that implements the group interface. With no
    /// PDA `signers`, both authorities must have signed directly.
    #[inline]
    pub fn invoke_on_program(
        &self,
        program: &Address,
        signers: &[Signer<'_, '_>],
    ) -> ProgramResult {
        if signers.is_empty()
            && !(self.member_mint_authority.is_signer() && self.group_update_authority.is_signer())
        {
            return Err(ProgramError::MissingRequiredSignature);
        }
        self.emit(&[], &mut Invoke { program, signers })
    }
}

#[cfg(test)]
mod tests {
    use super::encoders::*;
    use super::*;

    #[test]
    fn discriminators_are_the_hash_prefixes_of_the_interface_inputs() {
        for (input, expected) in [
            (
                "spl_token_metadata_interface:initialize_account",
                DISC_METADATA_INITIALIZE,
            ),
            (
                "spl_token_metadata_interface:updating_field",
                DISC_METADATA_UPDATE_FIELD,
            ),
            (
                "spl_token_metadata_interface:remove_key_ix",
                DISC_METADATA_REMOVE_KEY,
            ),
            (
                "spl_token_metadata_interface:update_the_authority",
                DISC_METADATA_UPDATE_AUTHORITY,
            ),
            ("spl_token_metadata_interface:emitter", DISC_METADATA_EMIT),
            (
                "spl_token_group_interface:initialize_token_group",
                DISC_GROUP_INITIALIZE,
            ),
            (
                "spl_token_group_interface:update_group_max_size",
                DISC_GROUP_UPDATE_MAX_SIZE,
            ),
            (
                "spl_token_group_interface:update_authority",
                DISC_GROUP_UPDATE_AUTHORITY,
            ),
            (
                "spl_token_group_interface:initialize_member",
                DISC_GROUP_INITIALIZE_MEMBER,
            ),
        ] {
            let digest = hopper_native::hash::sha256(&[input.as_bytes()]).unwrap();
            assert_eq!(&digest[..8], &expected, "{input}");
        }
    }

    #[test]
    fn strings_are_length_prefixed_and_bounded() {
        let data = encode_initialize("Hopper", "HOP", "https://hopperzero.dev/t.json").unwrap();
        let bytes = data.as_slice();
        assert_eq!(&bytes[..8], &DISC_METADATA_INITIALIZE);
        assert_eq!(&bytes[8..12], &6u32.to_le_bytes());
        assert_eq!(&bytes[12..18], b"Hopper");
        assert_eq!(&bytes[18..22], &3u32.to_le_bytes());
        assert_eq!(&bytes[22..25], b"HOP");
        assert_eq!(bytes.len(), 8 + 4 + 6 + 4 + 3 + 4 + 29);

        let long = "x".repeat(MAX_METADATA_INSTRUCTION_DATA);
        assert_eq!(
            encode_initialize(&long, "", "").err(),
            Some(ProgramError::InvalidArgument)
        );
        // Exactly at the limit is accepted.
        let fits = "x".repeat(MAX_METADATA_INSTRUCTION_DATA - 8 - 12);
        assert_eq!(
            encode_initialize(&fits, "", "").unwrap().as_slice().len(),
            MAX_METADATA_INSTRUCTION_DATA
        );
    }

    #[test]
    fn fields_options_and_authorities_encode_as_borsh() {
        assert_eq!(
            &encode_update_field(MetadataField::Uri, "u")
                .unwrap()
                .as_slice()[8..],
            &[2, 1, 0, 0, 0, b'u']
        );
        assert_eq!(
            &encode_update_field(MetadataField::Key("k"), "v")
                .unwrap()
                .as_slice()[8..],
            &[3, 1, 0, 0, 0, b'k', 1, 0, 0, 0, b'v']
        );
        assert_eq!(
            &encode_remove_key(true, "k").unwrap().as_slice()[8..],
            &[1, 1, 0, 0, 0, b'k']
        );
        assert_eq!(&encode_emit(None, None).unwrap().as_slice()[8..], &[0, 0]);
        assert_eq!(
            &encode_emit(Some(2), None).unwrap().as_slice()[8..],
            &[1, 2, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        let authority = Address::new_from_array([7; 32]);
        assert_eq!(
            &encode_update_authority(&DISC_GROUP_UPDATE_AUTHORITY, Some(&authority))
                .unwrap()
                .as_slice()[8..],
            &[7u8; 32]
        );
        assert_eq!(
            &encode_update_authority(&DISC_METADATA_UPDATE_AUTHORITY, None)
                .unwrap()
                .as_slice()[8..],
            &[0u8; 32]
        );
        assert!(encode_update_authority(
            &DISC_METADATA_UPDATE_AUTHORITY,
            Some(&Address::new_from_array([0; 32]))
        )
        .is_err());
        assert_eq!(
            &encode_initialize_group(None, 9).unwrap().as_slice()[40..],
            &9u64.to_le_bytes()
        );
        assert_eq!(encode_initialize_member().as_slice().len(), 8);
    }
}
