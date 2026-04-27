//! Safe default path for Hopper Native.
//!
//! Re-exports the checked, validated APIs that most programs should use.
//! This is the recommended entry point for standard Hopper development.

pub use crate::account_view::AccountView;
pub use crate::pda::{verify_pda, verify_pda_strict, verify_pda_with_bump};

#[cfg(feature = "cpi")]
pub use crate::cpi::{invoke, invoke_signed, invoke_signed_with_bounds, invoke_with_bounds};
