//! Compile-time Metaplex constants.

use hopper_runtime::address::Address;

/// Canonical Metaplex Token Metadata program ID:
/// `metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s`.
///
/// Decoded at compile time via `five8_const::decode_32_const`. The
/// constant is unchanged since the program's deployment in 2021 and is
/// the on-chain entry point for every Metaplex Token Metadata
/// instruction this crate builds.
pub const MPL_TOKEN_METADATA_PROGRAM_ID: Address = Address::new_from_array(
    five8_const::decode_32_const("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s"),
);

/// Seed prefix for the metadata PDA: `b"metadata"`.
///
/// The full metadata PDA is derived as
/// `find_program_address(&[b"metadata", program_id, mint], program_id)`
/// where `program_id` is the Metaplex Token Metadata program.
pub const METADATA_SEED_PREFIX: &[u8] = b"metadata";

/// Seed prefix for the master-edition PDA: `b"edition"`.
///
/// The full master-edition PDA is derived as
/// `find_program_address(&[b"metadata", program_id, mint, b"edition"], program_id)`.
/// Note that `b"metadata"` appears in both the metadata and the master-edition
/// seed lists; the difference is the trailing `b"edition"`.
pub const EDITION_SEED_PREFIX: &[u8] = b"edition";

/// Maximum on-chain length of an NFT's `name` field, in bytes
/// (matches the Metaplex `MAX_NAME_LENGTH` constant). A name longer
/// than this will be rejected by the on-chain program; the builder
/// rejects it earlier so the failure surfaces as
/// `ProgramError::InvalidInstructionData` from the calling program
/// instead of a less-clear Metaplex-side error.
pub const MAX_NAME_LEN: usize = 32;

/// Maximum on-chain length of an NFT's `symbol` field, in bytes
/// (matches the Metaplex `MAX_SYMBOL_LENGTH` constant).
pub const MAX_SYMBOL_LEN: usize = 10;

/// Maximum on-chain length of an NFT's `uri` field, in bytes (matches
/// the Metaplex `MAX_URI_LENGTH` constant).
pub const MAX_URI_LEN: usize = 200;
