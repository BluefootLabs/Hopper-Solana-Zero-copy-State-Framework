//! Phase-specific instruction argument decomposition.
//!
//! Allows instruction data to be split into typed phases that align with
//! the `PhasedFrame` execution model: resolve, validate, execute.
//!
//! Each phase can access only the subset of instruction args it needs,
//! enforcing separation of concerns at the type level.
//!
//! ## Usage
//!
//! ```ignore
//! struct WithdrawArgs<'a> {
//!     amount: u64,
//!     memo: &'a [u8],
//! }
//!
//! impl<'a> InstructionArgs<'a> for WithdrawArgs<'a> {
//!     fn parse(data: &'a [u8]) -> Result<Self, ProgramError> {
//!         if data.len() < 8 {
//!             return Err(ProgramError::InvalidInstructionData);
//!         }
//!         Ok(Self {
//!             amount: u64::from_le_bytes(data[..8].try_into().unwrap()),
//!             memo: &data[8..],
//!         })
//!     }
//! }
//!
//! // In phased execution:
//! let frame = PhasedFrame::new(program_id, accounts, ix_data)?;
//! let args = WithdrawArgs::parse(ix_data)?;
//!
//! frame
//!     .resolve(2, |accounts, _pid| Ok(MyAccounts { ... }))?
//!     .validate_with_args(&args, |ctx, _pid, args| {
//!         hopper_require!(args.amount > 0, ZeroAmount);
//!         Ok(())
//!     })?
//!     .execute_with_args(&args, |ctx, args| {
//!         // use args.amount for mutation
//!         Ok(())
//!     })?;
//! ```

use hopper_runtime::error::ProgramError;

/// Trait for parsing instruction data into a typed args struct.
///
/// Implement this for each instruction's argument format.
/// The lifetime `'a` allows zero-copy references into the instruction data.
pub trait InstructionArgs<'a>: Sized {
    /// Parse instruction data into typed args.
    fn parse(data: &'a [u8]) -> Result<Self, ProgramError>;
}

/// Trait for args that can be validated independently of accounts.
///
/// Implement this to perform pure argument validation (range checks,
/// non-zero assertions, format validation) before touching any account state.
pub trait ValidateArgs {
    /// Validate the arguments in isolation. Called before account validation.
    fn validate(&self) -> Result<(), ProgramError>;
}

// -- Extensions to ResolvedFrame for arg-aware validation/execution --

use super::phase::{ResolvedFrame, ValidatedFrame, ExecutionContext};
use hopper_runtime::{Address, ProgramResult};

impl<'a, T> ResolvedFrame<'a, T> {
    /// Validate with access to both resolved accounts AND typed instruction args.
    ///
    /// This is the arg-aware counterpart of `validate()`.
    /// The closure receives (accounts, program_id, args).
    #[inline]
    pub fn validate_with_args<A, F>(
        self,
        args: &A,
        f: F,
    ) -> Result<ValidatedFrame<'a, T>, ProgramError>
    where
        F: FnOnce(&T, &Address, &A) -> ProgramResult,
    {
        f(&self.resolved, self.program_id, args)?;
        Ok(ValidatedFrame {
            program_id: self.program_id,
            accounts: self.accounts,
            ix_data: self.ix_data,
            mutable_borrows: self.mutable_borrows,
            resolved: self.resolved,
        })
    }
}

impl<'a, T> ValidatedFrame<'a, T> {
    /// Execute with access to typed instruction args.
    ///
    /// This is the arg-aware counterpart of `execute()`.
    #[inline]
    pub fn execute_with_args<A, R, F>(
        mut self,
        args: &A,
        f: F,
    ) -> Result<R, ProgramError>
    where
        F: FnOnce(&mut ExecutionContext<'a, '_, T>, &A) -> Result<R, ProgramError>,
    {
        let mut ctx = ExecutionContext {
            program_id: self.program_id,
            accounts: self.accounts,
            ix_data: self.ix_data,
            mutable_borrows: &mut self.mutable_borrows,
            resolved: &self.resolved,
        };
        f(&mut ctx, args)
    }
}
