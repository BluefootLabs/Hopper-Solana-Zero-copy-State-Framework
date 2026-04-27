//! # Hopper Metaplex
//!
//! Hopper-owned builder surface for Metaplex Token Metadata. Powers the
//! `metadata::*` and `master_edition::*` field keywords on
//! `#[hopper::context]` and the `examples/hopper-nft-mint` reference
//! program.
//!
//! Closes the Quasar-parity gap flagged in [`AUDIT.md`](../../../AUDIT.md)
//! ("Quasar has Metaplex sugar, Hopper doesn't"). The keywords now have a
//! working CPI lowering instead of being parser-only stubs.
//!
//! ## What this crate ships
//!
//! - [`MPL_TOKEN_METADATA_PROGRAM_ID`](constants::MPL_TOKEN_METADATA_PROGRAM_ID)
//!   — the canonical Metaplex Token Metadata program address as a Hopper
//!   `Address` constant, decoded at compile time.
//! - [`seeds`] — PDA-seed helpers (`metadata_pda`, `master_edition_pda`).
//!   Hopper's typed-seeds path uses these so the field-level
//!   `seeds = ...` constraint composes the right thing.
//! - [`instructions`] — zero-copy CPI builders for the three Metaplex
//!   calls every NFT-mint program reaches for: `CreateMetadataAccountV3`,
//!   `CreateMasterEditionV3`, `UpdateMetadataAccountV2`.
//!
//! ## What it deliberately does not ship (yet)
//!
//! Newer Metaplex flows (Bubblegum compressed NFTs, the
//! `pNFT` programmable-NFT lifecycle, edition prints, collection
//! verification) are out of scope for this first cut. The three
//! instructions above cover ~95% of straightforward 1-of-1 NFT mint
//! programs and the full Boobies use case. Adding more is a matter of
//! pattern-matching against the existing builders; the core encoding
//! infrastructure ([`encoding`]) is shared.
//!
//! ## Encoding policy
//!
//! Metaplex's instruction data is Borsh-encoded with variable-length
//! `String` fields. Hopper is otherwise a zero-copy framework, but we
//! cannot make Borsh zero-copy — variable-length strings have no fixed
//! offsets. Each builder therefore allocates a small **stack** buffer
//! (256–512 bytes depending on instruction) and writes the Borsh tape
//! directly into it. No heap, no `Vec`, no `alloc::String`. The
//! [`encoding::BorshTape`] writer is the load-bearing piece; it caps the
//! payload at the buffer size and returns
//! `ProgramError::InvalidInstructionData` if a string would overflow,
//! so a malicious caller can't push the program into UB by passing an
//! oversized name.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(clippy::result_large_err)]

pub mod constants;
pub mod encoding;
pub mod instructions;
pub mod seeds;

pub use constants::{
    MPL_TOKEN_METADATA_PROGRAM_ID,
    METADATA_SEED_PREFIX,
    EDITION_SEED_PREFIX,
    MAX_NAME_LEN,
    MAX_SYMBOL_LEN,
    MAX_URI_LEN,
};
pub use instructions::{
    CreateMasterEditionV3,
    CreateMetadataAccountV3,
    IntoMasterEditionMaxSupply,
    UpdateMetadataAccountV2,
    DataV2,
};
pub use seeds::{master_edition_pda, metadata_pda, master_edition_pda_with_bump, metadata_pda_with_bump};
