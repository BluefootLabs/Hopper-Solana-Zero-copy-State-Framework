//! Framework-mode prelude for authored Hopper programs.
//!
//! This import is intentionally small: account wrappers, instruction context,
//! common wire types, result/error types, guard macros, CPI/token facade modules,
//! and the canonical proc macros. Protocol-grade state machinery lives behind
//! `hopper::systems::*` or explicit modules such as `hopper::segment`.

pub use crate::account::{
    Account, InitAccount, Program, ProgramId, Signer, System, SystemId, UncheckedAccount,
};
pub use crate::context::Context;
pub use crate::context::Context as Ctx;
pub use hopper_runtime::{AccountView, Address, ProgramError, ProgramResult};

/// Solana-familiar alias for Hopper's 32-byte address type.
pub type Pubkey = Address;

/// Handler result alias for examples that prefer `Result<()>` spelling.
pub type Result<T = (), E = ProgramError> = core::result::Result<T, E>;

pub use hopper_core::abi::{WireBool, WireU16, WireU32, WireU64};

pub use crate::{associated_token, cpi, events, memo, pda, system, token, token_2022};
pub use hopper_associated_token::ATA_PROGRAM_ID;
pub use hopper_system::SYSTEM_PROGRAM_ID;
pub use hopper_token::TOKEN_PROGRAM_ID;
pub use hopper_token_2022::TOKEN_2022_PROGRAM_ID;

#[cfg(feature = "metaplex")]
pub use hopper_metaplex;
#[cfg(feature = "metaplex")]
pub use hopper_metaplex::{
    master_edition_pda, master_edition_pda_with_bump, metadata_pda, metadata_pda_with_bump,
    CreateMasterEditionV3, CreateMetadataAccountV3, DataV2, UpdateMetadataAccountV2,
    MPL_TOKEN_METADATA_PROGRAM_ID,
};

pub use hopper_runtime::{
    address, err, error, hopper_emit_cpi, hopper_log, msg, require, require_eq, require_gt,
    require_gte, require_keys_eq, require_keys_neq, require_lt, require_lte, require_neq,
};

#[cfg(feature = "proc-macros")]
pub use crate::{account, accounts, args, error_code, event, program, Accounts, HopperInitSpace};
