//! # Hopper NFT Mint
//!
//! Reference program that mints a Metaplex 1-of-1 NFT in three
//! instructions:
//!
//! | Disc | Name              | What it does |
//! |-----:|-------------------|--------------|
//! | `0`  | `init_mint`       | The caller has already created the SPL mint and minted exactly 1 token to itself; this instruction does nothing on chain except validate the structure. Kept as a placeholder so the dispatch table is complete. |
//! | `1`  | `create_metadata` | Invokes Metaplex's `CreateMetadataAccountV3` to attach name/symbol/uri/SFBP to the mint. |
//! | `2`  | `create_master_edition` | Invokes Metaplex's `CreateMasterEditionV3` with `max_supply = Some(0)` to lock the mint as a 1-of-1 NFT. |
//!
//! Closes the Quasar-parity Metaplex gap from
//! [`AUDIT.md`](../../AUDIT.md). Built specifically with the Boobies
//! NFT project (Galápagos blue-footed boobies, conservation donations
//! via [bluefoot.xyz](https://bluefoot.xyz)) in mind: a real-world
//! pattern Hopper users can copy-paste.
//!
//! ## What this example does NOT do
//!
//! - Does not create the SPL mint itself. `spl-token::InitializeMint`
//!   is a separate concern; use `hopper_token::instructions` for that
//!   path. The example assumes the caller has already done it.
//! - Does not handle collection NFTs or verified collection
//!   membership. Both are extensions on top of this base flow.
//! - Does not handle Bubblegum compressed NFTs or pNFT lifecycle.
//!   Those need additional Metaplex programs not in `hopper-metaplex`
//!   (yet).
//!
//! ## How to call it from a client
//!
//! ```ignore
//! // 1. Caller creates the SPL mint and mints 1 token to themselves
//! //    via the standard SPL Token Program.
//! //
//! // 2. Caller derives the metadata PDA:
//! //    let (metadata, _) = hopper_metaplex::metadata_pda(&mint_key);
//! //
//! // 3. Caller derives the master-edition PDA:
//! //    let (master_edition, _) = hopper_metaplex::master_edition_pda(&mint_key);
//! //
//! // 4. Caller invokes instruction 1 with name/symbol/uri/sfbp packed
//! //    into the instruction data (see `process_create_metadata`).
//! //
//! // 5. Caller invokes instruction 2 with `max_supply = 0` to lock
//! //    the mint as a 1-of-1.
//! ```

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
fast_entrypoint!(process_instruction, 10);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (disc, rest) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match *disc {
        0 => process_init_mint(program_id, accounts),
        1 => process_create_metadata(program_id, accounts, rest),
        2 => process_create_master_edition(program_id, accounts, rest),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

// ---------------------------------------------------------------------------
// 0. init_mint
// ---------------------------------------------------------------------------

/// Validation-only stub. Caller has already created the SPL mint.
/// We accept the instruction so the dispatch table has a slot for it,
/// and so future revisions of this example can grow the on-chain
/// init logic without bumping the discriminator.
fn process_init_mint(_program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    hopper_load!(accounts => [authority, _mint]);
    authority.require_signer()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 1. create_metadata
// ---------------------------------------------------------------------------

/// Instruction data layout for `process_create_metadata`:
///
/// ```text
///   [u8: name_len]   [name bytes]
///   [u8: symbol_len] [symbol bytes]
///   [u8: uri_len]    [uri bytes]
///   [u16 LE: seller_fee_basis_points]
///   [u8: is_mutable]
/// ```
///
/// We use single-byte length prefixes here because each Metaplex field
/// is capped at 200 bytes and a `u8` is enough. The on-chain instruction
/// builder (`CreateMetadataAccountV3`) Borsh-encodes the `u32` length
/// itself; this is just the wire format the client uses to talk to
/// *our* program.
///
/// Account ordering:
/// 1. authority (signer + writable, the payer / mint authority / update authority)
/// 2. mint (read-only, already initialised SPL mint)
/// 3. metadata (writable, the PDA we will create)
/// 4. system_program (read-only)
/// 5. mpl_token_metadata_program (read-only)
fn process_create_metadata(
    _program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    hopper_load!(accounts => [
        authority,
        mint,
        metadata,
        system_program,
        mpl_token_metadata_program,
    ]);

    authority.require_signer()?;
    metadata.require_writable()?;
    if mpl_token_metadata_program.address().as_array()
        != hopper_metaplex::MPL_TOKEN_METADATA_PROGRAM_ID.as_array()
    {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Decode our packed instruction layout.
    let (name, rest) = read_short_string(data)?;
    let (symbol, rest) = read_short_string(rest)?;
    let (uri, rest) = read_short_string(rest)?;
    if rest.len() < 3 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let sfbp = u16::from_le_bytes([rest[0], rest[1]]);
    let is_mutable = rest[2] != 0;

    // Build the Metaplex CPI. The authority is being used in three
    // roles (mint authority, payer, update authority) which is the
    // typical 1-of-1 NFT mint shape.
    CreateMetadataAccountV3 {
        metadata,
        mint,
        mint_authority: authority,
        payer: authority,
        update_authority: authority,
        system_program,
        rent: None,
        data: DataV2::simple(name, symbol, uri, sfbp),
        is_mutable,
    }
    .invoke()
}

// ---------------------------------------------------------------------------
// 2. create_master_edition
// ---------------------------------------------------------------------------

/// Lock the mint as a 1-of-1 NFT by setting `max_supply = Some(0)`.
/// Subsequent print attempts via `MintNewEditionFromMasterEditionViaToken`
/// will fail with `MaxEditionsMintedAlready`.
///
/// Instruction data layout: `[u64 LE: max_supply]`. Pass `0` for a
/// 1-of-1 (no prints), or a positive value to allow exactly that many
/// numbered print editions.
///
/// Account ordering:
/// 1. authority (signer + writable, payer / mint authority / update authority)
/// 2. mint (writable)
/// 3. metadata (read-only, the PDA created in instruction 1)
/// 4. master_edition (writable, the PDA we will create)
/// 5. spl_token_program (read-only)
/// 6. system_program (read-only)
/// 7. mpl_token_metadata_program (read-only)
fn process_create_master_edition(
    _program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    hopper_load!(accounts => [
        authority,
        mint,
        metadata,
        master_edition,
        spl_token_program,
        system_program,
        mpl_token_metadata_program,
    ]);

    authority.require_signer()?;
    mint.require_writable()?;
    master_edition.require_writable()?;
    if mpl_token_metadata_program.address().as_array()
        != hopper_metaplex::MPL_TOKEN_METADATA_PROGRAM_ID.as_array()
    {
        return Err(ProgramError::IncorrectProgramId);
    }

    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let max_supply = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);

    CreateMasterEditionV3 {
        edition: master_edition,
        mint,
        update_authority: authority,
        mint_authority: authority,
        payer: authority,
        metadata,
        token_program: spl_token_program,
        system_program,
        rent: None,
        max_supply: Some(max_supply),
    }
    .invoke()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decode a length-prefixed UTF-8 string with a single-byte length.
/// Returns the string slice and the remaining bytes.
fn read_short_string(data: &[u8]) -> Result<(&str, &[u8]), ProgramError> {
    let (&len_byte, rest) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    let len = len_byte as usize;
    if rest.len() < len {
        return Err(ProgramError::InvalidInstructionData);
    }
    let (str_bytes, tail) = rest.split_at(len);
    let s = core::str::from_utf8(str_bytes).map_err(|_| ProgramError::InvalidInstructionData)?;
    Ok((s, tail))
}
