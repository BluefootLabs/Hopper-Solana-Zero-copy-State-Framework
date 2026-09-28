//! Hopper-owned SPL Token builder surface.
//!
//! Thin first-class Hopper wrappers over the canonical runtime builders.
//! This crate gives Hopper a native token CPI surface instead of forcing
//! authored programs to depend on external helper crates.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod layout;

pub use hopper_runtime::token_mint::{InitializeMint2, MintConfig, MintPlan, MintProgram};

pub use hopper_runtime::token::{
    return_data_string, return_data_u64, AmountToUiAmount, ApproveChecked, BurnChecked,
    CheckedMintDecimals, CheckedTokenAuthority, CheckedTokenMint, CloseAccount, FreezeAccount,
    GetAccountDataSize, InitializeAccount, InitializeAccount2, InitializeAccount3,
    InitializeImmutableOwner, InitializeMint, InitializeMultisig, InitializeMultisig2,
    MintToChecked, Revoke, SetAuthority, SplMint, SplMintView, SplTokenAccount,
    SplTokenAccountView, SyncNative, ThawAccount, TokenAmountSnapshot, TokenAuthorityType,
    TokenBatch, TokenInstruction, TokenProgram, TokenSink, Trailing, TransferChecked,
    UiAmountToAmount, UnwrapLamports, WithdrawExcessLamports, MAX_TOKEN_MULTISIG_SIGNERS,
    MAX_UI_AMOUNT_LEN, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
};
pub use hopper_runtime::token_batch::{BATCH_DISCRIMINATOR, BATCH_INSTRUCTION_HEADER_LEN};

#[cfg(feature = "legacy-token-instructions")]
#[allow(deprecated)]
pub use hopper_runtime::token::{Approve, Burn, MintTo, Transfer};

/// SPL Token instruction builders exported by Hopper.
///
/// Safety-by-default exports include checked variants plus operations whose
/// SPL semantics do not need a mint-decimals guard. Enable the explicit
/// `legacy-token-instructions` feature to expose the deprecated plain
/// `Transfer`, `MintTo`, `Burn`, and `Approve` builders for migration tests.
pub mod instructions {
    pub use hopper_runtime::token::{
        AmountToUiAmount, ApproveChecked, BurnChecked, CloseAccount, FreezeAccount,
        GetAccountDataSize, InitializeAccount, InitializeAccount2, InitializeAccount3,
        InitializeImmutableOwner, InitializeMint, InitializeMultisig, InitializeMultisig2,
        MintToChecked, Revoke, SetAuthority, SyncNative, ThawAccount, TokenAuthorityType,
        TokenBatch, TokenInstruction, TokenProgram, TransferChecked, UiAmountToAmount,
        UnwrapLamports, WithdrawExcessLamports,
    };
    pub use hopper_runtime::token_mint::InitializeMint2;

    #[cfg(feature = "legacy-token-instructions")]
    #[allow(deprecated)]
    pub use hopper_runtime::token::{Approve, Burn, MintTo, Transfer};
}
