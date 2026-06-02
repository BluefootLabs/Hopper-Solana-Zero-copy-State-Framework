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
pub mod entry;
#[cfg(feature = "explain")]
pub mod explain;
pub mod hopper_account;
pub mod meta;
#[cfg(feature = "migrate")]
pub mod migrating;
pub mod program;
pub mod program_account;
pub mod segmented;
pub mod signer;
pub mod traits;
pub mod unchecked;
pub mod validate;

pub use context::{HopperAccounts, HopperCtx};
pub use entry::{hopper_entry, HopperIx};
#[cfg(feature = "explain")]
pub use explain::{AccountClass, AccountExplain, ContextExplain};
pub use hopper_account::HopperAccount;
pub use meta::AccountMetaProvider;
#[cfg(feature = "migrate")]
pub use migrating::MigratingAccount;
pub use program::ProgramRef;
pub use program_account::ProgramAccount;
pub use segmented::SegmentedAccount;
pub use signer::SignerAccount;
#[cfg(feature = "explain")]
pub use traits::ExplainAccount;
pub use traits::ValidateAccount;
pub use unchecked::UncheckedAccount;
pub use validate::{require_executable, require_owner, require_signer, require_writable};
