//! Framework-mode prelude for authored Hopper programs.
//!
//! Keep this surface small and obvious: accounts, context, common wire types,
//! errors/results, guard macros, CPI/token modules, and the canonical proc
//! macros. Protocol-grade state machinery lives behind `hopper::systems::*` or
//! the explicit modules (`hopper::segment`, `hopper::receipt`, and friends).

// Account and context model.
pub use crate::account::{Account, InitAccount, Program, Signer, UncheckedAccount};
pub use crate::context::Context;
pub use hopper_runtime::{AccountView, Address, ProgramError, ProgramId, ProgramResult, SystemId};

/// Solana-familiar alias for Hopper's 32-byte address type.
pub type Pubkey = Address;

/// Handler result alias for examples that prefer `Result<()>` spelling.
pub type Result<T = (), E = ProgramError> = core::result::Result<T, E>;

// ABI wire types and common role markers.
pub use hopper_core::abi::{
    Authority, Mint, Program as ProgramRole, Token, TokenAccount, TypedAddress, UntypedAddress,
    WireBool, WireI128, WireI16, WireI32, WireI64, WireU128, WireU16, WireU32, WireU64,
};

// Account DSL and app-level validation traits.
pub use hopper_core::accounts::{
    hopper_entry, HopperAccount, HopperAccounts, HopperCtx, HopperIx, ProgramAccount, ProgramRef,
    SignerAccount, ValidateAccount,
};
pub use hopper_core::check::{find_and_verify_pda, rent_exempt_min};

// Runtime entrypoint and bootstrap helpers used by on-chain programs.
pub use hopper_runtime::{
    fast_entrypoint, hopper_entrypoint, hopper_fast_entrypoint, hopper_lazy_entrypoint,
    lazy_entrypoint, no_allocator, nostd_panic_handler, program_entrypoint,
};

// Instruction/CPI helpers that are common in framework-mode programs.
pub use hopper_runtime::{CpiAccount, InstructionAccount, InstructionView, Seed};

// Bounded dynamic payloads are common enough for app authors porting from
// Quasar-shaped `string<N>` / `vec<T, N>` fields.
pub use hopper_runtime::{BoundedString, BoundedVec, HopperString, HopperVec, TailCodec};

// Facade modules. Importing the module names keeps autocomplete compact while
// still making `token::transfer_checked(...)` and friends discoverable.
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

// Framework-mode macro_rules helpers.
pub use crate::{
    hopper_accounts, hopper_check, hopper_close, hopper_dynamic_fields, hopper_dynamic_tail,
    hopper_error, hopper_init, hopper_load, hopper_require, hopper_validate, hopper_verify_pda,
};

// Guard and log macros. These stay in the prelude because they are part of the
// daily handler-writing loop.
pub use hopper_runtime::{
    address, err, error, hopper_emit_cpi, hopper_log, msg, require, require_eq, require_gt,
    require_gte, require_keys_eq, require_keys_neq, require_lt, require_lte, require_neq,
};

#[cfg(target_os = "solana")]
pub use crate::pda::{
    create_program_address, find_program_address, verify_pda, verify_pda_with_bump,
};

// Macro hygiene re-exports. Hidden from docs, but kept at this stable path so
// generated code does not leak systems imports into user source.
#[cfg(feature = "receipt")]
#[doc(hidden)]
pub use crate::receipt::{emit_receipt, FailureStage, StateReceipt};
#[doc(hidden)]
pub use hopper_core::account::HEADER_LEN;

// Optional proc macro surface, enabled by `features = ["proc-macros"]`.
#[cfg(feature = "proc-macros")]
pub use crate::{
    account, accounts, args, context, dynamic, error_code, event, program, Accounts,
    HopperInitSpace,
};
