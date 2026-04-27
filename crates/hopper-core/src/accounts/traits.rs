//! Account validation and explain traits.

use hopper_runtime::error::ProgramError;

/// Trait for accounts that can self-validate.
///
/// Automatically satisfied by construction for modifier-wrapped types.
/// Useful for custom account implementations that need a post-construction
/// validation pass.
pub trait ValidateAccount {
    /// Run all validation checks. Returns `Ok(())` if valid.
    fn validate(&self) -> Result<(), ProgramError>;
}

/// Trait for accounts that can produce a structured explanation.
#[cfg(feature = "explain")]
pub trait ExplainAccount {
    /// Generate a human-readable explanation of this account.
    fn explain(&self) -> super::explain::AccountExplain;
}
