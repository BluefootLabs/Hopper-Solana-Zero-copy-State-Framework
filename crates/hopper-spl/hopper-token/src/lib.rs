//! Hopper-owned SPL Token builder surface.
//!
//! Thin first-class Hopper wrappers over the canonical runtime builders.
//! This crate gives Hopper a native token CPI surface instead of forcing
//! authored programs to depend on external helper crates.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod layout;

pub use hopper_runtime::token::{
    ApproveChecked, BurnChecked, CheckedMintDecimals, CheckedTokenAuthority, CheckedTokenMint,
    CloseAccount, FreezeAccount, InitializeAccount, InitializeAccount2, InitializeAccount3,
    MintToChecked, Revoke, SetAuthority, SplMint, SplMintView, SplTokenAccount,
    SplTokenAccountView, SyncNative, ThawAccount, TokenAmountSnapshot, TokenAuthorityType,
    TransferChecked, TOKEN_PROGRAM_ID,
};

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
        ApproveChecked, BurnChecked, CloseAccount, FreezeAccount, InitializeAccount,
        InitializeAccount2, InitializeAccount3, MintToChecked, Revoke, SetAuthority, SyncNative,
        ThawAccount, TokenAuthorityType, TransferChecked,
    };

    #[cfg(feature = "legacy-token-instructions")]
    #[allow(deprecated)]
    pub use hopper_runtime::token::{Approve, Burn, MintTo, Transfer};
}
