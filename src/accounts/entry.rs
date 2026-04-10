//! Typed instruction entry model.
//!
//! Provides `HopperIx` trait for instruction definition and `hopper_entry()`
//! for clean instruction dispatch. Bridges the Account DSL to Hopper's
//! existing dispatch system.

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use super::context::{HopperCtx, HopperAccounts};

/// Trait defining a Hopper instruction.
///
/// Combines argument parsing with account construction into a single
/// instruction definition. Used with `hopper_entry()` for typed dispatch.
pub trait HopperIx<'a>: Sized {
    /// The account struct for this instruction.
    type Accounts: HopperAccounts<'a>;
    /// The parsed argument type.
    type Args;

    /// Parse instruction arguments from raw data (after dispatch tag).
    fn parse_args(data: &'a [u8]) -> Result<Self::Args, ProgramError>;
}

/// Typed instruction entry point.
///
/// Parses arguments, constructs the validated context, and invokes the handler.
/// One-line replacement for manual dispatch + parse + validate + execute.
///
/// ```ignore
/// hopper_entry::<DepositIx, _>(program_id, accounts, data, |ctx, args| {
///     deposit(ctx, args.amount)
/// })
/// ```
#[inline]
pub fn hopper_entry<'a, I, F>(
    program_id: &'a Address,
    accounts: &'a [AccountView],
    instruction_data: &'a [u8],
    handler: F,
) -> Result<(), ProgramError>
where
    I: HopperIx<'a>,
    F: FnOnce(HopperCtx<'a, I::Accounts>, I::Args) -> Result<(), ProgramError>,
{
    let args = I::parse_args(instruction_data)?;
    let (accts, bumps) = I::Accounts::try_from_accounts(
        program_id,
        accounts,
        instruction_data,
    )?;
    let consumed = I::Accounts::ACCOUNT_COUNT;
    let remaining = if consumed < accounts.len() {
        &accounts[consumed..]
    } else {
        &[]
    };
    let ctx = HopperCtx::new(
        accts,
        bumps,
        program_id,
        instruction_data,
        remaining,
    );
    handler(ctx, args)
}
