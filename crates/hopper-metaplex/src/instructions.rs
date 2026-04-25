//! Builder structs for the three Metaplex Token Metadata calls Hopper
//! programs need most often.
//!
//! Each builder follows the same shape as `hopper_runtime::system::*`
//! and `hopper_runtime::token::*`: a struct with `&AccountView` and
//! `&str`/`u64`/`bool` fields, an `invoke()` method for unsigned CPI,
//! and an `invoke_signed()` method that takes `&[Signer]` for PDA
//! signing.
//!
//! ## Instruction discriminators
//!
//! These are the **enum-position** Borsh discriminators from
//! `mpl-token-metadata`'s `MetadataInstruction` enum, the canonical
//! on-chain encoding the program has accepted since deployment.
//! `mpl-token-metadata` 5.x added Shank-generated 8-byte discriminator
//! aliases for client tooling, but the on-chain dispatcher still
//! routes the legacy single-byte form. We use the legacy form because
//! it matches what's actually on chain and saves 7 bytes of
//! instruction data per call.
//!
//! - `CreateMetadataAccountV3` = 33
//! - `CreateMasterEditionV3`   = 17
//! - `UpdateMetadataAccountV2` = 15

use crate::constants::{
    MAX_NAME_LEN, MAX_SYMBOL_LEN, MAX_URI_LEN, MPL_TOKEN_METADATA_PROGRAM_ID,
};
use crate::encoding::BorshTape;
use hopper_runtime::account::AccountView;
use hopper_runtime::address::Address;
use hopper_runtime::error::ProgramError;
use hopper_runtime::instruction::{InstructionAccount, InstructionView, Signer};
use hopper_runtime::ProgramResult;

const DISC_UPDATE_METADATA_ACCOUNT_V2: u8 = 15;
const DISC_CREATE_MASTER_EDITION_V3: u8 = 17;
const DISC_CREATE_METADATA_ACCOUNT_V3: u8 = 33;

/// Borsh-shaped representation of Metaplex's `DataV2` payload.
///
/// Lower-level than the `CreateMetadataAccountV3` builder's flat
/// `name`/`symbol`/`uri`/`sfbp` fields. Use this directly when you
/// need to set creators / collection / uses; reach for the flat
/// builder fields when you don't.
#[derive(Clone, Copy)]
pub struct DataV2<'a> {
    pub name: &'a str,
    pub symbol: &'a str,
    pub uri: &'a str,
    pub seller_fee_basis_points: u16,
    /// Borsh `Option<Vec<Creator>>`. The current builder only emits
    /// `None` (no creators); pass `false` to keep the field empty.
    pub has_creators: bool,
    /// Borsh `Option<Collection>`. Builder emits `None`.
    pub has_collection: bool,
    /// Borsh `Option<Uses>`. Builder emits `None`.
    pub has_uses: bool,
}

impl<'a> DataV2<'a> {
    /// Construct a "simple" `DataV2` with no creators, collection, or
    /// uses. Most 1-of-1 NFT mints use exactly this shape.
    #[inline]
    pub const fn simple(
        name: &'a str,
        symbol: &'a str,
        uri: &'a str,
        seller_fee_basis_points: u16,
    ) -> Self {
        Self {
            name,
            symbol,
            uri,
            seller_fee_basis_points,
            has_creators: false,
            has_collection: false,
            has_uses: false,
        }
    }

    fn validate_lengths(&self) -> ProgramResult {
        if self.name.len() > MAX_NAME_LEN
            || self.symbol.len() > MAX_SYMBOL_LEN
            || self.uri.len() > MAX_URI_LEN
        {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }

    fn write_borsh(&self, tape: &mut BorshTape<'_>) -> ProgramResult {
        // Borsh layout of DataV2:
        //   string name | string symbol | string uri |
        //   u16 sfbp |
        //   Option<Vec<Creator>> | Option<Collection> | Option<Uses>
        tape.write_str(self.name)?;
        tape.write_str(self.symbol)?;
        tape.write_str(self.uri)?;
        tape.write_u16_le(self.seller_fee_basis_points)?;
        // Only the `simple` shape is supported in this first cut. The
        // flags are kept on the struct so a follow-up can extend the
        // builder without breaking source compatibility.
        if self.has_creators || self.has_collection || self.has_uses {
            return Err(ProgramError::InvalidInstructionData);
        }
        tape.write_option_none()?; // creators
        tape.write_option_none()?; // collection
        tape.write_option_none()?; // uses
        Ok(())
    }
}

// ── CreateMetadataAccountV3 ──────────────────────────────────────────

/// Builder for the Metaplex Token Metadata `CreateMetadataAccountV3`
/// instruction.
///
/// Initialises the metadata PDA for `mint` with the supplied
/// `DataV2`. The metadata PDA is at the canonical seeds
/// `["metadata", mpl_token_metadata_program_id, mint]` (see
/// [`crate::seeds::metadata_pda`]).
///
/// # Account ordering
///
/// 1. metadata          — writable, the PDA being created
/// 2. mint              — read-only, the SPL mint the metadata describes
/// 3. mint_authority    — signer (mint authority of `mint`)
/// 4. payer             — signer + writable (funds the new account)
/// 5. update_authority  — signer (the authority allowed to mutate the metadata later)
/// 6. system_program    — read-only
/// 7. rent (optional)   — read-only; modern Metaplex doesn't require it but
///    accepts it for backward compatibility
pub struct CreateMetadataAccountV3<'a> {
    pub metadata: &'a AccountView,
    pub mint: &'a AccountView,
    pub mint_authority: &'a AccountView,
    pub payer: &'a AccountView,
    pub update_authority: &'a AccountView,
    pub system_program: &'a AccountView,
    /// Rent sysvar account. Optional in current Metaplex; pass `None`
    /// to omit it from the account list (the on-chain program will
    /// load it from the sysvar cache when not supplied).
    pub rent: Option<&'a AccountView>,

    pub data: DataV2<'a>,
    pub is_mutable: bool,
}

impl CreateMetadataAccountV3<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        self.data.validate_lengths()?;

        // Borsh-encoded data payload, capped at 320 bytes (covers the
        // worst case: 32 + 10 + 200 + length prefixes + flags = 263
        // bytes; rounded up for safety). Stack-allocated.
        let mut buf = [0u8; 320];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_disc(DISC_CREATE_METADATA_ACCOUNT_V3)?;
        // CreateMetadataAccountArgsV3 layout:
        //   DataV2 data | bool is_mutable | Option<CollectionDetails> collection_details
        self.data.write_borsh_into(&mut tape)?;
        tape.write_bool(self.is_mutable)?;
        tape.write_option_none()?; // collection_details: None
        let len = tape.len();

        // Account list. The optional rent account is appended only if
        // present so the on-chain program sees the right account count.
        // Two branches because the metas array has a different length
        // in each case and Rust arrays carry their length in the type.
        if let Some(rent) = self.rent {
            let metas = [
                InstructionAccount::writable(self.metadata.address()),
                InstructionAccount::readonly(self.mint.address()),
                InstructionAccount::readonly_signer(self.mint_authority.address()),
                InstructionAccount::writable_signer(self.payer.address()),
                InstructionAccount::readonly_signer(self.update_authority.address()),
                InstructionAccount::readonly(self.system_program.address()),
                InstructionAccount::readonly(rent.address()),
            ];
            let views = [
                self.metadata,
                self.mint,
                self.mint_authority,
                self.payer,
                self.update_authority,
                self.system_program,
                rent,
            ];
            let instruction = InstructionView {
                program_id: &MPL_TOKEN_METADATA_PROGRAM_ID,
                data: &buf[..len],
                accounts: &metas,
            };
            hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
        } else {
            let metas = [
                InstructionAccount::writable(self.metadata.address()),
                InstructionAccount::readonly(self.mint.address()),
                InstructionAccount::readonly_signer(self.mint_authority.address()),
                InstructionAccount::writable_signer(self.payer.address()),
                InstructionAccount::readonly_signer(self.update_authority.address()),
                InstructionAccount::readonly(self.system_program.address()),
            ];
            let views = [
                self.metadata,
                self.mint,
                self.mint_authority,
                self.payer,
                self.update_authority,
                self.system_program,
            ];
            let instruction = InstructionView {
                program_id: &MPL_TOKEN_METADATA_PROGRAM_ID,
                data: &buf[..len],
                accounts: &metas,
            };
            hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
        }
    }
}

// ── CreateMasterEditionV3 ────────────────────────────────────────────

/// Builder for the Metaplex Token Metadata `CreateMasterEditionV3`
/// instruction.
///
/// Marks `mint` as a master edition with optional `max_supply` for
/// print editions. Set `max_supply = Some(0)` for a 1-of-1 NFT (no
/// prints), `Some(N)` for a numbered edition, `None` for unlimited.
///
/// # Account ordering
///
/// 1. edition           — writable, the master-edition PDA
/// 2. mint              — writable, the SPL mint
/// 3. update_authority  — signer
/// 4. mint_authority    — signer
/// 5. payer             — signer + writable
/// 6. metadata          — read-only, the metadata PDA from
///    `CreateMetadataAccountV3`
/// 7. token_program     — read-only (SPL Token program)
/// 8. system_program    — read-only
/// 9. rent (optional)   — read-only
pub struct CreateMasterEditionV3<'a> {
    pub edition: &'a AccountView,
    pub mint: &'a AccountView,
    pub update_authority: &'a AccountView,
    pub mint_authority: &'a AccountView,
    pub payer: &'a AccountView,
    pub metadata: &'a AccountView,
    pub token_program: &'a AccountView,
    pub system_program: &'a AccountView,
    pub rent: Option<&'a AccountView>,

    /// `None` for unlimited prints, `Some(0)` for a 1-of-1 NFT,
    /// `Some(N)` for a numbered edition.
    pub max_supply: Option<u64>,
}

impl CreateMasterEditionV3<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        let mut buf = [0u8; 16];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_disc(DISC_CREATE_MASTER_EDITION_V3)?;
        // CreateMasterEditionArgs { max_supply: Option<u64> }
        tape.write_option_u64_le(self.max_supply)?;
        let len = tape.len();

        if let Some(rent) = self.rent {
            let metas = [
                InstructionAccount::writable(self.edition.address()),
                InstructionAccount::writable(self.mint.address()),
                InstructionAccount::readonly_signer(self.update_authority.address()),
                InstructionAccount::readonly_signer(self.mint_authority.address()),
                InstructionAccount::writable_signer(self.payer.address()),
                InstructionAccount::readonly(self.metadata.address()),
                InstructionAccount::readonly(self.token_program.address()),
                InstructionAccount::readonly(self.system_program.address()),
                InstructionAccount::readonly(rent.address()),
            ];
            let views = [
                self.edition,
                self.mint,
                self.update_authority,
                self.mint_authority,
                self.payer,
                self.metadata,
                self.token_program,
                self.system_program,
                rent,
            ];
            let instruction = InstructionView {
                program_id: &MPL_TOKEN_METADATA_PROGRAM_ID,
                data: &buf[..len],
                accounts: &metas,
            };
            hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
        } else {
            let metas = [
                InstructionAccount::writable(self.edition.address()),
                InstructionAccount::writable(self.mint.address()),
                InstructionAccount::readonly_signer(self.update_authority.address()),
                InstructionAccount::readonly_signer(self.mint_authority.address()),
                InstructionAccount::writable_signer(self.payer.address()),
                InstructionAccount::readonly(self.metadata.address()),
                InstructionAccount::readonly(self.token_program.address()),
                InstructionAccount::readonly(self.system_program.address()),
            ];
            let views = [
                self.edition,
                self.mint,
                self.update_authority,
                self.mint_authority,
                self.payer,
                self.metadata,
                self.token_program,
                self.system_program,
            ];
            let instruction = InstructionView {
                program_id: &MPL_TOKEN_METADATA_PROGRAM_ID,
                data: &buf[..len],
                accounts: &metas,
            };
            hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
        }
    }
}

// ── UpdateMetadataAccountV2 ──────────────────────────────────────────

/// Builder for the Metaplex Token Metadata `UpdateMetadataAccountV2`
/// instruction.
///
/// Mutates an existing metadata account. Each field is `Option`-shaped;
/// `None` means "leave unchanged". The builder enforces this at the
/// Rust level so the caller can't accidentally overwrite a field they
/// didn't intend to.
///
/// # Account ordering
///
/// 1. metadata          — writable
/// 2. update_authority  — signer
pub struct UpdateMetadataAccountV2<'a> {
    pub metadata: &'a AccountView,
    pub update_authority: &'a AccountView,

    /// Replace `data` field. `None` keeps existing.
    pub new_data: Option<DataV2<'a>>,
    /// Rotate the update authority. `None` keeps existing.
    pub new_update_authority: Option<&'a Address>,
    /// Mark the primary sale as happened. `None` keeps existing
    /// (also: once `true`, can't go back to `false`).
    pub new_primary_sale_happened: Option<bool>,
    /// Toggle metadata mutability. `None` keeps existing.
    pub new_is_mutable: Option<bool>,
}

impl UpdateMetadataAccountV2<'_> {
    #[inline]
    pub fn invoke(&self) -> ProgramResult {
        self.invoke_signed(&[])
    }

    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult {
        if let Some(data) = self.new_data {
            data.validate_lengths()?;
        }

        let mut buf = [0u8; 384];
        let mut tape = BorshTape::new(&mut buf);
        tape.write_disc(DISC_UPDATE_METADATA_ACCOUNT_V2)?;

        // Option<DataV2> data
        match self.new_data {
            None => tape.write_option_none()?,
            Some(data) => {
                tape.write_option_some_tag()?;
                data.write_borsh_into(&mut tape)?;
            }
        }
        // Option<Pubkey> update_authority
        match self.new_update_authority {
            None => tape.write_option_none()?,
            Some(addr) => {
                tape.write_option_some_tag()?;
                tape.reserve_and_write_bytes(addr.as_array())?;
            }
        }
        // Option<bool> primary_sale_happened
        match self.new_primary_sale_happened {
            None => tape.write_option_none()?,
            Some(v) => {
                tape.write_option_some_tag()?;
                tape.write_bool(v)?;
            }
        }
        // Option<bool> is_mutable
        match self.new_is_mutable {
            None => tape.write_option_none()?,
            Some(v) => {
                tape.write_option_some_tag()?;
                tape.write_bool(v)?;
            }
        }

        let len = tape.len();
        let metas = [
            InstructionAccount::writable(self.metadata.address()),
            InstructionAccount::readonly_signer(self.update_authority.address()),
        ];
        let views = [self.metadata, self.update_authority];
        let instruction = InstructionView {
            program_id: &MPL_TOKEN_METADATA_PROGRAM_ID,
            data: &buf[..len],
            accounts: &metas,
        };
        hopper_runtime::cpi::invoke_signed(&instruction, &views, signers)
    }
}

// ── DataV2 / BorshTape glue ──────────────────────────────────────────
//
// DataV2 lives in this file (not encoding.rs) to keep encoding.rs
// purely about the Borsh tape mechanics. The `write_borsh_into`
// shim is named `write_borsh_into` (deliberately different from
// BorshTape::write_*) so it doesn't collide with any future BorshTape
// helper named `write_data_v2`.

impl<'a> DataV2<'a> {
    fn write_borsh_into(&self, tape: &mut BorshTape<'_>) -> ProgramResult {
        self.write_borsh(tape)
    }
}

// ── BorshTape extension for raw byte writes ──────────────────────────
//
// `UpdateMetadataAccountV2` writes a raw 32-byte pubkey for the
// optional new update authority; the encoding module's `write_str`
// would prefix it with a length tag (wrong format). We extend
// BorshTape inline here with a `reserve_and_write_bytes` helper used
// only by this file.

trait BorshTapeBytes {
    fn reserve_and_write_bytes(&mut self, bytes: &[u8]) -> ProgramResult;
}

impl<'a> BorshTapeBytes for BorshTape<'a> {
    fn reserve_and_write_bytes(&mut self, bytes: &[u8]) -> ProgramResult {
        // Re-use `write_str`-style logic without the length prefix.
        // The trait shim is the cleanest way to add this without
        // touching the encoding module's public API. If a second
        // callsite turns up the helper graduates into encoding.rs.
        if self.remaining() < bytes.len() {
            return Err(ProgramError::InvalidInstructionData);
        }
        for &b in bytes {
            self.write_u8(b)?;
        }
        Ok(())
    }
}
