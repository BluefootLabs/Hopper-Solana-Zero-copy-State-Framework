//! Typed instruction context and account struct trait.

use hopper_runtime::{AccountView, Address};
use hopper_runtime::error::ProgramError;

use super::explain::ContextExplain;

/// Trait implemented by account structs (manually or via derive).
///
/// Provides typed construction from raw accounts and optional schema metadata.
pub trait HopperAccounts<'a>: Sized {
    /// PDA bump storage type for this context.
    type Bumps: Default;

    /// Number of accounts consumed by this context struct.
    ///
    /// Used by `hopper_entry()` to split the accounts slice into consumed
    /// accounts and remaining accounts for CPI forwarding.
    const ACCOUNT_COUNT: usize;

    /// Construct the account struct from raw instruction inputs.
    ///
    /// Performs all validation: signer checks, writable checks, owner checks,
    /// PDA verification, layout validation.
    fn try_from_accounts(
        program_id: &'a Address,
        accounts: &'a [AccountView],
        instruction_data: &'a [u8],
    ) -> Result<(Self, Self::Bumps), ProgramError>;

    /// Optional static schema for introspection and explain.
    fn context_schema() -> Option<&'static crate::accounts::explain::ContextSchema> {
        None
    }
}

/// Typed instruction context carrying validated accounts, bumps, and metadata.
///
/// Replaces Anchor's `Context<T>` with Hopper-native semantics: receipts,
/// explain, schema access, and remaining accounts.
pub struct HopperCtx<'a, T>
where
    T: HopperAccounts<'a>,
{
    /// Validated accounts struct.
    pub accounts: T,
    /// Resolved PDA bumps.
    pub bumps: T::Bumps,
    /// The executing program's address.
    pub program_id: &'a Address,
    /// Remaining unparsed instruction data (after dispatch tag).
    pub instruction_data: &'a [u8],
    /// Accounts not consumed by the struct (for CPI or dynamic use).
    pub remaining_accounts: &'a [AccountView],
}

impl<'a, T> HopperCtx<'a, T>
where
    T: HopperAccounts<'a>,
{
    /// Emit a default receipt for the current mutation.
    ///
    /// Routes to the existing Hopper receipt infrastructure when a receipt
    /// profile is bound to this context.
    #[inline]
    pub fn emit_receipt(&self) -> Result<(), ProgramError> {
        // v1: receipt emission requires an active StateReceipt.
        // This is a convenience hook; callers should use StateReceipt::begin()
        // and commit() for full receipt control.
        Ok(())
    }

    /// Generate a human-readable explanation of this context and its accounts.
    #[inline]
    pub fn explain(&self) -> ContextExplain {
        ContextExplain::from_schema(T::context_schema())
    }

    /// Access the static context schema, if available.
    #[inline]
    pub fn schema(&self) -> Option<&'static crate::accounts::explain::ContextSchema> {
        T::context_schema()
    }

    /// Construct a context from pre-validated parts.
    ///
    /// Callers must ensure all accounts have already been validated.
    /// Typically used by derive-generated `try_from_accounts` or `entry()`.
    #[inline]
    pub fn new(
        accounts: T,
        bumps: T::Bumps,
        program_id: &'a Address,
        instruction_data: &'a [u8],
        remaining_accounts: &'a [AccountView],
    ) -> Self {
        Self {
            accounts,
            bumps,
            program_id,
            instruction_data,
            remaining_accounts,
        }
    }
}
