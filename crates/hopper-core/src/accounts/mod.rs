//! Hopper Account DSL -- typed account ergonomics for zero-copy programs.
//!
//! Provides composable account wrappers, a typed context, and an instruction
//! entry model. Built on top of Hopper's existing modifier + validation infra.
//!
//! ## Core types
//!
//! - [`HopperCtx`] -- typed instruction context with accounts, bumps, receipts
//! - [`HopperAccount`] -- layout-bound typed account with read/write/init
//! - [`ProgramAccount`] -- generic SPL-program-owned account
//! - [`SignerAccount`] -- verified signer account
//! - [`UncheckedAccount`] -- raw account with no validation
//! - [`MigratingAccount`] -- dual-layout migration wrapper
//! - [`SegmentedAccount`] -- multi-segment typed account
//! - [`ProgramRef`] -- verified executable program reference
//!
//! ## Instruction model
//!
//! - [`HopperAccounts`] -- trait for account struct construction + schema
//! - [`HopperIx`] -- instruction definition trait (args + accounts)
//! - [`entry()`] -- typed instruction entry point

pub mod context;
pub mod hopper_account;
pub mod program_account;
pub mod signer;
pub mod unchecked;
#[cfg(feature = "migrate")]
pub mod migrating;
pub mod segmented;
pub mod program;
pub mod traits;
pub mod validate;
#[cfg(feature = "explain")]
pub mod explain;
pub mod meta;
pub mod entry;

pub use context::{HopperCtx, HopperAccounts};
pub use hopper_account::HopperAccount;
pub use program_account::ProgramAccount;
pub use signer::SignerAccount;
pub use unchecked::UncheckedAccount;
#[cfg(feature = "migrate")]
pub use migrating::MigratingAccount;
pub use segmented::SegmentedAccount;
pub use program::ProgramRef;
pub use traits::ValidateAccount;
#[cfg(feature = "explain")]
pub use traits::ExplainAccount;
pub use validate::{require_signer, require_writable, require_owner, require_executable};
#[cfg(feature = "explain")]
pub use explain::{ContextExplain, AccountExplain};
pub use meta::AccountMetaProvider;
pub use entry::{HopperIx, hopper_entry};
