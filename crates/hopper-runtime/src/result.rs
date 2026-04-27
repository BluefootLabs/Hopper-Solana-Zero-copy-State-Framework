//! Canonical result type for Hopper programs.

use crate::ProgramError;

/// Result type returned by all Hopper instruction handlers.
pub type ProgramResult = Result<(), ProgramError>;
